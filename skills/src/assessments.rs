//! Bilimbi Skills.Assessments: immutable evidence, decision ledger and score
//! projection. All mutations run on the facade's locked company transaction.
use crate::{
    access, catalog, policy, profiles, rows, Assessment, AssessmentDecision, AssessmentFilters,
    AssessmentInput, AssessmentView, Result, ReviewDecision, Score, Skills, SkillsError, Write,
};
use bilibi_database::{Error, Result as DbResult};
use bilibi_tenancy::Scope;
use jiff::civil::Date;
use tokio_postgres::{GenericClient, Row};

pub(crate) fn today() -> Date {
    jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date()
}

pub(crate) fn band(gap: i32, assessed: i32, required: i32) -> &'static str {
    match gap {
        0 if assessed > required => "exceeds",
        0 => "meets",
        1 => "minor_gap",
        2 => "major_gap",
        _ => "critical_gap",
    }
}

pub(crate) fn normalize(input: &AssessmentInput) -> Result<AssessmentInput> {
    fn text(value: &str, max: usize, required: bool) -> Result<Option<String>> {
        let value = value.trim();
        if (required && value.is_empty()) || value.chars().count() > max {
            return Err(SkillsError::InvalidAssessment);
        }
        Ok((!value.is_empty()).then(|| value.to_owned()))
    }
    if input.employee_id <= 0
        || input.skill_id <= 0
        || input.supersedes_assessment_id.is_some_and(|id| id <= 0)
    {
        return Err(SkillsError::InvalidAssessment);
    }
    let assessed_on = input.assessed_on.unwrap_or_else(today);
    if assessed_on > today() {
        return Err(SkillsError::FutureAssessment);
    }
    if input.valid_until.is_some_and(|d| d < assessed_on) {
        return Err(SkillsError::InvalidValidity);
    }
    Ok(AssessmentInput {
        assessed_on: Some(assessed_on),
        evidence: text(&input.evidence, 4000, true)?.unwrap_or_default(),
        request_key: text(&input.request_key, 80, true)?.unwrap_or_default(),
        method: text(input.method.as_deref().unwrap_or_default(), 80, false)?,
        notes: text(input.notes.as_deref().unwrap_or_default(), 2000, false)?,
        ..input.clone()
    })
}

pub(crate) fn note(note: Option<&str>, decision: ReviewDecision) -> Result<Option<String>> {
    let note = note.map(str::trim).filter(|s| !s.is_empty());
    if decision == ReviewDecision::Return && note.is_none() {
        return Err(SkillsError::NoteRequired);
    }
    if note.is_some_and(|s| s.chars().count() > 2000) {
        return Err(SkillsError::InvalidNote);
    }
    Ok(note.map(str::to_owned))
}

fn columns() -> String {
    format!("id, employee_id, skill_id, profile_id, scale_id, required_level, criticality, weight_percent, mandatory, assessed_level, gap, priority_multiplier, priority_score, result_band, method, evidence, notes, {}, {}, {}, status, assessor_user_id, reviewed_by_user_id, {}, review_note, finalized_by_user_id, {}, supersedes_assessment_id",
        rows::date_text("assessed_on"), rows::date_text("valid_until"), rows::date_text("next_due_on"), rows::datetime_text("reviewed_at"), rows::datetime_text("finalized_at"))
}
fn assessment(r: &Row) -> DbResult<Assessment> {
    Ok(Assessment {
        id: r.get("id"),
        employee_id: r.get("employee_id"),
        skill_id: r.get("skill_id"),
        profile_id: r.get("profile_id"),
        scale_id: r.get("scale_id"),
        required_level: r.get("required_level"),
        criticality: r.get("criticality"),
        weight_percent: r.get("weight_percent"),
        mandatory: r.get("mandatory"),
        assessed_level: r.get("assessed_level"),
        gap: r.get("gap"),
        priority_multiplier: r.get("priority_multiplier"),
        priority_score: r.get("priority_score"),
        result_band: r.get("result_band"),
        method: r.get("method"),
        evidence: r.get("evidence"),
        notes: r.get("notes"),
        assessed_on: rows::date(r, "assessed_on")?,
        valid_until: rows::date(r, "valid_until")?,
        next_due_on: rows::date(r, "next_due_on")?,
        status: r.get("status"),
        assessor_user_id: r.get("assessor_user_id"),
        reviewed_by_user_id: r.get("reviewed_by_user_id"),
        reviewed_at: rows::datetime(r, "reviewed_at")?,
        review_note: r.get("review_note"),
        finalized_by_user_id: r.get("finalized_by_user_id"),
        finalized_at: rows::datetime(r, "finalized_at")?,
        supersedes_assessment_id: r.get("supersedes_assessment_id"),
    })
}

pub(crate) async fn get(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company: i64,
    id: i64,
) -> Result<Assessment> {
    let row = client.query_opt(&format!("SELECT {} FROM people_skill_assessments WHERE tenant_id=$1 AND company_id=$2 AND id=$3", columns()), &[&tenant,&company,&id]).await.map_err(Error::from)?.ok_or(SkillsError::NotFound)?;
    Ok(assessment(&row)?)
}

pub(crate) async fn lock(w: &Write<'_>, id: i64) -> Result<Assessment> {
    let row = w.sql().query_opt(&format!("SELECT {} FROM people_skill_assessments WHERE tenant_id=$1 AND company_id=$2 AND id=$3 FOR UPDATE",columns()), &[&w.tenant,&w.company_id,&id]).await.map_err(Error::from)?.ok_or(SkillsError::NotFound)?;
    Ok(assessment(&row)?)
}

pub(crate) async fn replay(w: &Write<'_>, input: &AssessmentInput) -> Result<Option<Assessment>> {
    let row = w.sql().query_opt(&format!("SELECT {} FROM people_skill_assessments WHERE tenant_id=$1 AND company_id=$2 AND assessor_user_id=$3 AND request_key=$4",columns()), &[&w.tenant,&w.company_id,&w.actor,&input.request_key]).await.map_err(Error::from)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let old = assessment(&row)?;
    // Bilimbi deliberately compares these five facts. Optional metadata and
    // later decision state do not turn a retry into another submission.
    if old.employee_id != input.employee_id
        || old.skill_id != input.skill_id
        || old.assessed_level != input.assessed_level
        || old.assessed_on != input.assessed_on
        || old.evidence != input.evidence
    {
        return Err(SkillsError::KeyConflict);
    }
    Ok(Some(old))
}

async fn supersession(w: &Write<'_>, input: &AssessmentInput) -> Result<()> {
    let Some(id) = input.supersedes_assessment_id else {
        return Ok(());
    };
    let prior = match lock(w, id).await {
        Err(SkillsError::NotFound) => return Err(SkillsError::NotSupersedable),
        other => other?,
    };
    if prior.employee_id != input.employee_id || prior.skill_id != input.skill_id {
        return Err(SkillsError::NotSupersedable);
    }
    if prior.status == "returned" && prior.assessor_user_id != w.actor {
        return Err(SkillsError::NotOriginalAssessor);
    }
    if !["returned", "finalized"].contains(&prior.status.as_str()) {
        return Err(SkillsError::NotSupersedable);
    }
    if w.sql().query_opt("SELECT 1 FROM people_skill_assessments WHERE tenant_id=$1 AND company_id=$2 AND supersedes_assessment_id=$3", &[&w.tenant,&w.company_id,&id]).await.map_err(Error::from)?.is_some() {
        return Err(SkillsError::AlreadySuperseded);
    }
    Ok(())
}

pub(crate) async fn insert(
    skills: &Skills,
    w: &mut Write<'_>,
    scope: &Scope,
    input: &AssessmentInput,
) -> Result<Assessment> {
    let skill = catalog::get_skill(w, input.skill_id)
        .await?
        .filter(|s| s.active)
        .ok_or(SkillsError::AssessmentSkillUnavailable)?;
    let assessed_on = input.assessed_on.ok_or(SkillsError::InvalidAssessment)?;
    let position = crate::positions::position_of(
        &skills.workforce,
        scope,
        w.company_id,
        input.employee_id,
        assessed_on,
    )
    .await?;
    let mut matches =
        profiles::in_force(w.sql(), w.tenant, w.company_id, position, assessed_on).await?;
    if matches.len() != 1 {
        return Err(SkillsError::NoRequirement);
    }
    let detail = profiles::detail(w.sql(), matches.remove(0)).await?;
    let item = detail
        .items
        .iter()
        .find(|i| i.skill_id == input.skill_id)
        .ok_or(SkillsError::NoRequirement)?;
    if !crate::scales::levels_of(w.sql(), detail.profile.scale_id)
        .await?
        .iter()
        .any(|l| l.level == input.assessed_level)
    {
        return Err(SkillsError::AssessmentLevelNotOnScale);
    }
    let policy = policy::get(&skills.settings, w.sql(), scope, w.company_id).await?;
    supersession(w, input).await?;
    let gap = (item.required_level - input.assessed_level).max(0);
    let multiplier = policy.multiplier(&item.criticality) as i32;
    // Jiff clamps a month-end day, as Bilimbi's add_months does.
    let next_due = match input.valid_until {
        Some(date) => date,
        None => assessed_on
            .checked_add(
                jiff::Span::new().months(
                    skill
                        .reassessment_months
                        .map(i64::from)
                        .unwrap_or(policy.default_reassessment_months),
                ),
            )
            .map_err(|_| SkillsError::InvalidAssessment)?,
    };
    let row = Assessment {
        id: 0,
        employee_id: input.employee_id,
        skill_id: input.skill_id,
        profile_id: detail.profile.id,
        scale_id: detail.profile.scale_id,
        required_level: item.required_level,
        criticality: item.criticality.clone(),
        weight_percent: item.weight_percent,
        mandatory: item.mandatory,
        assessed_level: input.assessed_level,
        gap,
        priority_multiplier: multiplier,
        priority_score: gap * multiplier,
        result_band: band(gap, input.assessed_level, item.required_level).into(),
        method: input.method.clone(),
        evidence: input.evidence.clone(),
        notes: input.notes.clone(),
        assessed_on: Some(assessed_on),
        valid_until: input.valid_until,
        next_due_on: Some(next_due),
        status: "pending_review".into(),
        assessor_user_id: w.actor,
        reviewed_by_user_id: None,
        reviewed_at: None,
        review_note: None,
        finalized_by_user_id: None,
        finalized_at: None,
        supersedes_assessment_id: input.supersedes_assessment_id,
    };
    let (tenant, company, key) = (w.tenant, w.company_id, input.request_key.clone());
    Ok(w.tx.mutate(move |sql| Box::pin(async move {
        let dates=(row.assessed_on.map(|d|d.to_string()),row.valid_until.map(|d|d.to_string()),row.next_due_on.map(|d|d.to_string()));
        let r=sql.query_one(&format!("INSERT INTO people_skill_assessments (tenant_id,company_id,employee_id,skill_id,profile_id,scale_id,required_level,criticality,weight_percent,mandatory,assessed_level,gap,priority_multiplier,priority_score,result_band,method,evidence,notes,assessed_on,valid_until,next_due_on,status,assessor_user_id,supersedes_assessment_id,request_key) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19::text::date,$20::text::date,$21::text::date,'pending_review',$22,$23,$24) RETURNING {}",columns()),
            &[&tenant,&company,&row.employee_id,&row.skill_id,&row.profile_id,&row.scale_id,&row.required_level,&row.criticality,&row.weight_percent,&row.mandatory,&row.assessed_level,&row.gap,&row.priority_multiplier,&row.priority_score,&row.result_band,&row.method,&row.evidence,&row.notes,&dates.0,&dates.1,&dates.2,&row.assessor_user_id,&row.supersedes_assessment_id,&key]).await?;
        let row=assessment(&r)?;
        let decision=insert_decision(sql,tenant,company,row.id,"submitted",row.assessor_user_id,None).await?;
        let mut audit=serde_json::to_value(&row).map_err(|_|Error::Internal)?;
        audit["request_key"]=key.into();
        let changes=vec![rows::created("people_skill_assessment",row.id,&audit),rows::created("people_skill_assessment_decision",decision.id,&decision)];
        Ok((row,changes))
    })).await?)
}

async fn insert_decision(
    sql: &(impl GenericClient + Sync),
    tenant: i64,
    company: i64,
    id: i64,
    decision: &str,
    actor: i64,
    note: Option<&str>,
) -> DbResult<AssessmentDecision> {
    let r=sql.query_one(&format!("INSERT INTO people_skill_assessment_decisions (tenant_id,company_id,assessment_id,decision,actor_user_id,note) VALUES ($1,$2,$3,$4,$5,$6) RETURNING id,assessment_id,decision,actor_user_id,note,{}",rows::datetime_text("inserted_at")), &[&tenant,&company,&id,&decision,&actor,&note]).await?;
    decision_row(&r)
}
fn decision_row(r: &Row) -> DbResult<AssessmentDecision> {
    Ok(AssessmentDecision {
        id: r.get("id"),
        assessment_id: r.get("assessment_id"),
        decision: r.get("decision"),
        actor_user_id: r.get("actor_user_id"),
        note: r.get("note"),
        inserted_at: rows::datetime(r, "inserted_at")?,
    })
}

pub(crate) async fn decide(
    w: &mut Write<'_>,
    old: Assessment,
    decision: &str,
    note: Option<String>,
) -> Result<Assessment> {
    let (tenant, company, actor, decision) = (w.tenant, w.company_id, w.actor, decision.to_owned());
    Ok(w.tx.mutate(move |sql| Box::pin(async move {
        let r=if decision=="finalized" {
            sql.query_one(&format!("UPDATE people_skill_assessments SET status='finalized',finalized_by_user_id=$4,finalized_at=(now() AT TIME ZONE 'UTC'),updated_at=(now() AT TIME ZONE 'UTC') WHERE tenant_id=$1 AND company_id=$2 AND id=$3 RETURNING {}",columns()), &[&tenant,&company,&old.id,&actor]).await?
        } else {
            sql.query_one(&format!("UPDATE people_skill_assessments SET status=$4,reviewed_by_user_id=$5,reviewed_at=(now() AT TIME ZONE 'UTC'),review_note=$6,updated_at=(now() AT TIME ZONE 'UTC') WHERE tenant_id=$1 AND company_id=$2 AND id=$3 RETURNING {}",columns()), &[&tenant,&company,&old.id,&decision,&actor,&note]).await?
        };
        let row=assessment(&r)?;
        let d=insert_decision(sql,tenant,company,row.id,&decision,actor,note.as_deref()).await?;
        Ok((row.clone(),vec![rows::changed("people_skill_assessment",row.id,&old,&row),rows::created("people_skill_assessment_decision",d.id,&d)]))
    })).await?)
}

pub(crate) async fn refresh_score(w: &mut Write<'_>, row: &Assessment) -> Result<()> {
    // Every ancestor of a finalized correction is excluded, including a
    // returned node in the middle. A pending correction leaves the score alone.
    let latest=w.sql().query_one("WITH RECURSIVE replaced(id) AS (SELECT supersedes_assessment_id FROM people_skill_assessments WHERE tenant_id=$1 AND company_id=$2 AND employee_id=$3 AND skill_id=$4 AND status='finalized' AND supersedes_assessment_id IS NOT NULL UNION SELECT a.supersedes_assessment_id FROM people_skill_assessments a JOIN replaced r ON a.id=r.id WHERE a.tenant_id=$1 AND a.company_id=$2 AND a.employee_id=$3 AND a.skill_id=$4 AND a.supersedes_assessment_id IS NOT NULL) SELECT id FROM people_skill_assessments WHERE tenant_id=$1 AND company_id=$2 AND employee_id=$3 AND skill_id=$4 AND status='finalized' AND id NOT IN (SELECT id FROM replaced) ORDER BY assessed_on DESC,id DESC LIMIT 1", &[&w.tenant,&w.company_id,&row.employee_id,&row.skill_id]).await.map_err(Error::from)?;
    let source = get(w.sql(), w.tenant, w.company_id, latest.get("id")).await?;
    let old=w.sql().query_opt(&format!("SELECT {} FROM people_skill_scores WHERE tenant_id=$1 AND company_id=$2 AND employee_id=$3 AND skill_id=$4",score_columns("")), &[&w.tenant,&w.company_id,&row.employee_id,&row.skill_id]).await.map_err(Error::from)?.as_ref().map(score).transpose()?;
    let (tenant, company) = (w.tenant, w.company_id);
    w.tx.mutate(move |sql| Box::pin(async move {
        let dates=(source.assessed_on.map(|d|d.to_string()),source.valid_until.map(|d|d.to_string()),source.next_due_on.map(|d|d.to_string()));
        let r=sql.query_one(&format!("INSERT INTO people_skill_scores (tenant_id,company_id,employee_id,skill_id,assessment_id,profile_id,required_level,current_level,gap,criticality,mandatory,priority_score,assessed_on,valid_until,next_due_on) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13::text::date,$14::text::date,$15::text::date) ON CONFLICT (company_id,employee_id,skill_id) DO UPDATE SET assessment_id=EXCLUDED.assessment_id,profile_id=EXCLUDED.profile_id,required_level=EXCLUDED.required_level,current_level=EXCLUDED.current_level,gap=EXCLUDED.gap,criticality=EXCLUDED.criticality,mandatory=EXCLUDED.mandatory,priority_score=EXCLUDED.priority_score,assessed_on=EXCLUDED.assessed_on,valid_until=EXCLUDED.valid_until,next_due_on=EXCLUDED.next_due_on,updated_at=(now() AT TIME ZONE 'UTC') RETURNING {}",score_columns("")), &[&tenant,&company,&source.employee_id,&source.skill_id,&source.id,&source.profile_id,&source.required_level,&source.assessed_level,&source.gap,&source.criticality,&source.mandatory,&source.priority_score,&dates.0,&dates.1,&dates.2]).await?;
        let new=score(&r)?;
        let change=match old {Some(old)=>rows::changed("people_skill_score",new.id,&old,&new),None=>rows::created("people_skill_score",new.id,&new)};
        Ok(((),vec![change]))
    })).await?;
    Ok(())
}

pub(crate) fn score_columns(prefix: &str) -> String {
    let fields="id,employee_id,skill_id,assessment_id,profile_id,required_level,current_level,gap,criticality,mandatory,priority_score".split(',').map(|f|format!("{prefix}{f}")).collect::<Vec<_>>().join(",");
    format!(
        "{fields},{},{},{}",
        rows::date_text(&format!("{prefix}assessed_on")),
        rows::date_text(&format!("{prefix}valid_until")),
        rows::date_text(&format!("{prefix}next_due_on"))
    )
}
pub(crate) fn score(r: &Row) -> DbResult<Score> {
    Ok(Score {
        id: r.get("id"),
        employee_id: r.get("employee_id"),
        skill_id: r.get("skill_id"),
        assessment_id: r.get("assessment_id"),
        profile_id: r.get("profile_id"),
        required_level: r.get("required_level"),
        current_level: r.get("current_level"),
        gap: r.get("gap"),
        criticality: r.get("criticality"),
        mandatory: r.get("mandatory"),
        priority_score: r.get("priority_score"),
        assessed_on: rows::date(r, "assessed_on")?,
        valid_until: rows::date(r, "valid_until")?,
        next_due_on: rows::date(r, "next_due_on")?,
    })
}

pub(crate) async fn list(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company: i64,
    reach: &access::Reach,
    filters: &AssessmentFilters,
) -> Result<Vec<Assessment>> {
    let ids = reach.ids();
    let sql=format!("SELECT {} FROM people_skill_assessments WHERE tenant_id=$1 AND company_id=$2 AND ($3::bigint[] IS NULL OR employee_id=ANY($3)) AND ($4::text IS NULL OR status=$4) AND ($5::bigint IS NULL OR employee_id=$5) AND ($6::bigint IS NULL OR skill_id=$6) ORDER BY id DESC LIMIT 200",columns());
    client
        .query(
            &sql,
            &[
                &tenant,
                &company,
                &ids,
                &filters.status.as_deref().filter(|s| !s.is_empty()),
                &filters.employee_id,
                &filters.skill_id,
            ],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(assessment)
        .collect::<DbResult<Vec<_>>>()
        .map_err(Into::into)
}

/// Bilimbi bounds the oldest 200 candidates before rejecting work by the
/// assessor or subject. Returned work is filtered to the actor before its bound.
pub(crate) async fn queue_rows(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company: i64,
    ids: Option<Vec<i64>>,
    actor: i64,
    own: Option<i64>,
    status: &str,
) -> Result<Vec<Assessment>> {
    let sql=format!("SELECT {} FROM (SELECT a.* FROM people_skill_assessments a WHERE tenant_id=$1 AND company_id=$2 AND status=$3 AND ($4::bigint[] IS NULL OR employee_id=ANY($4)) AND ($3<>'returned' OR (assessor_user_id=$5 AND NOT EXISTS (SELECT 1 FROM people_skill_assessments s WHERE s.tenant_id=$1 AND s.company_id=$2 AND s.supersedes_assessment_id=a.id))) ORDER BY id LIMIT 200) candidates WHERE $3='returned' OR (assessor_user_id<>$5 AND ($6::bigint IS NULL OR employee_id<>$6)) ORDER BY id",columns());
    client
        .query(&sql, &[&tenant, &company, &status, &ids, &actor, &own])
        .await
        .map_err(Error::from)?
        .iter()
        .map(assessment)
        .collect::<DbResult<Vec<_>>>()
        .map_err(Into::into)
}

pub(crate) async fn present(
    skills: &Skills,
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company: i64,
    rows: Vec<Assessment>,
) -> Result<Vec<AssessmentView>> {
    let names = access::names(
        skills,
        scope,
        company,
        &rows.iter().map(|a| a.employee_id).collect::<Vec<_>>(),
    )
    .await?;
    let catalog = catalog::skills_by_ids(
        client,
        scope.tenant(),
        company,
        &rows.iter().map(|a| a.skill_id).collect::<Vec<_>>(),
    )
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let skill = catalog.iter().find(|s| s.id == row.skill_id);
            AssessmentView {
                employee_name: names
                    .get(&row.employee_id)
                    .cloned()
                    .unwrap_or_else(|| format!("Employee #{}", row.employee_id)),
                skill_code: skill.map(|s| s.code.clone()),
                skill_name: skill.map(|s| s.name.clone()),
                assessment: row,
            }
        })
        .collect())
}

pub(crate) async fn decisions(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company: i64,
    id: i64,
) -> Result<Vec<AssessmentDecision>> {
    client.query(&format!("SELECT id,assessment_id,decision,actor_user_id,note,{} FROM people_skill_assessment_decisions WHERE tenant_id=$1 AND company_id=$2 AND assessment_id=$3 ORDER BY id",rows::datetime_text("inserted_at")), &[&tenant,&company,&id]).await.map_err(Error::from)?.iter().map(decision_row).collect::<DbResult<Vec<_>>>().map_err(Into::into)
}
