//! Company and employee resolution shared by every attendance workflow
//! (Bilimbi `Bilimbi.People.Attendance.Access`): both come from the Workforce
//! facade, and only a current read lets an operation go on.
use crate::{AttendanceError, Result};
use bilibi_company::{organisation, WriteError};
use bilibi_database::MutationTransaction;
use bilibi_people_workforce::{Company, Employee, Workforce};
use bilibi_tenancy::Scope;

/// A live company of the scope's tenant, as the workforce names it now.
pub(crate) async fn current_company(
    workforce: &Workforce,
    scope: &Scope,
    company_id: i64,
) -> Result<Company> {
    Ok(workforce
        .company(scope, company_id)
        .await?
        .require_current()?)
}

/// A working employee of the company, as the workforce names them now.
pub(crate) async fn current_employee(
    workforce: &Workforce,
    scope: &Scope,
    company_id: i64,
    employee_id: i64,
) -> Result<Employee> {
    Ok(workforce
        .employee(scope, company_id, employee_id)
        .await?
        .require_current()?)
}

/// The guard every attendance write begins with: the company's writable
/// lock, held for the transaction, which refuses an archived company and
/// serializes the company's writes.
pub(crate) async fn lock_company(
    tx: &MutationTransaction<'_>,
    scope: &Scope,
    company_id: i64,
) -> Result<()> {
    organisation::require_writable(tx.sql(), scope, company_id, true)
        .await
        .map(|_| ())
        .map_err(|error| match error {
            WriteError::NotFound => AttendanceError::NotFound,
            WriteError::Archived => AttendanceError::CompanyArchived,
            WriteError::Access(error) => error.into(),
            _ => AttendanceError::Unauthorized,
        })
}

/// Recheck the operator's grant on the transaction after waiting for the
/// company lock, so a revoked grant cannot authorize a queued write.
pub(crate) async fn lock_rules_company(
    tx: &MutationTransaction<'_>,
    scope: &Scope,
    company_id: i64,
) -> Result<()> {
    lock_company(tx, scope, company_id).await?;
    bilibi_people_workforce::authorization::authorize(
        tx.sql(),
        scope,
        company_id,
        crate::RULES_MANAGE,
    )
    .await?;
    Ok(())
}
