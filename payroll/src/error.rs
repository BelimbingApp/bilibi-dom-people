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
    #[error("Choose an allowance rule and a pay item effective for the whole mapping period.")]
    InvalidAttendanceMapping,
    #[error("Workforce data for this company is not current, so Leave and Claims sources cannot be read. Try again once it has refreshed.")]
    NotCurrent(Freshness),
    #[error("Choose a configured country, currency and unused period.")]
    RunUnavailable,
    #[error("Check required fields, date ranges and unique codes.")]
    Invalid(Vec<FieldError>),
    #[error("Payroll action refused: :intake_unavailable")]
    IntakeUnavailable,
    #[error("Payroll action refused: :replay_conflict")]
    ReplayConflict,
    #[error("Payroll action refused: :source_not_mapped")]
    SourceNotMapped,
    #[error("Payroll action refused: :allowance_not_mapped")]
    AllowanceNotMapped,
    #[error("Payroll action refused: :direction_conflict")]
    DirectionConflict,
    #[error("Payroll action refused: :run_not_locked")]
    RunNotLocked,
    #[error("Payroll action refused: :no_contributions")]
    NoContributions,
    #[error("Payroll action refused: :employee_not_found")]
    EmployeeNotFound,
    #[error("Payroll action refused: :invalid_decision")]
    InvalidDecision,
    #[error("Payroll action refused: :not_decidable")]
    NotDecidable,
    #[error("The amount exceeds the exact decimal range. Reduce the units or rate.")]
    DecimalRange,
    #[error(
        "The approved components exceed the event limit of 100 components or 64 KiB. Use fewer pay items or directions in one run."
    )]
    ApprovalTooLarge,
    #[error("Payroll action refused: :invalid_document")]
    InvalidDocument,
    #[error("Private documents require an operator-configured Base Artifacts storage directory and retention period.")]
    ArtifactPolicy,
    #[error(transparent)]
    ArtifactFailed(bilibi_artifacts::Error),
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
