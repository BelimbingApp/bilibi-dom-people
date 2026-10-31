//! Reviewed adapter for Bilimbi Leave.record_entry/4 (no Bilimbi page).
use crate::{Leave, LeaveError, NewEntry, MODULE, POLICIES_MANAGE};
use bilibi_agent_api::{
    Channels, Definitions, Guide, Handler, Operation, OperationClient, OperationFuture,
    WriteHandler, DEFINITIONS,
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

static FACADE: OnceLock<Arc<Leave>> = OnceLock::new();
pub(super) fn bind(facade: Arc<Leave>) -> bilibi_module_registry::Result<()> {
    FACADE.set(facade).map_err(|_| {
        bilibi_module_registry::RegistryError::Service("duplicate Leave operation binding".into())
    })
}
fn facade() -> bilibi_agent_api::Result<&'static Leave> {
    FACADE.get().map(Arc::as_ref).ok_or(Error::Internal.into())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    company_id: i64,
    employee_id: i64,
    leave_type_id: i64,
    entry_type: String,
    quantity: String,
    occurred_on: String,
    source: String,
    entry_key: String,
    note: Option<String>,
}
fn parse(input: &Value) -> bilibi_agent_api::Result<(i64, i64, NewEntry)> {
    let input: Input = serde_json::from_value(input.clone()).map_err(|_| Error::Invalid)?;
    let quantity = rust_decimal::Decimal::from_str_exact(&input.quantity)
        .map_err(|_| refusal(LeaveError::InvalidEntry))?;
    let occurred_on = input
        .occurred_on
        .parse()
        .map_err(|_| refusal(LeaveError::InvalidEntry))?;
    Ok((
        input.company_id,
        input.employee_id,
        NewEntry {
            leave_type_id: input.leave_type_id,
            entry_type: input.entry_type,
            quantity,
            occurred_on,
            source: input.source,
            entry_key: input.entry_key,
            note: input.note,
        },
    ))
}
fn refusal(error: LeaveError) -> bilibi_agent_api::Error {
    match error {
        LeaveError::Unauthorized => bilibi_agent_api::Error::Forbidden,
        LeaveError::NotFound => bilibi_agent_api::Error::NotFound,
        LeaveError::Failed(error) => error.into(),
        other => bilibi_agent_api::Error::InvalidInput(BTreeMap::from([(
            "entry".into(),
            vec![other.to_string()],
        )])),
    }
}
struct Record;
impl WriteHandler for Record {
    fn owner(&self) -> &'static str {
        MODULE
    }
    fn preview<'a>(
        &'a self,
        client: &'a dyn OperationClient,
        scope: &'a Scope,
        input: &'a Value,
    ) -> OperationFuture<'a> {
        Box::pin(async move {
            let (company, employee, entry) = parse(input)?;
            Ok(facade()?
                .review_entry(client, scope, company, employee, &entry)
                .await
                .map_err(refusal)?
                .into())
        })
    }
    fn commit<'a>(
        &'a self,
        tx: &'a mut MutationTransaction<'_>,
        scope: &'a Scope,
        input: &'a Value,
    ) -> OperationFuture<'a> {
        Box::pin(async move {
            let (company, employee, entry) = parse(input)?;
            let entry = facade()?
                .record_entry_in(tx, scope, company, employee, &entry)
                .await
                .map_err(refusal)?;
            Ok(json!(entry.entry).into())
        })
    }
}
fn input_schema() -> Value {
    let id = json!({"type":"integer","minimum":1});
    json!({"type":"object","required":["company_id","employee_id","leave_type_id","entry_type","quantity","occurred_on","source","entry_key"],"properties":{
        "company_id":id,"employee_id":id,"leave_type_id":id,
        "entry_type":{"type":"string","enum":["opening","adjustment"]},
        "quantity":{"type":"string","maxLength":32,"description":"Exact non-zero decimal from -9999.99 to 9999.99, at most two decimal places, in the leave type's unit."},
        "occurred_on":{"type":"string","format":"date"},
        "source":{"type":"string","maxLength":32,"description":"1 to 32 UTF-8 bytes. policy, request and carry_forward are reserved."},
        "entry_key":{"type":"string","maxLength":160,"description":"1 to 160 UTF-8 bytes, unique within the company and source; reuse only for an identical retry."},
        "note":{"type":"string","maxLength":500}
    }})
}
inventory::submit! { Contribution::new(DEFINITIONS, "people/leave.agent", Provenance {module:MODULE,source:file!(),line:line!()}, &Definitions {
    operations:&[Operation {
        key:"people.leave.entry.record", title:"Record a leave balance entry",
        summary:"Record an opening balance or adjustment, or return an identical entry already recorded under the same source and key.",
        keywords:&["leave","balance","opening","adjustment","entry"], capability:POLICIES_MANAGE,
        input:input_schema, output:||json!({"type":"object","description":"LedgerEntry: id, employee_id, leave_type_id, leave_year, entry_type, exact quantity string, unit, nullable policy_version, occurred_on date, source, entry_key and nullable note."}),
        handler:Handler::SafeWrite(&Record), channels:Channels::Every, step_up:&[],
    }],
    guides:&[Guide {
        key:"people.leave.entry", title:"Opening balances and adjustments",
        summary:"Review and record a leave balance change with a key that makes retries safe.",
        keywords:&["leave","opening","adjustment","balance","retry"], capability:POLICIES_MANAGE,
        body:include_str!("../entry-guide.md"),
    }],
}) }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestInput {
    company_id: i64,
    request_id: Option<i64>,
    leave_type_id: Option<i64>,
    starts_on: Option<String>,
    ends_on: Option<String>,
    day_part: Option<String>,
    hours: Option<String>,
    reason: Option<String>,
    request_key: Option<String>,
    note: Option<String>,
}
impl RequestInput {
    fn new_request(&self) -> bilibi_agent_api::Result<crate::NewRequest> {
        crate::requests::normalize(&crate::NewRequest {
            leave_type_id: self.leave_type_id.ok_or(Error::Invalid)?,
            starts_on: self
                .starts_on
                .as_deref()
                .ok_or(Error::Invalid)?
                .parse()
                .map_err(|_| Error::Invalid)?,
            ends_on: self
                .ends_on
                .as_deref()
                .ok_or(Error::Invalid)?
                .parse()
                .map_err(|_| Error::Invalid)?,
            day_part: self.day_part.clone().unwrap_or_else(|| "full".into()),
            hours: self
                .hours
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(str::parse)
                .transpose()
                .map_err(|_| Error::Invalid)?,
            reason: self.reason.clone(),
            request_key: self.request_key.clone().ok_or(Error::Invalid)?,
        })
        .map_err(refusal)
    }
}
struct RequestWrite(&'static str);
impl WriteHandler for RequestWrite {
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
            let input: RequestInput =
                serde_json::from_value(value.clone()).map_err(|_| Error::Invalid)?;
            let cap = if matches!(self.0, "submit" | "cancel") {
                crate::SELF_VIEW
            } else {
                crate::REQUESTS_APPROVE
            };
            bilibi_people_workforce::authorization::authorize_company_proof(
                client,
                scope,
                input.company_id,
                cap,
            )
            .await
            .map_err(LeaveError::from)
            .map_err(refusal)?;
            let leave = facade()?;
            let record = if self.0 == "submit" {
                let request = input.new_request()?;
                let summary = leave
                    .self_summary(scope, input.company_id, None)
                    .await
                    .map_err(refusal)?;
                json!({"leave_year":summary.leave_year,"leave_type_id":request.leave_type_id,"starts_on":request.starts_on.to_string(),"ends_on":request.ends_on.to_string()})
            } else {
                let row = leave
                    .request(
                        scope,
                        input.company_id,
                        input.request_id.ok_or(Error::Invalid)?,
                    )
                    .await
                    .map_err(refusal)?;
                if self.0 != "cancel" && row.request.status != "pending" {
                    return Err(refusal(LeaveError::NotPending));
                }
                if self.0 == "reject" && input.note.as_deref().is_none_or(|s| s.trim().is_empty()) {
                    return Err(refusal(LeaveError::NoteRequired));
                }
                json!(row)
            };
            Ok(json!({"company_id":input.company_id,"record":record,"input":value}).into())
        })
    }
    fn commit<'a>(
        &'a self,
        tx: &'a mut MutationTransaction<'_>,
        scope: &'a Scope,
        value: &'a Value,
    ) -> OperationFuture<'a> {
        Box::pin(async move {
            let input: RequestInput =
                serde_json::from_value(value.clone()).map_err(|_| Error::Invalid)?;
            let leave = facade()?;
            let request = match self.0 {
                "submit" => leave
                    .submit_request_in(tx, scope, input.company_id, &input.new_request()?)
                    .await
                    .map(|request| request.request),
                "cancel" => {
                    leave
                        .cancel_request_in(
                            tx,
                            scope,
                            input.company_id,
                            input.request_id.ok_or(Error::Invalid)?,
                        )
                        .await
                }
                action => {
                    leave
                        .decide_request_in(
                            tx,
                            scope,
                            input.company_id,
                            input.request_id.ok_or(Error::Invalid)?,
                            if action == "approve" {
                                crate::Decision::Approve
                            } else {
                                crate::Decision::Reject
                            },
                            input.note.as_deref(),
                        )
                        .await
                }
            }
            .map_err(refusal)?;
            Ok(json!(request).into())
        })
    }
}
fn request_schema(submit: bool) -> Value {
    json!({"type":"object","required":if submit{vec!["company_id","leave_type_id","starts_on","ends_on","request_key"]}else{vec!["company_id","request_id"]},"properties":{
        "company_id":{"type":"integer","minimum":1},"request_id":{"type":"integer","minimum":1},"leave_type_id":{"type":"integer","minimum":1},
        "starts_on":{"type":"string","format":"date"},"ends_on":{"type":"string","format":"date"},"day_part":{"type":"string","enum":["full","am","pm","hours"]},"hours":{"type":"string"},"reason":{"type":"string","maxLength":500},"note":{"type":"string","maxLength":500},"request_key":{"type":"string","maxLength":160}
    }})
}
macro_rules! request_operation { ($key:literal,$title:literal,$action:literal,$cap:ident) => {
    Operation{key:$key,title:$title,summary:$title,keywords:&["leave","request"],capability:crate::$cap,input:||request_schema($action=="submit"),output:||json!({"type":"object"}),handler:Handler::SafeWrite(&RequestWrite($action)),channels:Channels::Every,step_up:&[]}
}; }
inventory::submit! { Contribution::new(DEFINITIONS,"people/leave.request_operations",Provenance{module:MODULE,source:file!(),line:line!()},&Definitions {operations:&[
    request_operation!("people.leave.request.submit","Submit a leave request","submit",SELF_VIEW),
    request_operation!("people.leave.request.cancel","Cancel a leave request","cancel",SELF_VIEW),
    request_operation!("people.leave.request.approve","Approve a leave request","approve",REQUESTS_APPROVE),
    request_operation!("people.leave.request.reject","Reject a leave request","reject",REQUESTS_APPROVE),
],guides:&[]}) }
pub(super) fn page_error(error: bilibi_agent_api::Error) -> LeaveError {
    match error {
        bilibi_agent_api::Error::Forbidden => LeaveError::Unauthorized,
        bilibi_agent_api::Error::NotFound => LeaveError::NotFound,
        bilibi_agent_api::Error::InvalidInput(fields) => {
            for candidate in [
                LeaveError::SelfApproval,
                LeaveError::NotPending,
                LeaveError::NotCancellable,
                LeaveError::NoteRequired,
                LeaveError::YearClosed,
                LeaveError::CompanyArchived,
                LeaveError::NotLinked,
                LeaveError::EmployeeUnavailable,
                LeaveError::LeaveTypeUnavailable,
            ] {
                if fields
                    .values()
                    .flatten()
                    .any(|s| *s == candidate.to_string())
                {
                    return candidate;
                }
            }
            LeaveError::InvalidRequest
        }
        _ => LeaveError::InvalidRequest,
    }
}
