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
            Ok(json!(entry).into())
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
        key:"people.leave.entries", title:"Opening balances and adjustments",
        summary:"Review and record a leave balance change with a key that makes retries safe.",
        keywords:&["leave","opening","adjustment","balance","retry"], capability:POLICIES_MANAGE,
        body:include_str!("../entry-guide.md"),
    }],
}) }
