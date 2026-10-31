//! Shared page and agent contracts for Bilimbi Claims workflows.
use crate::{
    Approval, Claims, ClaimsError, NewRequest, Reimbursement, Rejection, APPROVE, MODULE,
    REIMBURSE, SUBMIT,
};
use bilibi_agent_api::{
    Channels, Definitions, Handler, Operation, OperationClient, OperationFuture, WriteHandler,
    DEFINITIONS,
};
use bilibi_database::{Error, MutationTransaction};
use bilibi_module_registry::{Contribution, Provenance};
use bilibi_tenancy::Scope;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};

static FACADE: OnceLock<Arc<Claims>> = OnceLock::new();
pub(super) fn bind(facade: Arc<Claims>) -> bilibi_module_registry::Result<()> {
    FACADE.set(facade).map_err(|_| {
        bilibi_module_registry::RegistryError::Service("duplicate Claims operation binding".into())
    })
}
fn facade() -> bilibi_agent_api::Result<&'static Claims> {
    FACADE.get().map(Arc::as_ref).ok_or(Error::Internal.into())
}
pub(super) fn refusal(error: ClaimsError) -> bilibi_agent_api::Error {
    match error {
        ClaimsError::Unauthorized => bilibi_agent_api::Error::Forbidden,
        ClaimsError::NotFound => bilibi_agent_api::Error::NotFound,
        ClaimsError::Failed(e) => e.into(),
        ClaimsError::Invalid(errors) => {
            let mut fields: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for e in errors {
                fields
                    .entry(e.field.to_string())
                    .or_default()
                    .push(e.message);
            }
            bilibi_agent_api::Error::InvalidInput(fields)
        }
        other => bilibi_agent_api::Error::InvalidInput(BTreeMap::from([(
            "claim".into(),
            vec![other.to_string()],
        )])),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    company_id: i64,
    request_id: Option<i64>,
    batch_id: Option<i64>,
    claim_type_id: Option<i64>,
    incurred_on: Option<String>,
    amount: Option<String>,
    currency: Option<String>,
    description: Option<String>,
    receipt_number: Option<String>,
    #[serde(default)]
    confirm_duplicate: bool,
    approved_amount: Option<String>,
    decision_reason: Option<String>,
    payment_reference: Option<String>,
}
impl Input {
    fn request(&self) -> bilibi_agent_api::Result<NewRequest> {
        let input = NewRequest {
            claim_type_id: self.claim_type_id.ok_or(Error::Invalid)?,
            incurred_on: self
                .incurred_on
                .as_deref()
                .ok_or(Error::Invalid)?
                .parse()
                .map_err(|_| Error::Invalid)?,
            amount: self
                .amount
                .as_deref()
                .ok_or(Error::Invalid)?
                .parse()
                .map_err(|_| Error::Invalid)?,
            currency: self.currency.clone().ok_or(Error::Invalid)?,
            description: self.description.clone(),
            receipt_number: self.receipt_number.clone(),
            confirm_duplicate: self.confirm_duplicate,
        };
        crate::requests::validate(&input).map_err(refusal)
    }
    fn id(&self) -> bilibi_agent_api::Result<i64> {
        self.request_id
            .filter(|id| *id > 0)
            .ok_or(Error::Invalid.into())
    }
    fn batch(&self) -> bilibi_agent_api::Result<i64> {
        self.batch_id
            .filter(|id| *id > 0)
            .ok_or(Error::Invalid.into())
    }
    fn payment(&self) -> Reimbursement {
        Reimbursement {
            payment_reference: self.payment_reference.clone(),
        }
    }
    fn approval(&self) -> bilibi_agent_api::Result<Approval> {
        Ok(Approval {
            approved_amount: self
                .approved_amount
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .map(str::parse)
                .transpose()
                .map_err(|_| Error::Invalid)?,
            decision_reason: self.decision_reason.clone(),
        })
    }
}
#[derive(Clone, Copy)]
enum Action {
    Submit,
    Withdraw,
    Approve,
    Reject,
    Reimburse,
    Handoff,
    PayBatch,
}
impl Action {
    fn capability(self) -> &'static str {
        match self {
            Self::Submit | Self::Withdraw => SUBMIT,
            Self::Approve | Self::Reject => APPROVE,
            _ => REIMBURSE,
        }
    }
}
struct Write(Action);
impl WriteHandler for Write {
    fn owner(&self) -> &'static str {
        MODULE
    }
    fn preview<'a>(
        &'a self,
        client: &'a dyn OperationClient,
        scope: &'a Scope,
        value: &'a Value,
    ) -> OperationFuture<'a> {
        Box::pin(async move {
            let input: Input = serde_json::from_value(value.clone()).map_err(|_| Error::Invalid)?;
            let claims = facade()?;
            // Use the supplied client for company reach and grant decisions.
            bilibi_people_workforce::authorization::authorize_company_proof(
                client,
                scope,
                input.company_id,
                self.0.capability(),
            )
            .await
            .map_err(ClaimsError::from)
            .map_err(refusal)?;
            let facts = match self.0 {
                Action::Submit => {
                    let request = input.request()?;
                    let me = claims
                        .self_service_employee(scope, input.company_id)
                        .await
                        .map_err(refusal)?;
                    let types = claims
                        .self_open_claim_types(scope, input.company_id)
                        .await
                        .map_err(refusal)?;
                    if !types
                        .iter()
                        .any(|t| t.claim_type.id == request.claim_type_id)
                    {
                        return Err(refusal(ClaimsError::ClaimTypeUnavailable));
                    }
                    json!({"employee":me.id,"claim_type_id":request.claim_type_id,"incurred_on":request.incurred_on.to_string(),"amount":request.amount.to_string(),"currency":request.currency})
                }
                Action::Handoff => {
                    let currency = input.currency.as_deref().ok_or(Error::Invalid)?;
                    let waiting = claims
                        .handoff_waiting(scope, input.company_id)
                        .await
                        .map_err(refusal)?;
                    json!(waiting
                        .into_iter()
                        .find(|w| w.currency == currency)
                        .ok_or_else(|| refusal(ClaimsError::NothingToHandOff))?)
                }
                Action::PayBatch => {
                    let batches = claims
                        .handoff_batches(scope, input.company_id)
                        .await
                        .map_err(refusal)?;
                    json!(batches
                        .into_iter()
                        .find(|b| Some(b.id) == input.batch_id)
                        .ok_or(Error::NotFound)?)
                }
                _ => {
                    let team = !matches!(self.0, Action::Withdraw);
                    let rows = claims
                        .requests(scope, input.company_id, team, "", None, Some(input.id()?))
                        .await
                        .map_err(refusal)?;
                    let row = rows.first().ok_or(Error::NotFound)?;
                    let expected = if matches!(self.0, Action::Reimburse) {
                        "approved"
                    } else {
                        "submitted"
                    };
                    if row.request.status != expected {
                        return Err(refusal(ClaimsError::NotDecidable));
                    }
                    match self.0 {
                        Action::Approve => {
                            input.approval()?;
                        }
                        Action::Reject
                            if input
                                .decision_reason
                                .as_deref()
                                .is_none_or(|s| s.trim().is_empty()) =>
                        {
                            return Err(Error::Invalid.into())
                        }
                        _ => {}
                    }
                    json!(row)
                }
            };
            Ok(json!({"company_id":input.company_id,"record":facts,"input":value}).into())
        })
    }
    fn commit<'a>(
        &'a self,
        tx: &'a mut MutationTransaction<'_>,
        scope: &'a Scope,
        value: &'a Value,
    ) -> OperationFuture<'a> {
        Box::pin(async move {
            let input: Input = serde_json::from_value(value.clone()).map_err(|_| Error::Invalid)?;
            let claims = facade()?;
            let output = match self.0 {
                Action::Submit => json!(claims
                    .submit_request_in(tx, scope, input.company_id, &input.request()?)
                    .await
                    .map_err(refusal)?),
                Action::Withdraw => json!(claims
                    .withdraw_request_in(tx, scope, input.company_id, input.id()?)
                    .await
                    .map_err(refusal)?),
                Action::Approve => json!(claims
                    .decide_in(
                        tx,
                        scope,
                        input.company_id,
                        input.id()?,
                        crate::workflow::Decision::Approve(&input.approval()?)
                    )
                    .await
                    .map_err(refusal)?),
                Action::Reject => json!(claims
                    .decide_in(
                        tx,
                        scope,
                        input.company_id,
                        input.id()?,
                        crate::workflow::Decision::Reject(&Rejection {
                            decision_reason: input.decision_reason.clone().unwrap_or_default()
                        })
                    )
                    .await
                    .map_err(refusal)?),
                Action::Reimburse => json!(claims
                    .decide_in(
                        tx,
                        scope,
                        input.company_id,
                        input.id()?,
                        crate::workflow::Decision::Reimburse(&input.payment())
                    )
                    .await
                    .map_err(refusal)?),
                Action::Handoff => json!(claims
                    .create_handoff_batch_in(
                        tx,
                        scope,
                        input.company_id,
                        input.currency.as_deref().ok_or(Error::Invalid)?
                    )
                    .await
                    .map_err(refusal)?),
                Action::PayBatch => json!(claims
                    .reimburse_batch_in(
                        tx,
                        scope,
                        input.company_id,
                        input.batch()?,
                        &input.payment()
                    )
                    .await
                    .map_err(refusal)?),
            };
            Ok(output.into())
        })
    }
}
fn schema(required: &[&str]) -> Value {
    let mut fields = vec!["company_id"];
    fields.extend(required);
    json!({"type":"object","required":fields,"properties":{
        "company_id":{"type":"integer","minimum":1},"request_id":{"type":"integer","minimum":1},"batch_id":{"type":"integer","minimum":1},"claim_type_id":{"type":"integer","minimum":1},
        "incurred_on":{"type":"string","format":"date"},"amount":{"type":"string"},"currency":{"type":"string"},"description":{"type":"string","maxLength":500},"receipt_number":{"type":"string","maxLength":100},"confirm_duplicate":{"type":"boolean"},"approved_amount":{"type":"string"},"decision_reason":{"type":"string","maxLength":500},"payment_reference":{"type":"string","maxLength":100}
    }})
}
macro_rules! operation { ($key:literal,$title:literal,$action:ident,$cap:ident,[$($required:literal),*]) => {
    Operation {key:$key,title:$title,summary:$title,keywords:&["claim"],capability:$cap,input:||schema(&[$($required),*]),output:||json!({"type":["object","array"]}),handler:Handler::SafeWrite(&Write(Action::$action)),channels:Channels::Every,step_up:&[]}
}; }
inventory::submit! { Contribution::new(DEFINITIONS,"people/claims.agent",Provenance {module:MODULE,source:file!(),line:line!()},&Definitions { operations:&[
    operation!("people.claims.request.submit","Submit a claim",Submit,SUBMIT,["claim_type_id","incurred_on","amount","currency"]),
    operation!("people.claims.request.withdraw","Withdraw a claim",Withdraw,SUBMIT,["request_id"]),
    operation!("people.claims.request.approve","Approve a claim",Approve,APPROVE,["request_id"]),
    operation!("people.claims.request.reject","Reject a claim",Reject,APPROVE,["request_id","decision_reason"]),
    operation!("people.claims.request.reimburse","Mark a claim reimbursed",Reimburse,REIMBURSE,["request_id"]),
    operation!("people.claims.handoff.create","Create a claim hand-off batch",Handoff,REIMBURSE,["currency"]),
    operation!("people.claims.handoff.reimburse","Mark a claim batch reimbursed",PayBatch,REIMBURSE,["batch_id"]),
], guides:&[] }) }

pub(super) fn page_error(error: bilibi_agent_api::Error) -> ClaimsError {
    match error {
        bilibi_agent_api::Error::Forbidden => ClaimsError::Unauthorized,
        bilibi_agent_api::Error::NotFound => ClaimsError::NotFound,
        bilibi_agent_api::Error::InvalidInput(fields) => {
            for candidate in [
                ClaimsError::PossibleDuplicate,
                ClaimsError::NotLinked,
                ClaimsError::CompanyArchived,
                ClaimsError::ClaimTypeUnavailable,
                ClaimsError::ClaimTypeNotAssigned,
                ClaimsError::FutureIncurredOn,
                ClaimsError::NoEffectivePolicy,
                ClaimsError::CurrencyNotAllowed,
                ClaimsError::ReceiptRequired,
                ClaimsError::PerClaimLimitExceeded,
                ClaimsError::MonthlyLimitExceeded,
                ClaimsError::YearlyLimitExceeded,
                ClaimsError::DuplicateReceipt,
                ClaimsError::NotWithdrawable,
                ClaimsError::NotDecidable,
                ClaimsError::OwnClaim,
                ClaimsError::NothingToHandOff,
                ClaimsError::NothingToReimburse,
            ] {
                if fields
                    .values()
                    .flatten()
                    .any(|message| *message == candidate.to_string())
                {
                    return candidate;
                }
            }
            ClaimsError::Invalid(
                fields
                    .into_iter()
                    .flat_map(|(key, messages)| {
                        messages
                            .into_iter()
                            .map(move |m| crate::FieldError::new(field_name(&key), m))
                    })
                    .collect(),
            )
        }
        _ => ClaimsError::Invalid(vec![crate::FieldError::new(
            "claim",
            "The action could not be completed.",
        )]),
    }
}

fn field_name(key: &str) -> &'static str {
    match key {
        "claim_type_id" => "claim_type_id",
        "incurred_on" => "incurred_on",
        "amount" => "amount",
        "currency" => "currency",
        "receipt_number" => "receipt_number",
        "description" => "description",
        "approved_amount" => "approved_amount",
        "decision_reason" => "decision_reason",
        "payment_reference" => "payment_reference",
        _ => "claim",
    }
}
