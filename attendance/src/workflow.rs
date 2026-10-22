//! Shared transaction guard for Bilimbi Attendance's roster, adjustment and allowance workflows.
use crate::{access, AttendanceError as Refusal, Result};
use bilibi_database::{Error, MutationTransaction};
use bilibi_tenancy::Scope;
use serde_json::Value;

pub(crate) fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|_| Refusal::Failed(Error::Internal))
}
pub(crate) async fn lock(
    tx: &MutationTransaction<'_>,
    scope: &Scope,
    company: i64,
    capability: &str,
) -> Result<()> {
    access::lock_company(tx, scope, company).await?;
    bilibi_people_workforce::authorization::authorize(tx.sql(), scope, company, capability).await?;
    Ok(())
}
pub(crate) async fn action(
    tx: &MutationTransaction<'_>,
    scope: &Scope,
    event: &str,
    payload: Value,
) -> Result<()> {
    bilibi_audit::record_action(
        tx.sql(),
        &bilibi_audit::Attribution::from_scope(scope),
        None,
        event,
        payload,
        true,
    )
    .await?;
    Ok(())
}
