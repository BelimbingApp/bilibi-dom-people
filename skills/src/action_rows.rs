//! Bilimbi Skills.Actions persistence: every action and append-only event
//! shares the deciding company's MutationBoundary transaction.
use crate::{rows, Action, ActionEvent, Result, SkillsError, Write};
use bilibi_database::{Error, Result as DbResult};
use tokio_postgres::{GenericClient, Row};
pub(crate) fn action_columns() -> String {
    [
        "id".into(),
        "employee_id".into(),
        "employee_name".into(),
        "skill_id".into(),
        "source_assessment_id".into(),
        "action_type_id".into(),
        "starting_level".into(),
        "target_level".into(),
        "gap_at_start".into(),
        "criticality".into(),
        "mandatory".into(),
        "priority_multiplier".into(),
        "priority_score".into(),
        "priority_explanation".into(),
        "manual_reason".into(),
        "objective".into(),
        "intervention".into(),
        "expected_evidence".into(),
        "owner_employee_id".into(),
        "coordinator_employee_id".into(),
        "provider_employee_id".into(),
        "provider_name".into(),
        rows::date_text("start_on"),
        rows::date_text("due_on"),
        "status".into(),
        "closure".into(),
        "approved_by_user_id".into(),
        rows::datetime_text("approved_at"),
        rows::datetime_text("completed_at"),
        "completion_evidence".into(),
        rows::date_text("reassessment_due_on"),
        "post_assessment_id".into(),
        "post_level".into(),
        "improvement".into(),
        "next_steps".into(),
        "created_by_user_id".into(),
    ]
    .join(",")
}
pub(crate) fn action(r: &Row) -> DbResult<Action> {
    Ok(Action {
        id: r.get("id"),
        employee_id: r.get("employee_id"),
        employee_name: r.get("employee_name"),
        skill_id: r.get("skill_id"),
        source_assessment_id: r.get("source_assessment_id"),
        action_type_id: r.get("action_type_id"),
        starting_level: r.get("starting_level"),
        target_level: r.get("target_level"),
        gap_at_start: r.get("gap_at_start"),
        criticality: r.get("criticality"),
        mandatory: r.get("mandatory"),
        priority_multiplier: r.get("priority_multiplier"),
        priority_score: r.get("priority_score"),
        priority_explanation: r.get("priority_explanation"),
        manual_reason: r.get("manual_reason"),
        objective: r.get("objective"),
        intervention: r.get("intervention"),
        expected_evidence: r.get("expected_evidence"),
        owner_employee_id: r.get("owner_employee_id"),
        coordinator_employee_id: r.get("coordinator_employee_id"),
        provider_employee_id: r.get("provider_employee_id"),
        provider_name: r.get("provider_name"),
        start_on: rows::date(r, "start_on")?,
        due_on: rows::date(r, "due_on")?,
        status: r.get("status"),
        closure: r.get("closure"),
        approved_by_user_id: r.get("approved_by_user_id"),
        approved_at: rows::datetime(r, "approved_at")?,
        completed_at: rows::datetime(r, "completed_at")?,
        completion_evidence: r.get("completion_evidence"),
        reassessment_due_on: rows::date(r, "reassessment_due_on")?,
        post_assessment_id: r.get("post_assessment_id"),
        post_level: r.get("post_level"),
        improvement: r.get("improvement"),
        next_steps: r.get("next_steps"),
        created_by_user_id: r.get("created_by_user_id"),
    })
}
pub(crate) fn event_row_columns() -> String {
    [
        "id".into(),
        "action_id".into(),
        "event_type".into(),
        "from_status".into(),
        "to_status".into(),
        "comment".into(),
        "evidence".into(),
        "actor_user_id".into(),
        rows::datetime_text("inserted_at"),
    ]
    .join(",")
}
pub(crate) fn event_row(r: &Row) -> DbResult<ActionEvent> {
    Ok(ActionEvent {
        id: r.get("id"),
        action_id: r.get("action_id"),
        event_type: r.get("event_type"),
        from_status: r.get("from_status"),
        to_status: r.get("to_status"),
        comment: r.get("comment"),
        evidence: r.get("evidence"),
        actor_user_id: r.get("actor_user_id"),
        inserted_at: rows::datetime(r, "inserted_at")?,
    })
}

pub(crate) async fn get(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company: i64,
    id: i64,
) -> Result<Action> {
    let r=client.query_opt(&format!("SELECT {} FROM people_skill_actions WHERE tenant_id=$1 AND company_id=$2 AND id=$3",action_columns()), &[&tenant,&company,&id]).await.map_err(Error::from)?.ok_or(SkillsError::NotFound)?;
    Ok(action(&r)?)
}
pub(crate) async fn lock(w: &Write<'_>, id: i64) -> Result<Action> {
    let r=w.sql().query_opt(&format!("SELECT {} FROM people_skill_actions WHERE tenant_id=$1 AND company_id=$2 AND id=$3 FOR UPDATE",action_columns()), &[&w.tenant,&w.company_id,&id]).await.map_err(Error::from)?.ok_or(SkillsError::NotFound)?;
    Ok(action(&r)?)
}

pub(crate) struct Event<'a> {
    pub kind: &'a str,
    pub comment: Option<String>,
    pub evidence: Option<String>,
}
impl<'a> Event<'a> {
    pub fn new(kind: &'a str) -> Self {
        Self {
            kind,
            comment: None,
            evidence: None,
        }
    }
}

pub(crate) async fn insert(w: &mut Write<'_>, new: Action, key: String) -> Result<Action> {
    let (tenant, company, actor) = (w.tenant, w.company_id, w.actor);
    Ok(w.tx.mutate(move |sql| Box::pin(async move {
        let start_on=new.start_on.map(|d|d.to_string());
        let due_on=new.due_on.map(|d|d.to_string());
        let approved_at=new.approved_at.map(|d|d.to_string());
        let completed_at=new.completed_at.map(|d|d.to_string());
        let reassessment_due_on=new.reassessment_due_on.map(|d|d.to_string());
        let r=sql.query_one(&format!("INSERT INTO people_skill_actions (tenant_id,company_id,request_key,employee_id,employee_name,skill_id,source_assessment_id,action_type_id,starting_level,target_level,gap_at_start,criticality,mandatory,priority_multiplier,priority_score,priority_explanation,manual_reason,objective,intervention,expected_evidence,owner_employee_id,coordinator_employee_id,provider_employee_id,provider_name,start_on,due_on,status,closure,approved_by_user_id,approved_at,completed_at,completion_evidence,reassessment_due_on,post_assessment_id,post_level,improvement,next_steps,created_by_user_id) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23,$24,$25::text::date,$26::text::date,$27,$28,$29,$30::text::timestamp,$31::text::timestamp,$32,$33::text::date,$34,$35,$36,$37,$38) RETURNING {}",action_columns()), &[&tenant,&company,&key,&new.employee_id,&new.employee_name,&new.skill_id,&new.source_assessment_id,&new.action_type_id,&new.starting_level,&new.target_level,&new.gap_at_start,&new.criticality,&new.mandatory,&new.priority_multiplier,&new.priority_score,&new.priority_explanation,&new.manual_reason,&new.objective,&new.intervention,&new.expected_evidence,&new.owner_employee_id,&new.coordinator_employee_id,&new.provider_employee_id,&new.provider_name,&start_on,&due_on,&new.status,&new.closure,&new.approved_by_user_id,&approved_at,&completed_at,&new.completion_evidence,&reassessment_due_on,&new.post_assessment_id,&new.post_level,&new.improvement,&new.next_steps,&new.created_by_user_id]).await?;
        let row=action(&r)?;
        let event=insert_event(sql,(tenant,company,row.id,actor),"proposed",(None,Some("proposed")),None,None).await?;
        let mut audit=serde_json::to_value(&row).map_err(|_|Error::Internal)?;
        audit["request_key"]=key.into();
        Ok((row.clone(),vec![rows::created("people_skill_action",row.id,&audit),rows::created("people_skill_action_event",event.id,&event)]))
    })).await?)
}

pub(crate) async fn save(
    w: &mut Write<'_>,
    old: Action,
    new: Action,
    event: Event<'_>,
) -> Result<Action> {
    let (tenant, company, actor) = (w.tenant, w.company_id, w.actor);
    let kind = event.kind.to_owned();
    Ok(w.tx.mutate(move |sql| Box::pin(async move {
        let start_on=new.start_on.map(|d|d.to_string());
        let due_on=new.due_on.map(|d|d.to_string());
        let approved_at=new.approved_at.map(|d|d.to_string());
        let completed_at=new.completed_at.map(|d|d.to_string());
        let reassessment_due_on=new.reassessment_due_on.map(|d|d.to_string());
        let r=sql.query_one(&format!("UPDATE people_skill_actions SET action_type_id=$4,objective=$5,intervention=$6,expected_evidence=$7,owner_employee_id=$8,coordinator_employee_id=$9,provider_employee_id=$10,provider_name=$11,start_on=$12::text::date,due_on=$13::text::date,status=$14,closure=$15,approved_by_user_id=$16,approved_at=$17::text::timestamp,completed_at=$18::text::timestamp,completion_evidence=$19,reassessment_due_on=$20::text::date,post_assessment_id=$21,post_level=$22,improvement=$23,next_steps=$24,updated_at=(now() AT TIME ZONE 'UTC') WHERE tenant_id=$1 AND company_id=$2 AND id=$3 RETURNING {}",action_columns()), &[&tenant,&company,&old.id,&new.action_type_id,&new.objective,&new.intervention,&new.expected_evidence,&new.owner_employee_id,&new.coordinator_employee_id,&new.provider_employee_id,&new.provider_name,&start_on,&due_on,&new.status,&new.closure,&new.approved_by_user_id,&approved_at,&completed_at,&new.completion_evidence,&reassessment_due_on,&new.post_assessment_id,&new.post_level,&new.improvement,&new.next_steps]).await?;
        let row=action(&r)?;
        let event=insert_event(sql,(tenant,company,row.id,actor),&kind,(Some(&old.status),Some(&row.status)),event.comment.as_deref(),event.evidence.as_deref()).await?;
        Ok((row.clone(),vec![rows::changed("people_skill_action",row.id,&old,&row),rows::created("people_skill_action_event",event.id,&event)]))
    })).await?)
}

// One fact per call, including comments on a terminal action. A comment never
// rewrites the immutable action, but is itself a captured mutation.
pub(crate) async fn comment(
    w: &mut Write<'_>,
    row: &Action,
    comment: String,
    evidence: Option<String>,
) -> Result<()> {
    let (tenant, company, actor, id) = (w.tenant, w.company_id, w.actor, row.id);
    w.tx.mutate(move |sql| {
        Box::pin(async move {
            let event = insert_event(
                sql,
                (tenant, company, id, actor),
                "commented",
                (None, None),
                Some(&comment),
                evidence.as_deref(),
            )
            .await?;
            Ok((
                (),
                vec![rows::created("people_skill_action_event", event.id, &event)],
            ))
        })
    })
    .await?;
    Ok(())
}

async fn insert_event(
    sql: &(impl GenericClient + Sync),
    identity: (i64, i64, i64, i64),
    kind: &str,
    statuses: (Option<&str>, Option<&str>),
    comment: Option<&str>,
    evidence: Option<&str>,
) -> DbResult<ActionEvent> {
    let (tenant, company, id, actor) = identity;
    let (from, to) = statuses;
    let r=sql.query_one(&format!("INSERT INTO people_skill_action_events (tenant_id,company_id,action_id,actor_user_id,event_type,from_status,to_status,comment,evidence) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) RETURNING {}",event_row_columns()), &[&tenant,&company,&id,&actor,&kind,&from,&to,&comment,&evidence]).await?;
    event_row(&r)
}
