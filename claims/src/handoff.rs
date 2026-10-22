//! Per-currency hand-off and RFC 4180 export (Bilimbi Claims and Claims.Csv).
use crate::{
    authorization, requests, workflow, writable, Claims, ClaimsError, FieldError, QueueClaim,
    Reimbursement, Request, Result, APPROVE, REIMBURSE,
};
use bilibi_database::{Change, Error};
use bilibi_tenancy::Scope;
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::json;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HandoffBatch {
    pub id: i64,
    pub currency: String,
    pub request_count: i32,
    pub total_amount: Decimal,
    pub created_by_actor_id: i64,
    pub created_at: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HandoffWaiting {
    pub currency: String,
    pub request_count: i64,
    pub total_amount: Decimal,
}
#[derive(Clone, Debug)]
pub struct HandoffExport {
    pub filename: String,
    pub content: String,
}
const BATCH_COLUMNS:&str="id,currency,request_count,total_amount,created_by_actor_id,to_char(created_at,'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS created_at";
fn read_batch(row: &tokio_postgres::Row) -> HandoffBatch {
    HandoffBatch {
        id: row.get("id"),
        currency: row.get("currency"),
        request_count: row.get("request_count"),
        total_amount: row.get("total_amount"),
        created_by_actor_id: row.get("created_by_actor_id"),
        created_at: row.get("created_at"),
    }
}
impl Claims {
    pub async fn handoff_waiting(
        &self,
        scope: &Scope,
        company_id: i64,
    ) -> Result<Vec<HandoffWaiting>> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, APPROVE).await?;
        Ok(client.query("SELECT currency,count(*) AS count,sum(approved_amount) AS total FROM people_claim_submissions WHERE tenant_id=$1 AND company_id=$2 AND status='approved' AND handoff_batch_id IS NULL GROUP BY currency ORDER BY currency", &[&scope.tenant(),&company_id]).await.map_err(Error::from)?.iter().map(|r|HandoffWaiting{currency:r.get("currency"),request_count:r.get("count"),total_amount:r.get("total")}).collect())
    }
    /// Every approved, unbatched claim in exactly one currency. Company lock
    /// serializes hand-off, decisions and reimbursement, including empty queues.
    pub async fn create_handoff_batch(
        &self,
        scope: &Scope,
        company_id: i64,
        currency: &str,
    ) -> Result<HandoffBatch> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, REIMBURSE).await?;
        let currency = currency.trim().to_ascii_uppercase();
        if !crate::currency_shaped(&currency) {
            return Err(ClaimsError::InvalidCurrency);
        }
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.claims.create_handoff_batch"),
            )
            .await?;
        writable(tx.sql(), scope, company_id).await?;
        authorization::authorize(tx.sql(), scope, company_id, REIMBURSE).await?;
        let rows=tx.sql().query(&format!("SELECT {} FROM people_claim_submissions r WHERE r.tenant_id=$1 AND r.company_id=$2 AND r.status='approved' AND r.handoff_batch_id IS NULL AND r.currency=$3 ORDER BY r.id FOR UPDATE",requests::COLUMNS), &[&scope.tenant(),&company_id,&currency]).await.map_err(Error::from)?;
        let claims = rows
            .iter()
            .map(requests::read)
            .collect::<bilibi_database::Result<Vec<_>>>()?;
        if claims.is_empty() {
            return Err(ClaimsError::NothingToHandOff);
        }
        let total = claims
            .iter()
            .try_fold(Decimal::ZERO, |sum, r| {
                sum.checked_add(r.approved_amount.unwrap_or_default())
            })
            .ok_or_else(total_too_large)?;
        let mut errors = Vec::new();
        crate::money::validate_amount(
            &mut errors,
            "total_amount",
            Some(total),
            crate::money::Min::Positive,
        );
        if !errors.is_empty() {
            return Err(ClaimsError::Invalid(errors));
        }
        let count = i32::try_from(claims.len()).map_err(|_| total_too_large())?;
        let (tenant, actor) = (scope.tenant(), scope.user());
        let batch=tx.mutate(move |sql|Box::pin(async move {
            let row=sql.query_one(&format!("INSERT INTO people_claim_handoff_batches (tenant_id,company_id,currency,request_count,total_amount,created_by_actor_id,created_at) VALUES ($1,$2,$3,$4,$5,$6,date_trunc('second',now() AT TIME ZONE 'UTC')) RETURNING {BATCH_COLUMNS}"), &[&tenant,&company_id,&currency,&count,&total,&actor]).await?;
            let batch=read_batch(&row);
            let mut changes=vec![Change{kind:"people_claim_handoff_batch".into(),id:batch.id,old:json!({}),new:json!(batch)}];
            for old in claims {
                let row=sql.query_one(&format!("UPDATE people_claim_submissions AS r SET handoff_batch_id=$4,updated_at=date_trunc('second',now() AT TIME ZONE 'UTC') WHERE r.tenant_id=$1 AND r.company_id=$2 AND r.id=$3 RETURNING {}",requests::COLUMNS), &[&tenant,&company_id,&old.id,&batch.id]).await?;
                let updated=requests::read(&row)?;
                changes.push(Change{kind:"people_claim_submission".into(),id:old.id,old:json!(old),new:json!(updated)});
            }
            Ok((batch,changes))
        })).await?;
        tx.commit().await?;
        Ok(batch)
    }
    pub async fn handoff_batches(
        &self,
        scope: &Scope,
        company_id: i64,
    ) -> Result<Vec<HandoffBatch>> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, APPROVE).await?;
        Ok(client.query(&format!("SELECT {BATCH_COLUMNS} FROM people_claim_handoff_batches WHERE tenant_id=$1 AND company_id=$2 ORDER BY id DESC LIMIT 500"), &[&scope.tenant(),&company_id]).await.map_err(Error::from)?.iter().map(read_batch).collect())
    }
    /// Re-read stored facts and current statuses; owner reauthorizes each export.
    pub async fn handoff_export(
        &self,
        scope: &Scope,
        company_id: i64,
        batch_id: i64,
    ) -> Result<HandoffExport> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, REIMBURSE).await?;
        if client.query_opt("SELECT id FROM people_claim_handoff_batches WHERE tenant_id=$1 AND company_id=$2 AND id=$3", &[&scope.tenant(),&company_id,&batch_id]).await.map_err(Error::from)?.is_none() {return Err(ClaimsError::NotFound);}
        let rows=client.query(&format!("SELECT {} FROM people_claim_submissions r WHERE r.tenant_id=$1 AND r.company_id=$2 AND r.handoff_batch_id=$3 ORDER BY r.id",requests::COLUMNS), &[&scope.tenant(),&company_id,&batch_id]).await.map_err(Error::from)?;
        let requests = rows
            .iter()
            .map(requests::read)
            .collect::<bilibi_database::Result<Vec<_>>>()?;
        drop(client);
        let rows = self.decorate(scope, company_id, requests).await?;
        Ok(HandoffExport {
            filename: format!("claim-handoff-{batch_id}.csv"),
            content: csv(batch_id, &rows),
        })
    }
    /// Preflight every claim before mutating any; one own claim refuses the batch.
    pub async fn reimburse_batch(
        &self,
        scope: &Scope,
        company_id: i64,
        batch_id: i64,
        input: &Reimbursement,
    ) -> Result<Vec<Request>> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, REIMBURSE).await?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.claims.reimburse_batch"),
            )
            .await?;
        writable(tx.sql(), scope, company_id).await?;
        authorization::authorize(tx.sql(), scope, company_id, REIMBURSE).await?;
        if tx.sql().query_opt("SELECT id FROM people_claim_handoff_batches WHERE tenant_id=$1 AND company_id=$2 AND id=$3", &[&scope.tenant(),&company_id,&batch_id]).await.map_err(Error::from)?.is_none() {return Err(ClaimsError::NotFound);}
        let rows=tx.sql().query(&format!("SELECT {} FROM people_claim_submissions r WHERE r.tenant_id=$1 AND r.company_id=$2 AND r.handoff_batch_id=$3 AND r.status='approved' ORDER BY r.id",requests::COLUMNS), &[&scope.tenant(),&company_id,&batch_id]).await.map_err(Error::from)?;
        let claims = rows
            .iter()
            .map(requests::read)
            .collect::<bilibi_database::Result<Vec<_>>>()?;
        if claims.is_empty() {
            return Err(ClaimsError::NothingToReimburse);
        }
        let mut prepared = Vec::with_capacity(claims.len());
        // Prove every employment before taking any Claims row locks.
        for claim in &claims {
            self.refuse_own(&tx, scope, company_id, claim, REIMBURSE)
                .await?;
        }
        for claim in claims {
            let locked =
                requests::find(tx.sql(), scope.tenant(), company_id, claim.id, true).await?;
            let new = workflow::reimbursement(&locked, scope.user(), input)?;
            prepared.push((locked, new));
        }
        let mut done = Vec::with_capacity(prepared.len());
        for (old, new) in prepared {
            let saved =
                requests::transition(&mut tx, scope.tenant(), company_id, old, new, scope.user())
                    .await?;
            self.publish_claim(&mut tx, scope, company_id, &saved)
                .await?;
            done.push(saved);
        }
        tx.commit().await?;
        Ok(done)
    }
}
fn total_too_large() -> ClaimsError {
    ClaimsError::Invalid(vec![FieldError::new("total_amount", "is too large")])
}
const HEADERS:&str="batch_id,claim_id,employee_number,employee_name,category,claim_type_code,claim_type_name,incurred_on,currency,claimed_amount,approved_amount,receipt_number,description,status,approved_at,reimbursed_at,payment_reference\r\n";
fn csv(batch: i64, rows: &[QueueClaim]) -> String {
    let mut out = HEADERS.to_owned();
    for row in rows {
        let r = &row.request;
        let values = [
            batch.to_string(),
            r.id.to_string(),
            row.employee_number.clone().unwrap_or_default(),
            row.employee_name.clone().unwrap_or_default(),
            row.category_name.clone(),
            row.claim_type_code.clone(),
            row.claim_type_name.clone(),
            r.incurred_on.to_string(),
            r.currency.clone(),
            r.amount.to_string(),
            r.approved_amount.map(|v| v.to_string()).unwrap_or_default(),
            r.receipt_number.clone().unwrap_or_default(),
            r.description.clone().unwrap_or_default(),
            r.status.clone(),
            r.decided_at
                .as_deref()
                .unwrap_or_default()
                .trim_end_matches('Z')
                .to_owned(),
            r.reimbursed_at
                .as_deref()
                .unwrap_or_default()
                .trim_end_matches('Z')
                .to_owned(),
            r.payment_reference.clone().unwrap_or_default(),
        ];
        out.push_str(&values.iter().map(|s| cell(s)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    out
}
fn cell(value: &str) -> String {
    // Also neutralise formulas hidden after leading whitespace. Prefix before
    // RFC 4180 quoting, so a quote cannot turn the apostrophe into syntax.
    let dangerous = value.starts_with(['=', '+', '-', '@', '\t', '\r', '\n'])
        || value.trim_start().starts_with(['=', '+', '-', '@']);
    let value = if dangerous {
        format!("'{value}")
    } else {
        value.to_owned()
    };
    if value.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value
    }
}
