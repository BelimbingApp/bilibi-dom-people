//! Company progression policies and eligibility, ported from Bilimbi.People.Progression.
//! This explanation never changes employment, position, promotion or pay.
mod access;
mod contributions;
mod explain;
mod policies;
mod store;
mod types;
mod validate;

pub use contributions::{CAPABILITIES, POLICY_MANAGE, POLICY_VIEW, SELF_VIEW};
pub use types::*;

use bilibi_database::{Db, Error, MutationBoundary, MutationTransaction};
use bilibi_people_performance::Performance;
use bilibi_people_skills::Skills;
use bilibi_people_workforce::{NotCurrent, Workforce, WorkforceError};
use std::sync::Arc;

pub const MODULE: &str = "people/progression";
pub const SERVICE: &str = "people/progression.progression";
pub type Result<T> = std::result::Result<T, ProgressionError>;

/// Bilimbi Progression.Web.Support's refusal sentences.
#[derive(Debug, thiserror::Error)]
pub enum ProgressionError {
    #[error(
        "You cannot do that for this company. Check your progression and evidence permissions."
    )]
    Unauthorized,
    #[error(
        "The record is unavailable. Check your company, employee link and evidence permissions."
    )]
    NotFound,
    #[error("Check the required policy fields.")]
    InvalidPolicy,
    #[error(
        "Choose competency requirements or a performance period, and complete their criteria."
    )]
    InvalidRules,
    #[error("Choose an exact published competency profile with requirements.")]
    ProfileUnavailable,
    #[error("Publish a version higher than the existing versions for this policy code.")]
    VersionOrder,
    #[error("Choose an effective date on or after the latest published effective date for this policy code.")]
    EffectiveOrder,
    #[error("This policy conflicts with immutable history or an existing version.")]
    InvariantRefused,
    #[error("This policy version has already been published.")]
    AlreadyPublished,
    #[error("End impersonation before recording a policy.")]
    ImpersonationRefused,
    #[error("A working employee must be linked to your account.")]
    EmployeeUnavailable,
    #[error("No progression policy has been published with an effective date on or before today.")]
    NoPublishedPolicy,
    #[error(
        "The record is unavailable. Check your company, employee link and evidence permissions."
    )]
    InvalidOptions,
    #[error("Current workforce data is unavailable. Try again when it is current.")]
    NotCurrent(NotCurrent),
    #[error("This company is archived and read-only")]
    CompanyArchived,
    #[error(transparent)]
    Skills(#[from] bilibi_people_skills::SkillsError),
    #[error(transparent)]
    Performance(#[from] bilibi_people_performance::PerformanceError),
    #[error(transparent)]
    Failed(Error),
}
impl From<Error> for ProgressionError {
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
impl From<WorkforceError> for ProgressionError {
    fn from(error: WorkforceError) -> Self {
        match error {
            WorkforceError::Unauthorized => Self::Unauthorized,
            WorkforceError::NotLinked => Self::EmployeeUnavailable,
            WorkforceError::CompanyArchived => Self::CompanyArchived,
            WorkforceError::Failed(error) => error.into(),
            _ => Self::EmployeeUnavailable,
        }
    }
}
impl From<NotCurrent> for ProgressionError {
    fn from(error: NotCurrent) -> Self {
        Self::NotCurrent(error)
    }
}
impl From<bilibi_company::WriteError> for ProgressionError {
    fn from(error: bilibi_company::WriteError) -> Self {
        match error {
            bilibi_company::WriteError::Archived => Self::CompanyArchived,
            bilibi_company::WriteError::NotFound => Self::NotFound,
            bilibi_company::WriteError::Access(error) => error.into(),
            _ => Self::Unauthorized,
        }
    }
}
pub struct Progression {
    db: Db,
    mutations: Arc<MutationBoundary>,
    workforce: Arc<Workforce>,
    skills: Arc<Skills>,
    performance: Arc<Performance>,
}
struct Write<'a> {
    tx: MutationTransaction<'a>,
    tenant: i64,
    company: i64,
    actor: i64,
}
