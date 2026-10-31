//! The page and agent share Bilimbi Learning's request commands.
use crate::*;
use bilibi_agent_api::{
    Channels, Definitions, Handler, Operation, OperationClient, OperationFuture, WriteHandler,
    DEFINITIONS,
};
use bilibi_database::{Error, MutationTransaction};
use bilibi_module_registry::{Contribution, Provenance};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::OnceLock};

static FACADE: OnceLock<Arc<Training>> = OnceLock::new();
pub(super) fn bind(facade: Arc<Training>) -> bilibi_module_registry::Result<()> {
    FACADE.set(facade).map_err(|_| {
        bilibi_module_registry::RegistryError::Service(
            "duplicate learning operation binding".into(),
        )
    })
}
fn facade() -> bilibi_agent_api::Result<&'static Training> {
    FACADE.get().map(Arc::as_ref).ok_or(Error::Internal.into())
}
fn refusal(e: TrainingError) -> bilibi_agent_api::Error {
    match e {
        TrainingError::Unauthorized => bilibi_agent_api::Error::Forbidden,
        TrainingError::NotFound => bilibi_agent_api::Error::NotFound,
        TrainingError::Failed(e) => e.into(),
        other => bilibi_agent_api::Error::InvalidInput(BTreeMap::from([(
            "request".into(),
            vec![other.to_string()],
        )])),
    }
}
pub(super) fn page_error(e: bilibi_agent_api::Error) -> TrainingError {
    match e {
        bilibi_agent_api::Error::Forbidden => TrainingError::Unauthorized,
        bilibi_agent_api::Error::NotFound => TrainingError::NotFound,
        bilibi_agent_api::Error::InvalidInput(fields) => {
            for candidate in [
                TrainingError::CurrencyUnavailable,
                TrainingError::BudgetUnavailable,
                TrainingError::BudgetExceeded,
                TrainingError::OutsideTeam,
                TrainingError::EmployeeUnavailable,
                TrainingError::SelfApproval,
                TrainingError::NotOwner,
                TrainingError::ReasonRequired,
                TrainingError::InvalidTransition,
                TrainingError::InvalidLearningValues,
                TrainingError::CompanyUnavailable,
                TrainingError::CourseUnavailable,
            ] {
                if fields
                    .values()
                    .flatten()
                    .any(|message| *message == candidate.to_string())
                {
                    return candidate;
                }
            }
            TrainingError::InvalidLearningValues
        }
        _ => TrainingError::InvalidLearningValues,
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    company_id: i64,
    request: Option<NewLearningRequest>,
    request_id: Option<i64>,
    reason: Option<String>,
}
struct Write {
    action: &'static str,
    capability: &'static str,
}
pub(super) fn decision_key(action: &str, capability: &str) -> &'static str {
    match (action, capability) {
        ("submit", _) => "people.training.request.submit",
        ("cancel", _) => "people.training.request.cancel",
        ("recommend", _) => "people.training.request.recommend",
        ("review", _) => "people.training.request.review",
        ("approve", _) => "people.training.request.approve",
        ("reject", REQUESTS_RECOMMEND) => "people.training.request.reject_recommendation",
        ("reject", REQUESTS_REVIEW) => "people.training.request.reject_review",
        _ => "people.training.request.reject_approval",
    }
}
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
            authorization::authorize_company_proof(
                client,
                scope,
                input.company_id,
                self.capability,
            )
            .await
            .map_err(TrainingError::from)
            .map_err(refusal)?;
            let training = facade()?;
            let facts = if self.action == "create" {
                let request = input.request.as_ref().ok_or(Error::Invalid)?;
                let employee = training
                    .workforce
                    .authorize_self(scope, input.company_id, REQUESTS_SUBMIT)
                    .await
                    .map_err(TrainingError::from)
                    .map_err(refusal)?;
                json!({"employee_id":employee.employee_id,"request":request})
            } else {
                let audience = match self.capability {
                    REQUESTS_SUBMIT => LearningAudience::SelfService,
                    REQUESTS_RECOMMEND => LearningAudience::Team,
                    _ => LearningAudience::Hr,
                };
                let row = training
                    .learning_request(
                        scope,
                        input.company_id,
                        input.request_id.ok_or(Error::Invalid)?,
                        audience,
                    )
                    .await
                    .map_err(refusal)?;
                let capability =
                    crate::governance::request_step(&row.status, self.action).map_err(refusal)?;
                if capability != self.capability {
                    return Err(bilibi_agent_api::Error::Forbidden);
                }
                if input
                    .reason
                    .as_deref()
                    .is_none_or(|r| r.trim().is_empty() || r.len() > 4000)
                {
                    return Err(Error::Invalid.into());
                }
                json!(row)
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
            authorization::authorize(tx.sql(), scope, input.company_id, self.capability)
                .await
                .map_err(TrainingError::from)
                .map_err(refusal)?;
            let training = facade()?;
            let result = if self.action == "create" {
                training
                    .create_learning_request_in(
                        tx,
                        scope,
                        input.company_id,
                        input.request.as_ref().ok_or(Error::Invalid)?,
                    )
                    .await
            } else {
                training
                    .decide_learning_request_in(
                        tx,
                        scope,
                        input.company_id,
                        input.request_id.ok_or(Error::Invalid)?,
                        self.action,
                        input.reason.as_deref().ok_or(Error::Invalid)?,
                    )
                    .await
            }
            .map_err(refusal)?;
            Ok(json!(result).into())
        })
    }
}
fn schema(create: bool) -> Value {
    json!({"type":"object","required":if create {vec!["company_id","request"]}else{vec!["company_id","request_id","reason"]},"properties":{
        "company_id":{"type":"integer","minimum":1},"request_id":{"type":"integer","minimum":1},"reason":{"type":"string","maxLength":4000},
        "request":{"type":"object","required":["need","objective","expected_result","proposed_on","estimated_cost","currency"],"properties":{
            "course_id":{"type":"integer"},"need":{"type":"string"},"objective":{"type":"string"},"expected_result":{"type":"string"},"proposed_on":{"type":"string","format":"date"},"estimated_cost":{"type":"string"},"currency":{"type":"string"}
        }}
    }})
}
macro_rules! operation { ($key:literal,$title:literal,$action:literal,$cap:ident) => {
    Operation {key:$key,title:$title,summary:$title,keywords:&["learning","request"],capability:$cap,input:||schema($action=="create"),output:||json!({"type":"object"}),handler:Handler::SafeWrite(&Write{action:$action,capability:$cap}),channels:Channels::Every,step_up:&[]}
}; }
inventory::submit! { Contribution::new(DEFINITIONS,"people/training.request_operations",Provenance {module:MODULE,source:file!(),line:line!()},&Definitions {operations:&[
    operation!("people.training.request.create","Create a learning request","create",REQUESTS_SUBMIT),
    operation!("people.training.request.submit","Submit a learning request","submit",REQUESTS_SUBMIT),
    operation!("people.training.request.cancel","Cancel a learning request","cancel",REQUESTS_SUBMIT),
    operation!("people.training.request.recommend","Recommend a learning request","recommend",REQUESTS_RECOMMEND),
    operation!("people.training.request.review","Review a learning request","review",REQUESTS_REVIEW),
    operation!("people.training.request.approve","Approve a learning request","approve",REQUESTS_APPROVE),
    operation!("people.training.request.reject_recommendation","Reject a learning recommendation","reject",REQUESTS_RECOMMEND),
    operation!("people.training.request.reject_review","Reject a learning review","reject",REQUESTS_REVIEW),
    operation!("people.training.request.reject_approval","Reject a learning approval","reject",REQUESTS_APPROVE),
],guides:&[]}) }
