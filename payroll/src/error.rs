//! Bilimbi Payroll refusal atoms, using its page sentences.
use bilibi_database::Error;
use bilibi_people_workforce::{FieldError, Freshness, WorkforceError};
#[derive(Debug, thiserror::Error)]
pub enum PayrollError {
    #[error("You cannot change this company's payroll setup.")]
    Unauthorized,
    #[error("Not found")]
    NotFound,
    #[error("This company is archived and read-only")]
    CompanyArchived,
    #[error("This code or source already has a version during those dates. Choose a non-overlapping period.")]
    OverlappingVersion,
    #[error("This run is locked and cannot change.")]
    Locked,
    #[error("Enter a country identifier and uppercase three-letter currency codes.")]
    InvalidSettings,
    #[error("Choose a classification covering the item dates, an allowed currency, and an amount with at most six decimal places.")]
    InvalidItem,
    #[error("Choose an available Leave or Claims source and an item covering the mapping dates.")]
    InvalidMapping,
    #[error("Choose an active allowance rule, a pay item in the same currency and dates covered by the item.")]
    InvalidAttendanceMapping,
    #[error("Workforce data for this company is not current, so Leave and Claims sources cannot be read. Try again once it has refreshed.")]
    NotCurrent(Freshness),
    #[error("Choose a configured country, currency and unused period.")]
    RunUnavailable,
    #[error("Check required fields, date ranges and unique codes.")]
    Invalid(Vec<FieldError>),
    #[error(transparent)]
    Failed(Error),
}
impl From<Error> for PayrollError {
    fn from(error: Error) -> Self {
        match error {
            Error::Forbidden | Error::Unauthorized => Self::Unauthorized,
            Error::NotFound => Self::NotFound,
            other => Self::Failed(other),
        }
    }
}
impl From<tokio_postgres::Error> for PayrollError {
    fn from(error: tokio_postgres::Error) -> Self {
        Error::from(error).into()
    }
}
impl From<WorkforceError> for PayrollError {
    fn from(error: WorkforceError) -> Self {
        match error {
            WorkforceError::Unauthorized => Self::Unauthorized,
            WorkforceError::NotFound => Self::NotFound,
            WorkforceError::CompanyArchived => Self::CompanyArchived,
            WorkforceError::Invalid(fields) => Self::Invalid(fields),
            WorkforceError::Failed(error) => error.into(),
            _ => Self::Failed(Error::Internal),
        }
    }
}
impl From<bilibi_people_leave::LeaveError> for PayrollError {
    fn from(error: bilibi_people_leave::LeaveError) -> Self {
        use bilibi_people_leave::LeaveError as E;
        match error {
            E::NotCurrent(f) => Self::NotCurrent(f),
            E::Unauthorized => Self::Unauthorized,
            E::NotFound => Self::NotFound,
            E::CompanyArchived => Self::CompanyArchived,
            E::Failed(e) => e.into(),
            _ => Self::Failed(Error::Internal),
        }
    }
}
impl From<bilibi_people_claims::ClaimsError> for PayrollError {
    fn from(error: bilibi_people_claims::ClaimsError) -> Self {
        use bilibi_people_claims::ClaimsError as E;
        match error {
            E::Unauthorized => Self::Unauthorized,
            E::NotFound => Self::NotFound,
            E::CompanyArchived => Self::CompanyArchived,
            E::Failed(e) => e.into(),
            _ => Self::Failed(Error::Internal),
        }
    }
}
impl From<bilibi_people_attendance::AttendanceError> for PayrollError {
    fn from(error: bilibi_people_attendance::AttendanceError) -> Self {
        use bilibi_people_attendance::AttendanceError as E;
        match error {
            E::Unavailable(f) => Self::NotCurrent(f.0),
            E::Unauthorized => Self::Unauthorized,
            E::NotFound => Self::NotFound,
            E::CompanyArchived => Self::CompanyArchived,
            E::Failed(e) => e.into(),
            _ => Self::Failed(Error::Internal),
        }
    }
}
