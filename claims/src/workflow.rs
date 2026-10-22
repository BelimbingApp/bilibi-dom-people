//! Claims workflows, mirroring Bilimbi.People.Claims. The facade is the only door.
use crate::{
    authorization, catalog, company, money, policies, requests, writable, Approval, Claims,
    ClaimsError, FieldError, NewRequest, QueueClaim, Reimbursement, Rejection, Request,
    RequestEvent, Result, Tx, APPROVE, REIMBURSE, SUBMIT,
};
use bilibi_database::Error;
use bilibi_events::NewEvent;
use bilibi_tenancy::Scope;
use jiff::civil::Date;
use serde_json::json;
use tokio_postgres::GenericClient;

pub const REQUEST_APPROVED: &str = "people/claims.request_approved";
pub const REQUEST_REIMBURSED: &str = "people/claims.request_reimbursed";

impl Claims {
    /// Self-service: resolve the signed-in employee on every read.
    pub async fn self_requests(&self, scope: &Scope, company_id: i64) -> Result<Vec<Request>> {
        let me = self
            .workforce
            .authorize_self(scope, company_id, SUBMIT)
            .await?;
        let client = self.db.get().await.map_err(Error::from)?;
        requests::of_employee(&**client, scope.tenant(), company_id, me.employee_id).await
    }
    /// Other employees' claims require APPROVE; any employment status is visible.
    pub async fn employee_requests(
        &self,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
    ) -> Result<Vec<Request>> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, APPROVE).await?;
        let employees = self
            .workforce
            .employees_at_any_status(scope, company_id, Some(&[employee_id]))
            .await?;
        if employees.value.unwrap_or_default().is_empty() {
            return Err(ClaimsError::NotFound);
        }
        requests::of_employee(&**client, scope.tenant(), company_id, employee_id).await
    }
    pub async fn request_events(
        &self,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
        request_id: i64,
    ) -> Result<Vec<RequestEvent>> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, APPROVE).await?;
        self.history_of(&**client, scope, company_id, employee_id, request_id)
            .await
    }
    pub async fn self_request_events(
        &self,
        scope: &Scope,
        company_id: i64,
        request_id: i64,
    ) -> Result<Vec<RequestEvent>> {
        let me = self
            .workforce
            .authorize_self(scope, company_id, SUBMIT)
            .await?;
        let client = self.db.get().await.map_err(Error::from)?;
        self.history_of(&**client, scope, company_id, me.employee_id, request_id)
            .await
    }
    async fn history_of(
        &self,
        client: &impl GenericClient,
        scope: &Scope,
        company: i64,
        employee: i64,
        id: i64,
    ) -> Result<Vec<RequestEvent>> {
        let request = requests::find(client, scope.tenant(), company, id, false).await?;
        if request.employee_id != employee {
            return Err(ClaimsError::NotFound);
        }
        Ok(client.query("SELECT id,from_status,to_status,actor_id,reason,to_char(occurred_at,'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS occurred_at FROM people_claim_request_events WHERE tenant_id=$1 AND company_id=$2 AND request_id=$3 ORDER BY id", &[&scope.tenant(),&company,&id]).await.map_err(Error::from)?.iter().map(|r| RequestEvent{id:r.get("id"),from_status:r.get("from_status"),to_status:r.get("to_status"),actor_id:r.get("actor_id"),reason:r.get("reason"),occurred_at:r.get("occurred_at")}).collect())
    }
    /// Submit under Company → Employment locks, including the account-link reproof.
    pub async fn submit_request(
        &self,
        scope: &Scope,
        company_id: i64,
        input: &NewRequest,
    ) -> Result<Request> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, SUBMIT).await?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.claims.submit_request"),
            )
            .await?;
        writable(tx.sql(), scope, company_id).await?;
        let me = self
            .workforce
            .lock_self_employee(&tx, scope, company_id, SUBMIT)
            .await?;
        let input = requests::validate(input)?;
        let claim_type =
            catalog::lock_claim_type(tx.sql(), scope.tenant(), company_id, input.claim_type_id)
                .await?
                .ok_or(ClaimsError::ClaimTypeUnavailable)?;
        let category =
            catalog::find_category(tx.sql(), scope.tenant(), company_id, claim_type.category_id)
                .await?
                .ok_or(ClaimsError::ClaimTypeUnavailable)?;
        if !claim_type.active || !category.active {
            return Err(ClaimsError::ClaimTypeUnavailable);
        }
        if claim_type.eligibility == "assigned_only"
            && !crate::assignments::assigned_type_ids(
                tx.sql(),
                scope.tenant(),
                company_id,
                me.employee_id,
                input.incurred_on,
            )
            .await?
            .contains(&claim_type.id)
        {
            return Err(ClaimsError::ClaimTypeNotAssigned);
        }
        if input.incurred_on > self.company_today(tx.sql(), scope, company_id).await? {
            return Err(ClaimsError::FutureIncurredOn);
        }
        let policy = policies::list(tx.sql(), scope.tenant(), company_id)
            .await?
            .into_iter()
            .find(|p| {
                p.claim_type_id == claim_type.id
                    && p.effective_from <= input.incurred_on
                    && p.effective_to.is_none_or(|to| to >= input.incurred_on)
            })
            .ok_or(ClaimsError::NoEffectivePolicy)?;
        if input.currency != policy.currency
            || !self
                .company_currencies(tx.sql(), scope, company_id)
                .await?
                .contains(&input.currency)
        {
            return Err(ClaimsError::CurrencyNotAllowed);
        }
        let receipt_required = claim_type.receipt_requirement == "always"
            || (claim_type.receipt_requirement == "above_threshold"
                && policy
                    .receipt_threshold
                    .is_some_and(|threshold| input.amount > threshold));
        if receipt_required && input.receipt_number.is_none() {
            return Err(ClaimsError::ReceiptRequired);
        }
        submission_limits(tx.sql(), scope, company_id, me.employee_id, &input, &policy).await?;
        let confirmed = duplicates(tx.sql(), scope, company_id, me.employee_id, &input).await?;
        let saved = requests::insert(
            &mut tx,
            scope,
            company_id,
            me.employee_id,
            policy.id,
            input,
            confirmed,
        )
        .await?;
        tx.commit().await?;
        Ok(saved)
    }
    pub async fn withdraw_request(
        &self,
        scope: &Scope,
        company_id: i64,
        request_id: i64,
    ) -> Result<Request> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, SUBMIT).await?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.claims.withdraw_request"),
            )
            .await?;
        writable(tx.sql(), scope, company_id).await?;
        let me = self
            .workforce
            .lock_self_employee(&tx, scope, company_id, SUBMIT)
            .await?;
        let old = requests::find(tx.sql(), scope.tenant(), company_id, request_id, true).await?;
        if old.employee_id != me.employee_id {
            return Err(ClaimsError::NotFound);
        }
        if old.status != "submitted" {
            return Err(ClaimsError::NotWithdrawable);
        }
        let mut new = old.clone();
        new.status = "withdrawn".into();
        new.withdrawn_by_actor_id = Some(scope.user());
        let saved =
            requests::transition(&mut tx, scope.tenant(), company_id, old, new, scope.user())
                .await?;
        tx.commit().await?;
        Ok(saved)
    }
    pub async fn approve_request(
        &self,
        scope: &Scope,
        company_id: i64,
        request_id: i64,
        input: &Approval,
    ) -> Result<Request> {
        self.decide(scope, company_id, request_id, Decision::Approve(input))
            .await
    }
    pub async fn reject_request(
        &self,
        scope: &Scope,
        company_id: i64,
        request_id: i64,
        input: &Rejection,
    ) -> Result<Request> {
        self.decide(scope, company_id, request_id, Decision::Reject(input))
            .await
    }
    pub async fn reimburse_request(
        &self,
        scope: &Scope,
        company_id: i64,
        request_id: i64,
        input: &Reimbursement,
    ) -> Result<Request> {
        self.decide(scope, company_id, request_id, Decision::Reimburse(input))
            .await
    }
    async fn decide(
        &self,
        scope: &Scope,
        company_id: i64,
        id: i64,
        decision: Decision<'_>,
    ) -> Result<Request> {
        let (capability, from, action) = match decision {
            Decision::Approve(_) => (APPROVE, "submitted", "people.claims.approve_request"),
            Decision::Reject(_) => (APPROVE, "submitted", "people.claims.reject_request"),
            Decision::Reimburse(_) => (REIMBURSE, "approved", "people.claims.reimburse_request"),
        };
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, capability).await?;
        let mut tx = self
            .mutations
            .begin(&mut client, scope.mutation_context(action))
            .await?;
        writable(tx.sql(), scope, company_id).await?;
        authorization::authorize(tx.sql(), scope, company_id, capability).await?;
        let unlocked = requests::find(tx.sql(), scope.tenant(), company_id, id, false).await?;
        if unlocked.status != from {
            return Err(ClaimsError::NotDecidable);
        }
        self.refuse_own(&tx, scope, company_id, &unlocked, capability)
            .await?;
        let old = requests::find(tx.sql(), scope.tenant(), company_id, id, true).await?;
        if old.status != from {
            return Err(ClaimsError::NotDecidable);
        }
        let new = decision.apply(&old, scope.user())?;
        let saved =
            requests::transition(&mut tx, scope.tenant(), company_id, old, new, scope.user())
                .await?;
        if saved.status != "rejected" {
            self.publish_claim(&mut tx, scope, company_id, &saved)
                .await?;
        }
        tx.commit().await?;
        Ok(saved)
    }
    pub(crate) async fn refuse_own(
        &self,
        tx: &Tx<'_>,
        scope: &Scope,
        company: i64,
        request: &Request,
        capability: &str,
    ) -> Result<()> {
        let linked = self
            .workforce
            .owns_employee(tx, scope, company, request.employee_id, capability)
            .await?;
        if request.submitted_by_actor_id == scope.user() || linked {
            return Err(ClaimsError::OwnClaim);
        }
        Ok(())
    }
    pub(crate) async fn publish_claim(
        &self,
        tx: &mut Tx<'_>,
        scope: &Scope,
        company: i64,
        request: &Request,
    ) -> Result<()> {
        let category=tx.sql().query_one("SELECT c.code FROM people_claim_catalog_types t JOIN people_claim_catalog_groups c ON c.tenant_id=t.tenant_id AND c.company_id=t.company_id AND c.id=t.category_id WHERE t.tenant_id=$1 AND t.company_id=$2 AND t.id=$3", &[&scope.tenant(),&company,&request.claim_type_id]).await.map_err(Error::from)?.get::<_,String>(0);
        let name = if request.status == "approved" {
            REQUEST_APPROVED
        } else {
            REQUEST_REIMBURSED
        };
        let mut payload = json!({"company_id":company,"claim_id":request.id,"reference":format!("CLM-{}",request.id),"currency":request.currency,"approved_amount":money::text(request.approved_amount),"approval_date":request.decided_at.as_deref().map(|s|&s[..10]),"category_code":category});
        if request.status == "reimbursed" {
            payload["reimbursed_at"] = json!(request.reimbursed_at);
            payload["payment_reference"] = json!(request.payment_reference);
        }
        self.events
            .record(
                tx,
                NewEvent {
                    name,
                    subject: &format!("claim:{}", request.id),
                    key: &format!("claim:{}:{}", request.id, request.status),
                    payload,
                },
            )
            .await?;
        Ok(())
    }
    /// Four operator queues; submitted oldest first, the others newest first (≤500).
    pub async fn claim_queue(
        &self,
        scope: &Scope,
        company_id: i64,
        status: &str,
    ) -> Result<Vec<QueueClaim>> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, APPROVE).await?;
        if !["submitted", "approved", "rejected", "reimbursed"].contains(&status) {
            return Err(ClaimsError::Invalid(vec![FieldError::new(
                "status",
                "is invalid",
            )]));
        }
        let order = if status == "submitted" { "ASC" } else { "DESC" };
        let rows=client.query(&format!("SELECT {} FROM people_claim_submissions r WHERE r.tenant_id=$1 AND r.company_id=$2 AND r.status=$3 ORDER BY r.id {order} LIMIT 500",requests::COLUMNS), &[&scope.tenant(),&company_id,&status]).await.map_err(Error::from)?;
        let requests = rows
            .iter()
            .map(requests::read)
            .collect::<bilibi_database::Result<Vec<_>>>()?;
        self.decorate(scope, company_id, requests).await
    }
    pub(crate) async fn decorate(
        &self,
        scope: &Scope,
        company: i64,
        requests: Vec<Request>,
    ) -> Result<Vec<QueueClaim>> {
        let ids: Vec<_> = requests
            .iter()
            .map(|r| r.employee_id)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut employees = Vec::new();
        for page in ids.chunks(crate::MEMBER_LIMIT) {
            employees.extend(
                self.workforce
                    .employees_at_any_status(scope, company, Some(page))
                    .await?
                    .require_current()
                    .map_err(|_| ClaimsError::EmployeeNotFound)?,
            );
        }
        let client = self.db.get().await.map_err(Error::from)?;
        let types = catalog::claim_types(&**client, scope.tenant(), company).await?;
        let categories = catalog::categories(&**client, scope.tenant(), company).await?;
        requests
            .into_iter()
            .map(|request| {
                let employee = employees
                    .iter()
                    .find(|e| e.reference.stable_id == request.employee_id.to_string());
                let kind = types
                    .iter()
                    .find(|t| t.id == request.claim_type_id)
                    .ok_or(ClaimsError::ClaimTypeNotFound)?;
                let category = categories
                    .iter()
                    .find(|c| c.id == kind.category_id)
                    .ok_or(ClaimsError::CategoryNotFound)?;
                Ok(QueueClaim {
                    employee_number: employee.map(|e| e.employee_number.clone()),
                    employee_name: employee.map(|e| e.display_name.clone()),
                    category_name: category.name.clone(),
                    claim_type_code: kind.code.clone(),
                    claim_type_name: kind.name.clone(),
                    request,
                })
            })
            .collect()
    }
    /// Payroll configuration read: live source types incurred in the inclusive period.
    pub async fn requested_claim_type_ids(
        &self,
        scope: &Scope,
        company_id: i64,
        from: Date,
        to: Date,
    ) -> Result<Vec<i64>> {
        let client = self.db.get().await.map_err(Error::from)?;
        company(&**client, scope, company_id).await?;
        Ok(client.query("SELECT DISTINCT claim_type_id FROM people_claim_submissions WHERE tenant_id=$1 AND company_id=$2 AND status NOT IN ('withdrawn','rejected') AND incurred_on BETWEEN $3::text::date AND $4::text::date ORDER BY claim_type_id", &[&scope.tenant(),&company_id,&from.to_string(),&to.to_string()]).await.map_err(Error::from)?.iter().map(|r|r.get(0)).collect())
    }
}

enum Decision<'a> {
    Approve(&'a Approval),
    Reject(&'a Rejection),
    Reimburse(&'a Reimbursement),
}
impl Decision<'_> {
    fn apply(self, old: &Request, actor: i64) -> Result<Request> {
        let mut new = old.clone();
        let mut errors = Vec::new();
        match self {
            Self::Approve(input) => {
                let amount = input.approved_amount.unwrap_or(old.amount);
                money::validate_amount(
                    &mut errors,
                    "approved_amount",
                    Some(amount),
                    money::Min::Positive,
                );
                let reason = requests::optional_text(
                    &mut errors,
                    "decision_reason",
                    input.decision_reason.as_deref(),
                    500,
                );
                if amount > old.amount {
                    errors.push(FieldError::new(
                        "approved_amount",
                        "must not exceed the claimed amount",
                    ));
                }
                if amount < old.amount && reason.is_none() {
                    errors.push(FieldError::new(
                        "decision_reason",
                        "is required when approving less than claimed",
                    ));
                }
                new.status = "approved".into();
                new.approved_amount = Some(amount);
                new.decision_reason = reason;
                new.decided_by_actor_id = Some(actor);
            }
            Self::Reject(input) => {
                let reason = requests::optional_text(
                    &mut errors,
                    "decision_reason",
                    Some(&input.decision_reason),
                    500,
                );
                if reason.is_none() {
                    errors.push(FieldError::new("decision_reason", "can't be blank"));
                }
                new.status = "rejected".into();
                new.decision_reason = reason;
                new.decided_by_actor_id = Some(actor);
            }
            Self::Reimburse(input) => {
                new.payment_reference = requests::optional_text(
                    &mut errors,
                    "payment_reference",
                    input.payment_reference.as_deref(),
                    100,
                );
                new.status = "reimbursed".into();
                new.reimbursed_by_actor_id = Some(actor);
            }
        }
        if errors.is_empty() {
            Ok(new)
        } else {
            Err(ClaimsError::Invalid(errors))
        }
    }
}
pub(crate) fn reimbursement(old: &Request, actor: i64, input: &Reimbursement) -> Result<Request> {
    Decision::Reimburse(input).apply(old, actor)
}

async fn submission_limits(
    client: &impl GenericClient,
    scope: &Scope,
    company: i64,
    employee: i64,
    input: &NewRequest,
    policy: &crate::Policy,
) -> Result<()> {
    if policy
        .per_claim_limit
        .is_some_and(|limit| input.amount > limit)
    {
        return Err(ClaimsError::PerClaimLimitExceeded);
    }
    // Compare in SQL NUMERIC: a sum can exceed rust_decimal's range even
    // though every individual stored value fits. Approved amounts replace claimed.
    let row=client.query_one("SELECT COALESCE(sum(COALESCE(approved_amount,amount)) FILTER (WHERE date_trunc('month',incurred_on)=date_trunc('month',$6::text::date)),0)+$7 > $8 AS month_exceeded,COALESCE(sum(COALESCE(approved_amount,amount)),0)+$7 > $9 AS year_exceeded FROM people_claim_submissions WHERE tenant_id=$1 AND company_id=$2 AND employee_id=$3 AND claim_type_id=$4 AND currency=$5 AND status NOT IN ('withdrawn','rejected') AND date_trunc('year',incurred_on)=date_trunc('year',$6::text::date)", &[&scope.tenant(),&company,&employee,&input.claim_type_id,&input.currency,&input.incurred_on.to_string(),&input.amount,&policy.monthly_limit,&policy.yearly_limit]).await.map_err(Error::from)?;
    if row.get::<_, Option<bool>>("month_exceeded") == Some(true) {
        return Err(ClaimsError::MonthlyLimitExceeded);
    }
    if row.get::<_, Option<bool>>("year_exceeded") == Some(true) {
        return Err(ClaimsError::YearlyLimitExceeded);
    }
    Ok(())
}
async fn duplicates(
    client: &impl GenericClient,
    scope: &Scope,
    company: i64,
    employee: i64,
    input: &NewRequest,
) -> Result<bool> {
    let row=client.query_one("SELECT EXISTS (SELECT 1 FROM people_claim_submissions WHERE tenant_id=$1 AND company_id=$2 AND employee_id=$3 AND status NOT IN ('withdrawn','rejected') AND receipt_number=$4) AS receipt, EXISTS (SELECT 1 FROM people_claim_submissions WHERE tenant_id=$1 AND company_id=$2 AND employee_id=$3 AND status NOT IN ('withdrawn','rejected') AND claim_type_id=$5 AND currency=$6 AND incurred_on=$7::text::date AND amount=$8) AS shape", &[&scope.tenant(),&company,&employee,&input.receipt_number,&input.claim_type_id,&input.currency,&input.incurred_on.to_string(),&input.amount]).await.map_err(Error::from)?;
    if row.get::<_, bool>("receipt") {
        return Err(ClaimsError::DuplicateReceipt);
    }
    if row.get::<_, bool>("shape") {
        if input.confirm_duplicate {
            return Ok(true);
        }
        return Err(ClaimsError::PossibleDuplicate);
    }
    Ok(false)
}
