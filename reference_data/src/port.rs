//! Employment type choices, from the ReferenceData facade on the caller's
//! connection, so Workforce's input picker adds no cyclic dependency.
use crate::ReferenceData;
use bilibi_database::Error;
use bilibi_people_workforce::WorkforceError;
use bilibi_ports::{on_client, PortClient, PortFuture, PortProvider, Value};
use bilibi_tenancy::Scope;
use serde_json::json;
pub(crate) struct EmploymentTypes;
impl PortProvider for EmploymentTypes {
    fn call<'a>(&'a self, client: PortClient<'a>, scope: &'a Scope, body: Value) -> PortFuture<'a> {
        Box::pin(async move {
            let fields = body.as_object().ok_or(Error::Invalid)?;
            if fields.len() != 1 {
                return Err(Error::Invalid);
            }
            let company = fields
                .get("company_id")
                .and_then(Value::as_i64)
                .filter(|id| *id > 0)
                .ok_or(Error::Invalid)?;
            let entries = on_client!(client, |c| ReferenceData::list_on(
                c,
                scope,
                company,
                "employment_type"
            )
            .await)
            .map_err(|e| match e {
                WorkforceError::Failed(e) => e,
                WorkforceError::NotFound => Error::NotFound,
                WorkforceError::Unauthorized => Error::Forbidden,
                _ => Error::Invalid,
            })?;
            Ok(json!(entries
                .into_iter()
                .filter(|e| e.active)
                .map(|e| json!({"code":e.code,"label":e.label}))
                .collect::<Vec<_>>()))
        })
    }
}
