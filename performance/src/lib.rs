//! Performance (`people/performance`), ported from Bilimbi.People.Performance.
//! One facade owns immutable descriptions, KPI targets, evidence and reviews.
//! Pages and other callers use this same authorization boundary.
mod access;
mod contributions;
mod descriptions;
mod inputs;
mod reads;
mod reviews;
mod store;
mod tables;
mod targets;
mod types;
mod validate;
mod web;

pub use bilibi_people_workforce::FieldError;
pub use contributions::{
    CAPABILITIES, DESCRIPTIONS_MANAGE, KPIS_APPROVE, KPIS_REVIEW, KPIS_SUBMIT, REVIEWS_APPROVE,
    REVIEWS_SUBMIT, SELF_VIEW, VIEW,
};
pub use inputs::*;
pub use types::*;

use bilibi_database::{Db, Error, MutationBoundary, MutationTransaction};
use bilibi_people_organisation::Organisation;
use bilibi_people_skills::Skills;
use bilibi_people_workforce::{NotCurrent, Workforce, WorkforceError};
use std::sync::Arc;

pub const MODULE: &str = "people/performance";
pub const SERVICE: &str = "people/performance.performance";
pub type Result<T> = std::result::Result<T, PerformanceError>;

/// Bilimbi refusal atoms and the exact sentences its pages show.
#[derive(Debug, thiserror::Error)]
pub enum PerformanceError {
    #[error("You cannot do that for this company.")]
    Unauthorized,
    #[error("That record is not available to you.")]
    NotFound,
    #[error("End impersonation before recording performance decisions.")]
    ImpersonationRefused,
    #[error("Choose a current employee who reports directly to you.")]
    OutOfReach,
    #[error("A current employee must be linked to this account.")]
    Unavailable,
    #[error("Choose an exact published competency profile version.")]
    ProfileUnavailable,
    #[error("The position version is not applicable on that date.")]
    PositionVersionUnavailable,
    #[error("That position is not available in this company.")]
    PositionUnavailable,
    #[error("A published description already covers these dates.")]
    OverlappingDescription,
    #[error("The position assignment must cover the review period.")]
    AssignmentUnavailable,
    #[error("A published position description must cover the review period.")]
    DescriptionUnavailable,
    #[error("The author and the employee cannot approve their own record.")]
    SelfApproval,
    #[error("Only a reviewed, non-confidential target can be communicated.")]
    NotPublishable,
    #[error("Pin at least one observation and communicated target.")]
    EvidenceRequired,
    #[error("The evidence must concern this employee, period and cutoff.")]
    EvidenceOutsideWindow,
    #[error("The target must be communicated to this employee within this period and cutoff.")]
    TargetUnavailable,
    #[error("The cutoff has not passed yet.")]
    FutureCutoff,
    #[error("This version already has a correction.")]
    AlreadyCorrected,
    #[error("Check the period, effective dates and UTC cutoff.")]
    InvalidDates,
    #[error("State the reason for this decision or correction.")]
    ReasonRequired,
    #[error("Only the original author can correct this released record.")]
    NotAuthor,
    #[error("The record conflicts with existing history or release rules.")]
    InvariantRefused,
    #[error("The workforce is not current. Try again when its data is available.")]
    NotCurrent(NotCurrent),
    #[error("This company is archived and read-only")]
    CompanyArchived,
    #[error("The record cannot move from its current state.")]
    NotPending,
    #[error("The record cannot move from its current state.")]
    NotReleased,
    #[error("The record cannot move from its current state.")]
    RubricRequired,
    #[error("The record cannot move from its current state.")]
    InvalidEvidence,
    #[error("The record cannot move from its current state.")]
    InvalidOptions,
    #[error("Check the required fields and their values.")]
    Invalid(Vec<FieldError>),
    #[error(transparent)]
    Failed(Error),
}
impl From<Error> for PerformanceError {
    fn from(error: Error) -> Self {
        use std::error::Error as _;
        if let Error::Database(traced) = &error {
            if traced
                .source()
                .and_then(|e| e.downcast_ref::<tokio_postgres::Error>())
                .and_then(|e| e.as_db_error())
                .is_some_and(|e| matches!(e.code().code(), "23514" | "23505" | "23503"))
            {
                return Self::InvariantRefused;
            }
        }
        match error {
            Error::Forbidden | Error::Unauthorized => Self::Unauthorized,
            Error::NotFound => Self::NotFound,
            other => Self::Failed(other),
        }
    }
}
impl From<WorkforceError> for PerformanceError {
    fn from(error: WorkforceError) -> Self {
        match error {
            WorkforceError::Unauthorized => Self::Unauthorized,
            WorkforceError::NotLinked => Self::Unavailable,
            WorkforceError::CompanyArchived => Self::CompanyArchived,
            WorkforceError::Failed(error) => error.into(),
            _ => Self::NotFound,
        }
    }
}
impl From<NotCurrent> for PerformanceError {
    fn from(error: NotCurrent) -> Self {
        Self::NotCurrent(error)
    }
}
impl From<bilibi_company::WriteError> for PerformanceError {
    fn from(error: bilibi_company::WriteError) -> Self {
        match error {
            bilibi_company::WriteError::Archived => Self::CompanyArchived,
            bilibi_company::WriteError::NotFound => Self::NotFound,
            bilibi_company::WriteError::Access(error) => error.into(),
            _ => Self::Unauthorized,
        }
    }
}

pub struct Performance {
    db: Db,
    mutations: Arc<MutationBoundary>,
    workforce: Arc<Workforce>,
    organisation: Arc<Organisation>,
    skills: Arc<Skills>,
}
struct Write<'a> {
    tx: MutationTransaction<'a>,
    tenant: i64,
    company: i64,
    actor: i64,
}
