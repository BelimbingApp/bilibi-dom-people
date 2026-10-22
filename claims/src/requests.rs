//! Request records, validation and persistence, mirroring Bilimbi Claims.Request.
use crate::{money, ClaimsError, FieldError, Result, Tx};
use bilibi_database::{Change, Error};
use jiff::civil::Date;
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::json;
use tokio_postgres::{GenericClient, Row};

#[derive(Clone, Debug)]
pub struct NewRequest {
    pub claim_type_id: i64,
    pub incurred_on: Date,
    pub amount: Decimal,
    pub currency: String,
    pub description: Option<String>,
    pub receipt_number: Option<String>,
    pub confirm_duplicate: bool,
}
#[derive(Clone, Debug, Default)]
pub struct Approval {
    /// Absent means the full claimed amount.
    pub approved_amount: Option<Decimal>,
    pub decision_reason: Option<String>,
}
#[derive(Clone, Debug)]
pub struct Rejection {
    pub decision_reason: String,
}
#[derive(Clone, Debug, Default)]
pub struct Reimbursement {
    pub payment_reference: Option<String>,
}

/// Stored facts; timestamps are ISO 8601 UTC text, amounts exact decimals.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Request {
    pub id: i64,
    pub employee_id: i64,
    pub claim_type_id: i64,
    pub claim_policy_id: i64,
    #[serde(serialize_with = "crate::as_text")]
    pub incurred_on: Date,
    pub amount: Decimal,
    pub currency: String,
    pub description: Option<String>,
    pub receipt_number: Option<String>,
    pub status: String,
    pub duplicate_confirmed: bool,
    pub submitted_by_actor_id: i64,
    pub submitted_at: String,
    pub withdrawn_by_actor_id: Option<i64>,
    pub withdrawn_at: Option<String>,
    pub approved_amount: Option<Decimal>,
    pub decided_by_actor_id: Option<i64>,
    pub decided_at: Option<String>,
    pub decision_reason: Option<String>,
    pub reimbursed_by_actor_id: Option<i64>,
    pub reimbursed_at: Option<String>,
    pub payment_reference: Option<String>,
    pub handoff_batch_id: Option<i64>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RequestEvent {
    pub id: i64,
    pub from_status: Option<String>,
    pub to_status: String,
    pub actor_id: i64,
    pub reason: Option<String>,
    pub occurred_at: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct QueueClaim {
    #[serde(flatten)]
    pub request: Request,
    pub employee_number: Option<String>,
    pub employee_name: Option<String>,
    pub category_name: String,
    pub claim_type_code: String,
    pub claim_type_name: String,
}

pub(crate) const COLUMNS: &str = "r.id,r.employee_id,r.claim_type_id,r.claim_policy_id, \
 to_char(r.incurred_on,'YYYY-MM-DD') AS expense_date,r.amount,r.currency,r.description, \
 r.receipt_number,r.status,r.duplicate_confirmed,r.submitted_by_actor_id, \
 to_char(r.submitted_at,'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS submission_time, \
 r.withdrawn_by_actor_id,to_char(r.withdrawn_at,'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS withdrawal_time, \
 r.approved_amount,r.decided_by_actor_id,to_char(r.decided_at,'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS decision_time, \
 r.decision_reason,r.reimbursed_by_actor_id,to_char(r.reimbursed_at,'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS reimbursement_time, \
 r.payment_reference,r.handoff_batch_id";
pub(crate) fn read(row: &Row) -> bilibi_database::Result<Request> {
    Ok(Request {
        id: row.get("id"),
        employee_id: row.get("employee_id"),
        claim_type_id: row.get("claim_type_id"),
        claim_policy_id: row.get("claim_policy_id"),
        incurred_on: row
            .get::<_, String>("expense_date")
            .parse()
            .map_err(|_| Error::Internal)?,
        amount: row.get("amount"),
        currency: row.get("currency"),
        description: row.get("description"),
        receipt_number: row.get("receipt_number"),
        status: row.get("status"),
        duplicate_confirmed: row.get("duplicate_confirmed"),
        submitted_by_actor_id: row.get("submitted_by_actor_id"),
        submitted_at: row.get("submission_time"),
        withdrawn_by_actor_id: row.get("withdrawn_by_actor_id"),
        withdrawn_at: row.get("withdrawal_time"),
        approved_amount: row.get("approved_amount"),
        decided_by_actor_id: row.get("decided_by_actor_id"),
        decided_at: row.get("decision_time"),
        decision_reason: row.get("decision_reason"),
        reimbursed_by_actor_id: row.get("reimbursed_by_actor_id"),
        reimbursed_at: row.get("reimbursement_time"),
        payment_reference: row.get("payment_reference"),
        handoff_batch_id: row.get("handoff_batch_id"),
    })
}
pub(crate) async fn find(
    client: &impl GenericClient,
    tenant: i64,
    company: i64,
    id: i64,
    lock: bool,
) -> Result<Request> {
    let suffix = if lock { " FOR UPDATE" } else { "" };
    let row = client.query_opt(&format!("SELECT {COLUMNS} FROM people_claim_submissions r WHERE r.tenant_id=$1 AND r.company_id=$2 AND r.id=$3{suffix}"), &[&tenant,&company,&id]).await.map_err(Error::from)?.ok_or(ClaimsError::NotFound)?;
    Ok(read(&row)?)
}
pub(crate) async fn of_employee(
    client: &impl GenericClient,
    tenant: i64,
    company: i64,
    employee: i64,
) -> Result<Vec<Request>> {
    client.query(&format!("SELECT {COLUMNS} FROM people_claim_submissions r WHERE r.tenant_id=$1 AND r.company_id=$2 AND r.employee_id=$3 ORDER BY r.incurred_on DESC,r.id DESC"), &[&tenant,&company,&employee]).await.map_err(Error::from)?.iter().map(|r| read(r).map_err(Into::into)).collect()
}
pub(crate) fn optional_text(
    errors: &mut Vec<FieldError>,
    field: &'static str,
    value: Option<&str>,
    max: usize,
) -> Option<String> {
    let value = value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    if value.as_ref().is_some_and(|s| s.chars().count() > max) {
        errors.push(FieldError::new(
            field,
            format!("should be at most {max} character(s)"),
        ));
    }
    value
}
pub(crate) fn validate(input: &NewRequest) -> Result<NewRequest> {
    let mut input = input.clone();
    let mut errors = Vec::new();
    if input.claim_type_id <= 0 {
        errors.push(FieldError::new("claim_type_id", "can't be blank"));
    }
    input.currency =
        crate::text_field(&mut errors, "currency", &input.currency, 3).to_ascii_uppercase();
    if !input.currency.is_empty() && !crate::currency_shaped(&input.currency) {
        errors.push(FieldError::new("currency", "has invalid format"));
    }
    input.description = optional_text(
        &mut errors,
        "description",
        input.description.as_deref(),
        500,
    );
    let receipt = input.receipt_number.as_ref().map(|s| {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_uppercase()
    });
    input.receipt_number = optional_text(&mut errors, "receipt_number", receipt.as_deref(), 100);
    money::validate_amount(
        &mut errors,
        "amount",
        Some(input.amount),
        money::Min::Positive,
    );
    if errors.is_empty() {
        Ok(input)
    } else {
        Err(ClaimsError::Invalid(errors))
    }
}

pub(crate) async fn insert(
    tx: &mut Tx<'_>,
    scope: &bilibi_tenancy::Scope,
    company: i64,
    employee: i64,
    policy: i64,
    input: NewRequest,
    confirmed: bool,
) -> Result<Request> {
    let (tenant, actor) = (scope.tenant(), scope.user());
    Ok(tx.mutate(move |sql| Box::pin(async move {
        let row = sql.query_one(&format!("INSERT INTO people_claim_submissions AS r (tenant_id,company_id,employee_id,claim_type_id,claim_policy_id,incurred_on,amount,currency,description,receipt_number,status,duplicate_confirmed,submitted_by_actor_id,submitted_at) VALUES ($1,$2,$3,$4,$5,$6::text::date,$7,$8,$9,$10,'submitted',$11,$12,date_trunc('second',now() AT TIME ZONE 'UTC')) RETURNING {COLUMNS}"), &[&tenant,&company,&employee,&input.claim_type_id,&policy,&input.incurred_on.to_string(),&input.amount,&input.currency,&input.description,&input.receipt_number,&confirmed,&actor]).await?;
        let request = read(&row)?;
        let mut changes = vec![Change{kind:"people_claim_submission".into(),id:request.id,old:json!({}),new:json!(request)}];
        changes.push(history(sql,tenant,company,&request,None,actor).await?);
        Ok((request,changes))
    })).await?)
}

pub(crate) async fn transition(
    tx: &mut Tx<'_>,
    tenant: i64,
    company: i64,
    old: Request,
    new: Request,
    actor: i64,
) -> Result<Request> {
    Ok(tx.mutate(move |sql| Box::pin(async move {
        let row = sql.query_one(&format!("UPDATE people_claim_submissions AS r SET status=$4,withdrawn_by_actor_id=$5,withdrawn_at=CASE WHEN $4='withdrawn' THEN date_trunc('second',now() AT TIME ZONE 'UTC') ELSE withdrawn_at END,approved_amount=$6,decided_by_actor_id=$7,decided_at=CASE WHEN $4 IN ('approved','rejected') THEN date_trunc('second',now() AT TIME ZONE 'UTC') ELSE decided_at END,decision_reason=$8,reimbursed_by_actor_id=$9,reimbursed_at=CASE WHEN $4='reimbursed' THEN date_trunc('second',now() AT TIME ZONE 'UTC') ELSE reimbursed_at END,payment_reference=$10,updated_at=date_trunc('second',now() AT TIME ZONE 'UTC') WHERE r.tenant_id=$1 AND r.company_id=$2 AND r.id=$3 RETURNING {COLUMNS}"), &[&tenant,&company,&old.id,&new.status,&new.withdrawn_by_actor_id,&new.approved_amount,&new.decided_by_actor_id,&new.decision_reason,&new.reimbursed_by_actor_id,&new.payment_reference]).await?;
        let updated = read(&row)?;
        let mut changes = vec![Change{kind:"people_claim_submission".into(),id:updated.id,old:json!(old),new:json!(updated)}];
        changes.push(history(sql,tenant,company,&updated,Some(old.status),actor).await?);
        Ok((updated,changes))
    })).await?)
}
async fn history(
    sql: &impl GenericClient,
    tenant: i64,
    company: i64,
    request: &Request,
    from: Option<String>,
    actor: i64,
) -> bilibi_database::Result<Change> {
    let reason = if from.as_deref() == Some("submitted") && request.status != "withdrawn" {
        request.decision_reason.clone()
    } else {
        None
    };
    let row = sql.query_one("INSERT INTO people_claim_request_events (tenant_id,company_id,request_id,from_status,to_status,actor_id,reason,occurred_at) VALUES ($1,$2,$3,$4,$5,$6,$7,date_trunc('second',now() AT TIME ZONE 'UTC')) RETURNING id,to_char(occurred_at,'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS occurred_at", &[&tenant,&company,&request.id,&from,&request.status,&actor,&reason]).await?;
    Ok(Change {
        kind: "people_claim_request_event".into(),
        id: row.get("id"),
        old: json!({}),
        new: json!({"request_id":request.id,"from_status":from,"to_status":request.status,"actor_id":actor,"reason":reason,"occurred_at":row.get::<_,String>("occurred_at")}),
    })
}
