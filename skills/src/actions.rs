//! Bilimbi Skills.Actions: proposals, reach, transitions and presentation.
use crate::{
    access,
    action_rows::{self, Event},
    assessments, catalog, policy, Action, ActionGroup, ActionInput, ActionPlan, ActionReassessment,
    ActionSource, ActionType, ActionView, Assessment, NewActionType, Result, Skills, SkillsError,
    Write, ACTIONS_MANAGE, ACTIONS_UPDATE,
};
use bilibi_database::{Error, Result as DbResult};
use bilibi_people_workforce::authorization;
use bilibi_tenancy::Scope;
use jiff::civil::{Date, DateTime};
use tokio_postgres::{GenericClient, Row};

const OPEN: &[&str] = &["not_started", "scheduled", "in_progress", "on_hold"];
const TYPE_COLUMNS: &str = "id,code,name,requires_provider,active";

pub(crate) fn now() -> DateTime {
    jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .datetime()
}

pub(crate) fn text(value: &str, max: usize, required: bool) -> Result<Option<String>> {
    let value = value.trim();
    if (required && value.is_empty()) || value.chars().count() > max {
        return Err(SkillsError::InvalidAction);
    }
    Ok((!value.is_empty()).then(|| value.to_owned()))
}

pub(crate) fn required(value: &str, error: SkillsError) -> Result<String> {
    text(value, 2000, true)
        .map_err(|_| error)
        .map(Option::unwrap_or_default)
}

pub(crate) fn normalize(input: &ActionPlan) -> Result<ActionPlan> {
    let start = input.start_on.unwrap_or_else(assessments::today);
    if input.action_type_id <= 0
        || input.owner_employee_id <= 0
        || input.coordinator_employee_id <= 0
        || input.provider_employee_id.is_some_and(|id| id <= 0)
        || input.due_on.is_none_or(|d| d < start)
    {
        return Err(SkillsError::InvalidAction);
    }
    Ok(ActionPlan {
        start_on: Some(start),
        objective: text(&input.objective, 2000, true)?.unwrap_or_default(),
        intervention: text(&input.intervention, 2000, true)?.unwrap_or_default(),
        expected_evidence: text(&input.expected_evidence, 2000, true)?.unwrap_or_default(),
        next_steps: text(input.next_steps.as_deref().unwrap_or_default(), 2000, false)?,
        provider_name: text(
            input.provider_name.as_deref().unwrap_or_default(),
            160,
            false,
        )?,
        ..input.clone()
    })
}

fn type_row(r: &Row) -> ActionType {
    ActionType {
        id: r.get("id"),
        code: r.get("code"),
        name: r.get("name"),
        requires_provider: r.get("requires_provider"),
        active: r.get("active"),
    }
}
pub(crate) async fn types(
    client: &impl GenericClient,
    tenant: i64,
    company: i64,
) -> Result<Vec<ActionType>> {
    Ok(client.query(&format!("SELECT {TYPE_COLUMNS} FROM people_skill_action_types WHERE tenant_id=$1 AND company_id=$2 ORDER BY active DESC,name,id"), &[&tenant,&company]).await.map_err(Error::from)?.iter().map(type_row).collect())
}
pub(crate) async fn get_type(
    client: &impl GenericClient,
    tenant: i64,
    company: i64,
    id: i64,
) -> Result<ActionType> {
    client.query_opt(&format!("SELECT {TYPE_COLUMNS} FROM people_skill_action_types WHERE tenant_id=$1 AND company_id=$2 AND id=$3"), &[&tenant,&company,&id]).await.map_err(Error::from)?.as_ref().map(type_row).ok_or(SkillsError::NotFound)
}
pub(crate) async fn create_type(w: &mut Write<'_>, input: &NewActionType) -> Result<ActionType> {
    let valid = crate::validate::new_scale(&crate::NewScale {
        code: input.code.clone(),
        name: input.name.clone(),
    })?;
    if w.sql().query_opt("SELECT 1 FROM people_skill_action_types WHERE tenant_id=$1 AND company_id=$2 AND code=$3", &[&w.tenant,&w.company_id,&valid.code]).await.map_err(Error::from)?.is_some() {
        return Err(SkillsError::Invalid(vec![crate::FieldError::new("code","has already been taken")]));
    }
    let (tenant, company, provider) = (w.tenant, w.company_id, input.requires_provider);
    Ok(w.tx.mutate(move |sql|Box::pin(async move {
        let r=sql.query_one(&format!("INSERT INTO people_skill_action_types (tenant_id,company_id,code,name,requires_provider,active) VALUES ($1,$2,$3,$4,$5,true) RETURNING {TYPE_COLUMNS}"), &[&tenant,&company,&valid.code,&valid.name,&provider]).await?;
        let row=type_row(&r);
        Ok((row.clone(),vec![crate::rows::created("people_skill_action_type",row.id,&row)]))
    })).await?)
}
pub(crate) async fn set_type_active(
    w: &mut Write<'_>,
    old: ActionType,
    active: bool,
) -> Result<ActionType> {
    let (tenant, company) = (w.tenant, w.company_id);
    Ok(w.tx.mutate(move |sql|Box::pin(async move {
        let r=sql.query_one(&format!("UPDATE people_skill_action_types SET active=$4,updated_at=(now() AT TIME ZONE 'UTC') WHERE tenant_id=$1 AND company_id=$2 AND id=$3 RETURNING {TYPE_COLUMNS}"), &[&tenant,&company,&old.id,&active]).await?;
        let row=type_row(&r);
        Ok((row.clone(),vec![crate::rows::changed("people_skill_action_type",row.id,&old,&row)]))
    })).await?)
}

pub(crate) async fn validate_plan(
    skills: &Skills,
    w: &Write<'_>,
    scope: &Scope,
    input: &ActionPlan,
) -> Result<()> {
    let kind = get_type(w.sql(), w.tenant, w.company_id, input.action_type_id).await;
    let kind = match kind {
        Ok(t) if t.active => t,
        Err(SkillsError::Failed(e)) => return Err(e.into()),
        _ => return Err(SkillsError::TypeUnavailable),
    };
    if kind.requires_provider
        && input.provider_employee_id.is_none()
        && input.provider_name.is_none()
    {
        return Err(SkillsError::ProviderRequired);
    }
    let mut ids = vec![input.owner_employee_id, input.coordinator_employee_id];
    ids.extend(input.provider_employee_id);
    ids.sort_unstable();
    ids.dedup();
    for id in &ids {
        match skills
            .workforce
            .lock_working_employee(&w.tx, scope, w.company_id, *id)
            .await
        {
            Ok(()) => {}
            Err(bilibi_people_workforce::WorkforceError::NotFound) => {
                return Err(SkillsError::EmployeeUnavailable)
            }
            Err(e) => return Err(e.into()),
        }
    }
    let people = skills
        .workforce
        .employees_by_ids(scope, w.company_id, &ids)
        .await?
        .require_current()?;
    if people.len() != ids.len() {
        return Err(SkillsError::EmployeeUnavailable);
    }
    Ok(())
}

fn apply_plan(row: &mut Action, p: ActionPlan) {
    row.action_type_id = p.action_type_id;
    row.owner_employee_id = p.owner_employee_id;
    row.coordinator_employee_id = p.coordinator_employee_id;
    row.provider_employee_id = p.provider_employee_id;
    row.provider_name = p.provider_name;
    row.objective = p.objective;
    row.intervention = p.intervention;
    row.expected_evidence = p.expected_evidence;
    row.next_steps = p.next_steps;
    row.start_on = p.start_on;
    row.due_on = p.due_on;
}

pub(crate) async fn propose(
    skills: &Skills,
    w: &mut Write<'_>,
    scope: &Scope,
    input: &ActionInput,
) -> Result<(Action, bool)> {
    let key = text(&input.request_key, 80, true)?.unwrap_or_default();
    let plan = normalize(&input.plan)?;
    if let Some(r)=w.sql().query_opt("SELECT id,objective,action_type_id FROM people_skill_actions WHERE tenant_id=$1 AND company_id=$2 AND request_key=$3", &[&w.tenant,&w.company_id,&key]).await.map_err(Error::from)? {
        // Bilimbi intentionally compares these two facts, even after a revision.
        if r.get::<_,String>("objective")!=plan.objective || r.get::<_,i64>("action_type_id")!=plan.action_type_id {return Err(SkillsError::KeyConflict);}
        return Ok((action_rows::get(w.sql(),w.tenant,w.company_id,r.get("id")).await?,true));
    }
    let (employee_id, skill_id, assessment_id, starting, target, criticality, mandatory, reason) =
        match &input.source {
            ActionSource::Assessment(id) => {
                let a = assessments::get(w.sql(), w.tenant, w.company_id, *id).await?;
                if a.status != "finalized" {
                    return Err(SkillsError::NotActionable);
                }
                let current=w.sql().query_opt("SELECT assessment_id FROM people_skill_scores WHERE tenant_id=$1 AND company_id=$2 AND employee_id=$3 AND skill_id=$4", &[&w.tenant,&w.company_id,&a.employee_id,&a.skill_id]).await.map_err(Error::from)?;
                if current.is_none_or(|r| r.get::<_, i64>(0) != a.id) {
                    return Err(SkillsError::AssessmentNotCurrent);
                }
                if a.gap == 0
                    && !(a.criticality == "critical"
                        && a.valid_until.is_some_and(|d| d < assessments::today()))
                {
                    return Err(SkillsError::NotActionable);
                }
                if w.sql().query_opt("SELECT 1 FROM people_skill_actions WHERE tenant_id=$1 AND company_id=$2 AND source_assessment_id=$3", &[&w.tenant,&w.company_id,&a.id]).await.map_err(Error::from)?.is_some(){return Err(SkillsError::AlreadyProposed);}
                (
                    a.employee_id,
                    a.skill_id,
                    Some(a.id),
                    a.assessed_level,
                    a.required_level,
                    a.criticality,
                    a.mandatory,
                    None,
                )
            }
            ActionSource::Manual {
                employee_id,
                skill_id,
                starting_level,
                target_level,
                criticality,
                mandatory,
                reason,
            } => {
                if reason.trim().is_empty() {
                    return Err(SkillsError::ReasonRequired);
                }
                let reason = text(reason, 1000, true)?.unwrap_or_default();
                if *employee_id <= 0
                    || !(0..=crate::MAX_LEVEL).contains(starting_level)
                    || !(0..=crate::MAX_LEVEL).contains(target_level)
                    || !crate::CRITICALITIES.contains(&criticality.as_str())
                    || !catalog::get_skill(w, *skill_id)
                        .await?
                        .is_some_and(|s| s.active)
                {
                    return Err(SkillsError::InvalidAction);
                }
                (
                    *employee_id,
                    *skill_id,
                    None,
                    *starting_level,
                    *target_level,
                    criticality.clone(),
                    *mandatory,
                    Some(reason),
                )
            }
        };
    skills
        .workforce
        .lock_working_employee(&w.tx, scope, w.company_id, employee_id)
        .await?;
    let employee = skills
        .workforce
        .employee(scope, w.company_id, employee_id)
        .await?
        .require_current()?;
    validate_plan(skills, w, scope, &plan).await?;
    let policy = policy::get(&skills.settings, w.sql(), scope, w.company_id).await?;
    let gap = (target - starting).max(0);
    let multiplier = policy.multiplier(&criticality) as i32;
    let gate = if mandatory {
        "Mandatory requirement: yes; escalated ahead of the others. "
    } else {
        "Mandatory requirement: no. "
    };
    let row = Action {
        id: 0,
        employee_id,
        employee_name: employee.display_name,
        skill_id,
        source_assessment_id: assessment_id,
        action_type_id: plan.action_type_id,
        starting_level: starting,
        target_level: target,
        gap_at_start: gap,
        criticality: criticality.clone(),
        mandatory,
        priority_multiplier: multiplier,
        priority_score: gap * multiplier,
        priority_explanation: format!(
            "{gate}Score {} = gap {gap} x {criticality} multiplier {multiplier}.",
            gap * multiplier
        ),
        manual_reason: reason,
        objective: plan.objective,
        intervention: plan.intervention,
        expected_evidence: plan.expected_evidence,
        owner_employee_id: plan.owner_employee_id,
        coordinator_employee_id: plan.coordinator_employee_id,
        provider_employee_id: plan.provider_employee_id,
        provider_name: plan.provider_name,
        start_on: plan.start_on,
        due_on: plan.due_on,
        status: "proposed".into(),
        closure: "open".into(),
        approved_by_user_id: None,
        approved_at: None,
        completed_at: None,
        completion_evidence: None,
        reassessment_due_on: None,
        post_assessment_id: None,
        post_level: None,
        improvement: None,
        next_steps: plan.next_steps,
        created_by_user_id: w.actor,
    };
    Ok((action_rows::insert(w, row, key).await?, false))
}

pub(crate) async fn revise(
    skills: &Skills,
    w: &mut Write<'_>,
    scope: &Scope,
    id: i64,
    input: &ActionPlan,
) -> Result<Action> {
    let plan = normalize(input)?;
    // Employment locks precede the workflow lock, as for creation.
    validate_plan(skills, w, scope, &plan).await?;
    let old = action_rows::lock(w, id).await?;
    if old.status != "proposed" {
        return Err(SkillsError::NotProposed);
    }
    let mut new = old.clone();
    apply_plan(&mut new, plan);
    action_rows::save(w, old, new, Event::new("revised")).await
}

/// Owner writes use Workforce's existing-transaction self lock, the same
/// proof as with_self_employee_lock, without nesting a mutation transaction.
pub(crate) async fn progress_owner(
    skills: &Skills,
    w: &Write<'_>,
    scope: &Scope,
) -> Result<Option<i64>> {
    if authorization::allowed(w.sql(), scope, w.company_id, ACTIONS_MANAGE).await {
        return Ok(None);
    }
    let me = skills
        .workforce
        .lock_self_employee(&w.tx, scope, w.company_id, ACTIONS_UPDATE)
        .await
        .map_err(|e| match e {
            bilibi_people_workforce::WorkforceError::NotLinked => SkillsError::NotFound,
            other => other.into(),
        })?;
    Ok(Some(me.employee_id))
}

pub(crate) enum Progress<'a> {
    Start,
    Hold(&'a str),
    Complete(&'a str, Date),
    Cancel(&'a str),
}
pub(crate) async fn progress(
    w: &mut Write<'_>,
    old: Action,
    command: Progress<'_>,
) -> Result<Action> {
    let mut new = old.clone();
    let event = match command {
        Progress::Start => {
            if !OPEN.contains(&old.status.as_str()) || old.status == "in_progress" {
                return Err(SkillsError::InvalidTransition);
            }
            new.status = "in_progress".into();
            Event::new("started")
        }
        Progress::Hold(reason) => {
            let reason = required(reason, SkillsError::ReasonRequired)?;
            if !["not_started", "scheduled", "in_progress"].contains(&old.status.as_str()) {
                return Err(SkillsError::InvalidTransition);
            }
            new.status = "on_hold".into();
            let mut e = Event::new("put_on_hold");
            e.comment = Some(reason);
            e
        }
        Progress::Complete(evidence, due) => {
            let evidence = required(evidence, SkillsError::EvidenceRequired)?;
            if due < assessments::today() {
                return Err(SkillsError::InvalidDate);
            }
            if !OPEN.contains(&old.status.as_str()) {
                return Err(SkillsError::InvalidTransition);
            }
            new.status = "pending_reassessment".into();
            new.closure = "pending_reassessment".into();
            new.completed_at = Some(now());
            new.completion_evidence = Some(evidence.clone());
            new.reassessment_due_on = Some(due);
            let mut e = Event::new("intervention_completed");
            e.evidence = Some(evidence);
            e
        }
        Progress::Cancel(reason) => {
            let reason = required(reason, SkillsError::ReasonRequired)?;
            if !OPEN.contains(&old.status.as_str())
                && !["proposed", "pending_reassessment"].contains(&old.status.as_str())
            {
                return Err(SkillsError::InvalidTransition);
            }
            new.status = "cancelled".into();
            new.closure = "cancelled".into();
            let mut e = Event::new("cancelled");
            e.comment = Some(reason);
            e
        }
    };
    action_rows::save(w, old, new, event).await
}

pub(crate) fn follows(a: &Assessment, action: &Action) -> bool {
    a.status == "finalized"
        && a.employee_id == action.employee_id
        && a.skill_id == action.skill_id
        && Some(a.id) != action.source_assessment_id
        && a.assessed_on
            .zip(action.completed_at)
            .is_some_and(|(d, t)| d >= t.date())
}
pub(crate) async fn close(w: &mut Write<'_>, old: Action, a: &Assessment) -> Result<Action> {
    if old.status != "pending_reassessment" {
        return Err(SkillsError::NotPendingReassessment);
    }
    if !follows(a, &old) {
        return Err(SkillsError::NotAReassessment);
    }
    let mut new = old.clone();
    new.status = "completed".into();
    new.closure = if a.assessed_level >= old.target_level {
        "closed_competent"
    } else {
        "further_action_required"
    }
    .into();
    new.post_assessment_id = Some(a.id);
    new.post_level = Some(a.assessed_level);
    new.improvement = Some(a.assessed_level - old.starting_level);
    let mut event = Event::new("reassessment_linked");
    event.evidence = Some(a.evidence.clone());
    action_rows::save(w, old, new, event).await
}

/// PK3's automatic closure is part of the finalization mutation, even if
/// that finalizer has no action-management grant. It cannot close an action
/// for another subject, skill, or an assessment before the intervention.
pub(crate) async fn close_after_finalization(w: &mut Write<'_>, a: &Assessment) -> Result<()> {
    let ids=w.sql().query("SELECT id FROM people_skill_actions WHERE tenant_id=$1 AND company_id=$2 AND employee_id=$3 AND skill_id=$4 AND status='pending_reassessment' AND source_assessment_id IS DISTINCT FROM $5 AND completed_at::date <= $6::text::date ORDER BY id FOR UPDATE", &[&w.tenant,&w.company_id,&a.employee_id,&a.skill_id,&a.id,&a.assessed_on.map(|d|d.to_string())]).await.map_err(Error::from)?;
    for r in ids {
        let old = action_rows::lock(w, r.get(0)).await?;
        close(w, old, a).await?;
    }
    Ok(())
}

pub(crate) async fn list(
    client: &impl GenericClient,
    tenant: i64,
    company: i64,
    group: ActionGroup,
    reach: &access::Reach,
    owner: Option<i64>,
    owned_only: bool,
) -> Result<Vec<Action>> {
    let closed = group == ActionGroup::Closed;
    let ids = reach.ids();
    let order = if closed {
        "updated_at DESC,id DESC"
    } else {
        "mandatory DESC,priority_score DESC,due_on,id"
    };
    client.query(&format!("SELECT {} FROM people_skill_actions WHERE tenant_id=$1 AND company_id=$2 AND ((status IN ('completed','cancelled'))=$3) AND (($6 AND owner_employee_id=$5) OR (NOT $6 AND ($4::bigint[] IS NULL OR employee_id=ANY($4) OR owner_employee_id=$5))) ORDER BY {order} LIMIT 300",action_rows::action_columns()), &[&tenant,&company,&closed,&ids,&owner,&owned_only]).await.map_err(Error::from)?.iter().map(action_rows::action).collect::<DbResult<Vec<_>>>().map_err(Into::into)
}

pub(crate) async fn present(
    skills: &Skills,
    client: &impl GenericClient,
    scope: &Scope,
    company: i64,
    rows: Vec<Action>,
) -> Result<Vec<ActionView>> {
    let ids = rows
        .iter()
        .flat_map(|r| [r.owner_employee_id, r.coordinator_employee_id])
        .collect::<Vec<_>>();
    let names = access::names(skills, scope, company, &ids).await?;
    let catalog = catalog::skills_by_ids(
        client,
        scope.tenant(),
        company,
        &rows.iter().map(|a| a.skill_id).collect::<Vec<_>>(),
    )
    .await?;
    let types = types(client, scope.tenant(), company).await?;
    let mut result = Vec::new();
    for row in rows {
        let mut candidates = Vec::new();
        if row.status == "pending_reassessment" {
            let ids=client.query("SELECT id FROM people_skill_assessments WHERE tenant_id=$1 AND company_id=$2 AND employee_id=$3 AND skill_id=$4 AND status='finalized' AND assessed_on >= $5::text::date AND id IS DISTINCT FROM $6 ORDER BY assessed_on DESC,id DESC", &[&scope.tenant(),&company,&row.employee_id,&row.skill_id,&row.completed_at.map(|t|t.date().to_string()),&row.source_assessment_id]).await.map_err(Error::from)?;
            for r in ids {
                let a = assessments::get(client, scope.tenant(), company, r.get(0)).await?;
                candidates.push(ActionReassessment {
                    id: a.id,
                    assessed_on: a.assessed_on,
                    assessed_level: a.assessed_level,
                });
            }
        }
        result.push(ActionView {
            skill_name: catalog
                .iter()
                .find(|s| s.id == row.skill_id)
                .map(|s| s.name.clone()),
            type_name: types
                .iter()
                .find(|t| t.id == row.action_type_id)
                .map(|t| t.name.clone()),
            owner_name: names
                .get(&row.owner_employee_id)
                .cloned()
                .unwrap_or_else(|| format!("Employee #{}", row.owner_employee_id)),
            coordinator_name: names
                .get(&row.coordinator_employee_id)
                .cloned()
                .unwrap_or_else(|| format!("Employee #{}", row.coordinator_employee_id)),
            action: row,
            reassessments: candidates,
        });
    }
    Ok(result)
}
