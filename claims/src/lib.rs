//! Claims (`people/claims`, Bilimbi `Bilimbi.People.Claims`): a company's
//! claim catalog, its effective-dated claim policies and the assignments
//! that open assigned-only claim types to employees.
//!
//! This is the module's one facade. Pages and sibling modules call
//! [`Claims`], resolved from the registry as [`SERVICE`]; the schema, SQL
//! and validation stay private. Every function takes the sealed [`Scope`] it
//! was handed and an explicit platform company id, and authorizes its own
//! capability through the Workforce `Authorization` helpers when it runs: a
//! grant proven at page mount is not authority. Catalog and policy writes
//! need `people.claims.manage`; self-service needs `people.claims.submit`.
//! Reads of a company's catalog, policies and assignments are
//! capability-free company configuration, as Bilimbi's are, for any account
//! that can see the company.
//!
//! Claim currencies are the company setting `people.claims.currencies`. Its
//! default is empty: a company accepts no claim until an operator chooses
//! its currencies, and the module ships no currency list. A claim type
//! belongs to one category and declares a receipt rule (`always`,
//! `above_threshold`, `never`) and who may claim it (`all_employees`,
//! `assigned_only`). A policy gives one claim type a period, a currency from
//! the company's list and optional per-claim, calendar-month and
//! calendar-year limits; a threshold rule needs the policy's threshold;
//! periods of one claim type never overlap; an open-ended policy can be
//! ended, but not before a live claim incurred under it. An assignment is a
//! named, effective-dated group of claim types and employees.
//!
//! Requests, decisions and hand-off preserve the employee lock, status history
//! and transactional events consumed by other Domains.
//!
//! See `README.md`.
mod assignments;
mod catalog;
mod contributions;
mod downloads;
mod handoff;
mod money;
mod operations_web;
mod policies;
mod requests;
mod web;
mod workflow;

pub use handoff::{HandoffBatch, HandoffExport, HandoffWaiting};
pub use requests::{
    Approval, NewRequest, QueueClaim, Reimbursement, Rejection, Request, RequestEvent,
};
pub use workflow::{REQUEST_APPROVED, REQUEST_REIMBURSED};

pub use bilibi_people_workforce::FieldError;
pub use contributions::{APPROVE, CAPABILITIES, CURRENCIES_KEY, MANAGE, REIMBURSE, SUBMIT};

use bilibi_company::{organisation, WriteError};
use bilibi_database::{Db, Error, MutationBoundary, MutationTransaction};
use bilibi_party_ids::OrganisationId;
use bilibi_people_workforce::{authorization, Workforce, WorkforceError};
use bilibi_settings::{Settings, Target};
use bilibi_tenancy::Scope;
use jiff::{civil::Date, tz::TimeZone, Timestamp};
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::json;
use std::sync::Arc;
use tokio_postgres::GenericClient;

/// The module ID, Bilimbi's, kept because IDs are persisted data.
pub const MODULE: &str = "people/claims";
/// The registry service ID of the [`Claims`] facade.
pub const SERVICE: &str = "people/claims.claims";
/// The most currencies a company lists.
pub const MAX_CURRENCIES: usize = 20;
/// The most claim types or employees one assignment call names.
pub const MEMBER_LIMIT: usize = 1_000;
/// A claim type's receipt rules.
pub const RECEIPT_REQUIREMENTS: [&str; 3] = ["always", "above_threshold", "never"];
/// Who may claim a claim type.
pub const ELIGIBILITIES: [&str; 2] = ["all_employees", "assigned_only"];
/// A claim's statuses; a withdrawn or rejected claim stops counting toward
/// limits and releases its receipt reference.
pub const REQUEST_STATUSES: [&str; 5] = [
    "submitted",
    "approved",
    "rejected",
    "reimbursed",
    "withdrawn",
];

pub type Result<T> = std::result::Result<T, ClaimsError>;

/// Why a call did not happen (Bilimbi's refusal atoms and changeset errors).
#[derive(Debug, thiserror::Error)]
pub enum ClaimsError {
    /// A denied or unknown capability, or a company outside the actor's reach.
    #[error("Access denied")]
    Unauthorized,
    /// A company or record the scope cannot see: missing, deleted or another
    /// company's (indistinguishable on purpose).
    #[error("Not found")]
    NotFound,
    /// The actor's account is not linked to a working employee of the company.
    #[error("Your account is not linked to an employee of this company")]
    NotLinked,
    #[error("This company is archived and read-only")]
    CompanyArchived,
    /// Not a list of at most twenty three-letter uppercase codes.
    #[error("Invalid currencies")]
    InvalidCurrencies,
    #[error("Category not found")]
    CategoryNotFound,
    #[error("Claim type not found")]
    ClaimTypeNotFound,
    /// An employee who does not belong to the company.
    #[error("Employee not found")]
    EmployeeNotFound,
    #[error("This period overlaps another policy for the claim type.")]
    OverlappingPolicy,
    /// The policy or assignment already has an end date.
    #[error("Already ended")]
    AlreadyEnded,
    #[error("Claims under this policy were incurred after that date.")]
    RequestsAfterEnd,
    #[error("Choose a claim type that is open for claims.")]
    ClaimTypeUnavailable,
    #[error("This claim type is not assigned to you.")]
    ClaimTypeNotAssigned,
    #[error("The expense date cannot be in the future.")]
    FutureIncurredOn,
    #[error("No claim policy covers this type on the expense date.")]
    NoEffectivePolicy,
    #[error("Use the currency shown for this claim type.")]
    CurrencyNotAllowed,
    #[error("This claim needs a receipt number.")]
    ReceiptRequired,
    #[error("The amount is above the per-claim limit.")]
    PerClaimLimitExceeded,
    #[error("The amount would exceed this month's limit.")]
    MonthlyLimitExceeded,
    #[error("The amount would exceed this year's limit.")]
    YearlyLimitExceeded,
    #[error("This receipt number is already on one of your claims.")]
    DuplicateReceipt,
    #[error("You already claimed this type, date, and amount. Confirm it is a separate expense to submit.")]
    PossibleDuplicate,
    #[error("This claim is no longer withdrawable.")]
    NotWithdrawable,
    #[error("This claim is no longer in the right state for that action.")]
    NotDecidable,
    #[error("You cannot act on your own claim.")]
    OwnClaim,
    #[error("No approved claims in that currency are waiting.")]
    NothingToHandOff,
    #[error("No approved claims are left in this batch.")]
    NothingToReimburse,
    #[error("Invalid currency")]
    InvalidCurrency,
    /// Refused values, in the words a form shows beside each field.
    #[error("Invalid input")]
    Invalid(Vec<FieldError>),
    /// A database or capture failure; never a refusal.
    #[error(transparent)]
    Failed(Error),
}
impl From<Error> for ClaimsError {
    fn from(error: Error) -> Self {
        match error {
            Error::Forbidden | Error::Unauthorized => Self::Unauthorized,
            Error::NotFound => Self::NotFound,
            other => Self::Failed(other),
        }
    }
}
impl From<WorkforceError> for ClaimsError {
    fn from(error: WorkforceError) -> Self {
        match error {
            WorkforceError::Unauthorized => Self::Unauthorized,
            WorkforceError::NotFound => Self::NotFound,
            WorkforceError::NotLinked => Self::NotLinked,
            WorkforceError::CompanyArchived => Self::CompanyArchived,
            WorkforceError::Invalid(errors) => Self::Invalid(errors),
            WorkforceError::Failed(error) => Self::Failed(error),
            // The ids this facade hands Workforce are validated first.
            _ => Self::Failed(Error::Internal),
        }
    }
}
impl From<bilibi_party_ids::InvalidId> for ClaimsError {
    fn from(_: bilibi_party_ids::InvalidId) -> Self {
        Self::NotFound
    }
}

/// A claim category (Bilimbi `people_claim_catalog_groups`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Category {
    pub id: i64,
    pub code: String,
    pub name: String,
    pub active: bool,
}

/// A claim type: its category, receipt rule and eligibility.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ClaimType {
    pub id: i64,
    pub category_id: i64,
    pub code: String,
    pub name: String,
    pub receipt_requirement: String,
    pub eligibility: String,
    pub active: bool,
}

/// One effective-dated policy of a claim type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Policy {
    pub id: i64,
    pub claim_type_id: i64,
    #[serde(serialize_with = "as_text")]
    pub effective_from: Date,
    #[serde(serialize_with = "as_optional_text")]
    pub effective_to: Option<Date>,
    pub currency: String,
    pub per_claim_limit: Option<Decimal>,
    pub monthly_limit: Option<Decimal>,
    pub yearly_limit: Option<Decimal>,
    pub receipt_threshold: Option<Decimal>,
}

/// A claim type open for claims on a date, with its category's name and the
/// policy in effect that day.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OpenClaimType {
    #[serde(flatten)]
    pub claim_type: ClaimType,
    pub category_name: String,
    pub policy: Policy,
}

/// A named, effective-dated group (Bilimbi `people_claim_employee_enrolments`)
/// with the claim types it opens and the employees it covers, by id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Assignment {
    pub id: i64,
    pub code: String,
    pub name: String,
    #[serde(serialize_with = "as_text")]
    pub effective_from: Date,
    #[serde(serialize_with = "as_optional_text")]
    pub effective_to: Option<Date>,
    pub claim_type_ids: Vec<i64>,
    pub employee_ids: Vec<i64>,
}

/// What `set_assignment_members` saved, sorted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AssignmentMembers {
    pub claim_type_ids: Vec<i64>,
    pub employee_ids: Vec<i64>,
}

/// An employee an assignment may cover, for a claims manager's picker.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AssignableEmployee {
    pub id: i64,
    pub employee_number: String,
    pub name: String,
}

/// The working employee the scope's actor is linked to in a company.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SelfEmployee {
    pub id: i64,
    pub display_name: String,
    pub employee_number: String,
}

fn as_text<T: std::fmt::Display, S: serde::Serializer>(
    value: &T,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.collect_str(value)
}
fn as_optional_text<T: std::fmt::Display, S: serde::Serializer>(
    value: &Option<T>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    match value {
        Some(value) => serializer.collect_str(value),
        None => serializer.serialize_none(),
    }
}

/// What creates a category: a code (unique in the company) and a name.
#[derive(Clone, Debug)]
pub struct NewCategory {
    pub code: String,
    pub name: String,
}

/// What creates a claim type: its category, a code (unique in the company),
/// a name, a receipt rule and who may claim it (`all_employees` when absent).
#[derive(Clone, Debug)]
pub struct NewClaimType {
    pub category_id: i64,
    pub code: String,
    pub name: String,
    pub receipt_requirement: String,
    pub eligibility: Option<String>,
}

/// What creates a policy: the claim type, the period (open-ended without
/// `effective_to`), a currency from the company's list and optional limits
/// and receipt threshold, each a two-decimal amount.
#[derive(Clone, Debug)]
pub struct NewPolicy {
    pub claim_type_id: i64,
    pub effective_from: Date,
    pub effective_to: Option<Date>,
    pub currency: String,
    pub per_claim_limit: Option<Decimal>,
    pub monthly_limit: Option<Decimal>,
    pub yearly_limit: Option<Decimal>,
    pub receipt_threshold: Option<Decimal>,
}

/// What creates an assignment: a code (unique in the company), a name and
/// the period. Members are set separately.
#[derive(Clone, Debug)]
pub struct NewAssignment {
    pub code: String,
    pub name: String,
    pub effective_from: Date,
    pub effective_to: Option<Date>,
}

/// The facade, resolved from the registry as [`SERVICE`].
pub struct Claims {
    db: Db,
    mutations: Arc<MutationBoundary>,
    settings: Arc<Settings>,
    events: Arc<bilibi_events::Events>,
    workforce: Arc<Workforce>,
}

impl Claims {
    // Currencies

    /// The company's allowed claim currencies, in operator order
    /// (`people.claims.currencies`; default none).
    pub async fn currencies(&self, scope: &Scope, company_id: i64) -> Result<Vec<String>> {
        let client = self.db.get().await.map_err(Error::from)?;
        company(&**client, scope, company_id).await?;
        self.company_currencies(&**client, scope, company_id).await
    }

    /// Replaces the company's allowed claim currencies
    /// (`people.claims.manage`): at most twenty codes of three letters, kept
    /// uppercase, each once. An empty list closes new policies and claims.
    pub async fn put_currencies(
        &self,
        scope: &Scope,
        company_id: i64,
        codes: &[String],
    ) -> Result<Vec<String>> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, MANAGE).await?;
        company(&**client, scope, company_id).await?;
        let codes = normalize_currencies(codes)?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.claims.put_currencies"),
            )
            .await?;
        writable(tx.sql(), scope, company_id).await?;
        authorization::authorize(tx.sql(), scope, company_id, MANAGE).await?;
        self.settings
            .put(
                &mut tx,
                scope,
                Target::Company(OrganisationId::new(company_id)?),
                CURRENCIES_KEY,
                json!(codes),
            )
            .await?;
        tx.commit().await?;
        Ok(codes)
    }

    // Catalog

    /// The company's categories, by name.
    pub async fn categories(&self, scope: &Scope, company_id: i64) -> Result<Vec<Category>> {
        let client = self.db.get().await.map_err(Error::from)?;
        company(&**client, scope, company_id).await?;
        catalog::categories(&**client, scope.tenant(), company_id).await
    }

    /// Adds a category (`people.claims.manage`); a blank or overlong code
    /// or name and a taken code are `Invalid` field errors.
    pub async fn create_category(
        &self,
        scope: &Scope,
        company_id: i64,
        new: &NewCategory,
    ) -> Result<Category> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, MANAGE).await?;
        let valid = catalog::validate_category(new).map_err(ClaimsError::Invalid)?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.claims.create_category"),
            )
            .await?;
        writable(tx.sql(), scope, company_id).await?;
        authorization::authorize(tx.sql(), scope, company_id, MANAGE).await?;
        catalog::check_category_code(tx.sql(), company_id, &valid.code).await?;
        let category = catalog::insert_category(&mut tx, scope.tenant(), company_id, valid).await?;
        tx.commit().await?;
        Ok(category)
    }

    /// Activates or deactivates a category (`people.claims.manage`); an
    /// inactive category closes its claim types to new claims.
    pub async fn set_category_active(
        &self,
        scope: &Scope,
        company_id: i64,
        category_id: i64,
        active: bool,
    ) -> Result<Category> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, MANAGE).await?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.claims.set_category_active"),
            )
            .await?;
        writable(tx.sql(), scope, company_id).await?;
        authorization::authorize(tx.sql(), scope, company_id, MANAGE).await?;
        let category = catalog::lock_category(tx.sql(), scope.tenant(), company_id, category_id)
            .await?
            .ok_or(ClaimsError::NotFound)?;
        if category.active == active {
            tx.rollback().await?;
            return Ok(category);
        }
        let category = catalog::set_category_active(&mut tx, category, active).await?;
        tx.commit().await?;
        Ok(category)
    }

    /// The company's claim types, by name.
    pub async fn claim_types(&self, scope: &Scope, company_id: i64) -> Result<Vec<ClaimType>> {
        let client = self.db.get().await.map_err(Error::from)?;
        company(&**client, scope, company_id).await?;
        catalog::claim_types(&**client, scope.tenant(), company_id).await
    }

    /// Adds a claim type (`people.claims.manage`) to one of the company's
    /// categories (`CategoryNotFound`); a blank or overlong code or name, an
    /// unknown receipt rule or eligibility and a taken code are `Invalid`.
    pub async fn create_claim_type(
        &self,
        scope: &Scope,
        company_id: i64,
        new: &NewClaimType,
    ) -> Result<ClaimType> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, MANAGE).await?;
        let valid = catalog::validate_claim_type(new).map_err(ClaimsError::Invalid)?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.claims.create_claim_type"),
            )
            .await?;
        writable(tx.sql(), scope, company_id).await?;
        authorization::authorize(tx.sql(), scope, company_id, MANAGE).await?;
        let category =
            catalog::find_category(tx.sql(), scope.tenant(), company_id, new.category_id)
                .await?
                .ok_or(ClaimsError::CategoryNotFound)?;
        catalog::check_claim_type_code(tx.sql(), company_id, &valid.code).await?;
        let claim_type =
            catalog::insert_claim_type(&mut tx, scope.tenant(), company_id, category.id, valid)
                .await?;
        tx.commit().await?;
        Ok(claim_type)
    }

    /// Activates or deactivates a claim type (`people.claims.manage`).
    pub async fn set_claim_type_active(
        &self,
        scope: &Scope,
        company_id: i64,
        claim_type_id: i64,
        active: bool,
    ) -> Result<ClaimType> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, MANAGE).await?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.claims.set_claim_type_active"),
            )
            .await?;
        writable(tx.sql(), scope, company_id).await?;
        authorization::authorize(tx.sql(), scope, company_id, MANAGE).await?;
        let claim_type =
            catalog::lock_claim_type(tx.sql(), scope.tenant(), company_id, claim_type_id)
                .await?
                .ok_or(ClaimsError::NotFound)?;
        if claim_type.active == active {
            tx.rollback().await?;
            return Ok(claim_type);
        }
        let claim_type = catalog::set_claim_type_active(&mut tx, claim_type, active).await?;
        tx.commit().await?;
        Ok(claim_type)
    }

    // Policies

    /// The company's policies, by claim type and newest period first.
    pub async fn policies(&self, scope: &Scope, company_id: i64) -> Result<Vec<Policy>> {
        let client = self.db.get().await.map_err(Error::from)?;
        company(&**client, scope, company_id).await?;
        policies::list(&**client, scope.tenant(), company_id).await
    }

    /// Adds an effective-dated policy for one claim type
    /// (`people.claims.manage`).
    ///
    /// The currency must be one of the company's claim currencies; the end,
    /// when given, not before the start; each limit positive and the
    /// threshold not negative, with at most two decimals and below the
    /// column's ceiling (`Invalid` field errors). A threshold is required
    /// when the claim type asks for receipts above a threshold and dropped
    /// otherwise. Under the claim type's row lock the period must not
    /// overlap another policy of the type (`OverlappingPolicy`).
    pub async fn create_policy(
        &self,
        scope: &Scope,
        company_id: i64,
        new: &NewPolicy,
    ) -> Result<Policy> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, MANAGE).await?;
        company(&**client, scope, company_id).await?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.claims.create_policy"),
            )
            .await?;
        writable(tx.sql(), scope, company_id).await?;
        authorization::authorize(tx.sql(), scope, company_id, MANAGE).await?;
        let currencies = self.company_currencies(tx.sql(), scope, company_id).await?;
        let mut valid = policies::validate(new, &currencies).map_err(ClaimsError::Invalid)?;
        let claim_type =
            catalog::lock_claim_type(tx.sql(), scope.tenant(), company_id, new.claim_type_id)
                .await?
                .ok_or(ClaimsError::ClaimTypeNotFound)?;
        policies::apply_receipt_rule(&mut valid, &claim_type).map_err(ClaimsError::Invalid)?;
        if policies::overlaps(
            tx.sql(),
            scope.tenant(),
            company_id,
            claim_type.id,
            valid.effective_from,
            valid.effective_to,
        )
        .await?
        {
            return Err(ClaimsError::OverlappingPolicy);
        }
        let policy =
            policies::insert(&mut tx, scope.tenant(), company_id, claim_type.id, valid).await?;
        tx.commit().await?;
        Ok(policy)
    }

    /// Closes an open-ended policy on `effective_to` (`people.claims.manage`):
    /// a policy that already ends is `AlreadyEnded`, an end before the start
    /// is `Invalid`, and an end before a live claim incurred under the
    /// policy is `RequestsAfterEnd`.
    pub async fn end_policy(
        &self,
        scope: &Scope,
        company_id: i64,
        policy_id: i64,
        effective_to: Date,
    ) -> Result<Policy> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, MANAGE).await?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.claims.end_policy"),
            )
            .await?;
        writable(tx.sql(), scope, company_id).await?;
        authorization::authorize(tx.sql(), scope, company_id, MANAGE).await?;
        let unlocked = policies::find(tx.sql(), scope.tenant(), company_id, policy_id)
            .await?
            .ok_or(ClaimsError::NotFound)?;
        // Lock order: the claim type (which `create_policy` holds while it
        // checks overlaps), then the policy row.
        catalog::lock_claim_type(tx.sql(), scope.tenant(), company_id, unlocked.claim_type_id)
            .await?;
        let policy = policies::lock(tx.sql(), scope.tenant(), company_id, policy_id)
            .await?
            .ok_or(ClaimsError::NotFound)?;
        if policy.effective_to.is_some() {
            return Err(ClaimsError::AlreadyEnded);
        }
        if effective_to < policy.effective_from {
            return Err(ClaimsError::Invalid(vec![FieldError::new(
                "effective_to",
                "must not be before the start date",
            )]));
        }
        if policies::requests_after(
            tx.sql(),
            scope.tenant(),
            company_id,
            policy.id,
            effective_to,
        )
        .await?
        {
            return Err(ClaimsError::RequestsAfterEnd);
        }
        let policy = policies::end(&mut tx, policy, effective_to).await?;
        tx.commit().await?;
        Ok(policy)
    }

    /// Claim types open for claims on `on_date` (default: today in the
    /// company's time zone), each with the policy in effect that day: active
    /// types of active categories with a policy covering the date, by
    /// category and type name.
    ///
    /// With `employee_id`, assigned-only types appear only when an
    /// assignment in effect on the date covers the type and the employee;
    /// without it every open type appears.
    pub async fn open_claim_types(
        &self,
        scope: &Scope,
        company_id: i64,
        on_date: Option<Date>,
        employee_id: Option<i64>,
    ) -> Result<Vec<OpenClaimType>> {
        let client = self.db.get().await.map_err(Error::from)?;
        company(&**client, scope, company_id).await?;
        let on_date = match on_date {
            Some(date) => date,
            None => self.company_today(&**client, scope, company_id).await?,
        };
        let assigned = match employee_id {
            Some(employee_id) => Some(
                assignments::assigned_type_ids(
                    &**client,
                    scope.tenant(),
                    company_id,
                    employee_id,
                    on_date,
                )
                .await?,
            ),
            None => None,
        };
        policies::open_types(
            &**client,
            scope.tenant(),
            company_id,
            on_date,
            assigned.as_deref(),
        )
        .await
    }

    // Self-service

    /// The working employee the scope's signed-in actor is linked to in the
    /// company, resolved now, once the actor holds `people.claims.submit`
    /// there (Bilimbi `self_service_employee/2`). A login actor is not an
    /// employee: only an account linked to an affiliation that is a working
    /// employment of this very company resolves; anything else is
    /// `NotLinked`.
    pub async fn self_service_employee(
        &self,
        scope: &Scope,
        company_id: i64,
    ) -> Result<SelfEmployee> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.self_employee(&**client, scope, company_id).await
    }

    /// Claim types open today for the actor's own linked employee
    /// (`people.claims.submit`); see [`Claims::open_claim_types`].
    pub async fn self_open_claim_types(
        &self,
        scope: &Scope,
        company_id: i64,
    ) -> Result<Vec<OpenClaimType>> {
        let client = self.db.get().await.map_err(Error::from)?;
        let employee = self.self_employee(&**client, scope, company_id).await?;
        let on_date = self.company_today(&**client, scope, company_id).await?;
        let assigned = assignments::assigned_type_ids(
            &**client,
            scope.tenant(),
            company_id,
            employee.id,
            on_date,
        )
        .await?;
        policies::open_types(
            &**client,
            scope.tenant(),
            company_id,
            on_date,
            Some(&assigned),
        )
        .await
    }

    // Assignments

    /// The company's assignments, by name, each with the claim types it
    /// opens and the employees it covers.
    pub async fn assignments(&self, scope: &Scope, company_id: i64) -> Result<Vec<Assignment>> {
        let client = self.db.get().await.map_err(Error::from)?;
        company(&**client, scope, company_id).await?;
        assignments::list(&**client, scope.tenant(), company_id).await
    }

    /// Adds an effective-dated assignment (`people.claims.manage`); members
    /// are set with [`Claims::set_assignment_members`].
    pub async fn create_assignment(
        &self,
        scope: &Scope,
        company_id: i64,
        new: &NewAssignment,
    ) -> Result<Assignment> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, MANAGE).await?;
        let valid = assignments::validate(new).map_err(ClaimsError::Invalid)?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.claims.create_assignment"),
            )
            .await?;
        writable(tx.sql(), scope, company_id).await?;
        authorization::authorize(tx.sql(), scope, company_id, MANAGE).await?;
        assignments::check_code(tx.sql(), company_id, &valid.code).await?;
        let assignment = assignments::insert(&mut tx, scope.tenant(), company_id, valid).await?;
        tx.commit().await?;
        Ok(assignment)
    }

    /// Closes an open-ended assignment on `effective_to`
    /// (`people.claims.manage`): `AlreadyEnded` when it already ends,
    /// `Invalid` for an end before the start.
    pub async fn end_assignment(
        &self,
        scope: &Scope,
        company_id: i64,
        assignment_id: i64,
        effective_to: Date,
    ) -> Result<Assignment> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, MANAGE).await?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.claims.end_assignment"),
            )
            .await?;
        writable(tx.sql(), scope, company_id).await?;
        authorization::authorize(tx.sql(), scope, company_id, MANAGE).await?;
        let mut assignment = assignments::lock(tx.sql(), scope.tenant(), company_id, assignment_id)
            .await?
            .ok_or(ClaimsError::NotFound)?;
        if assignment.effective_to.is_some() {
            return Err(ClaimsError::AlreadyEnded);
        }
        assignments::members_of(tx.sql(), scope.tenant(), company_id, &mut assignment).await?;
        if effective_to < assignment.effective_from {
            return Err(ClaimsError::Invalid(vec![FieldError::new(
                "effective_to",
                "must not be before the start date",
            )]));
        }
        let assignment = assignments::end(&mut tx, assignment, effective_to).await?;
        tx.commit().await?;
        Ok(assignment)
    }

    /// Replaces the claim types an assignment opens and the employees it
    /// covers, together (`people.claims.manage`). Every claim type must be
    /// one of the company's (`ClaimTypeNotFound`) and every employee a
    /// employee of the company at any status (`EmployeeNotFound`); at most
    /// [`MEMBER_LIMIT`] of each. Whether an employee is working is judged
    /// again when a claim is submitted.
    pub async fn set_assignment_members(
        &self,
        scope: &Scope,
        company_id: i64,
        assignment_id: i64,
        claim_type_ids: &[i64],
        employee_ids: &[i64],
    ) -> Result<AssignmentMembers> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, MANAGE).await?;
        let claim_type_ids = unique_ids(claim_type_ids).ok_or(ClaimsError::ClaimTypeNotFound)?;
        let employee_ids = unique_ids(employee_ids).ok_or(ClaimsError::EmployeeNotFound)?;
        if !employee_ids.is_empty() {
            let found = self
                .workforce
                .employees_at_any_status(scope, company_id, Some(&employee_ids))
                .await?
                .value
                .unwrap_or_default();
            if found.len() != employee_ids.len() {
                return Err(ClaimsError::EmployeeNotFound);
            }
        }
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.claims.set_assignment_members"),
            )
            .await?;
        writable(tx.sql(), scope, company_id).await?;
        authorization::authorize(tx.sql(), scope, company_id, MANAGE).await?;
        let assignment = assignments::lock(tx.sql(), scope.tenant(), company_id, assignment_id)
            .await?
            .ok_or(ClaimsError::NotFound)?;
        if !catalog::all_claim_types_in_company(
            tx.sql(),
            scope.tenant(),
            company_id,
            &claim_type_ids,
        )
        .await?
        {
            return Err(ClaimsError::ClaimTypeNotFound);
        }
        let (members, changed) =
            assignments::replace_members(&mut tx, &assignment, claim_type_ids, employee_ids)
                .await?;
        if changed {
            tx.commit().await?;
        } else {
            tx.rollback().await?;
        }
        Ok(members)
    }

    /// Employees of the company an assignment may cover, by employee number
    /// (`people.claims.manage`), at every employment status, as Workforce
    /// names them.
    pub async fn assignable_employees(
        &self,
        scope: &Scope,
        company_id: i64,
    ) -> Result<Vec<AssignableEmployee>> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, MANAGE).await?;
        let employees = self
            .workforce
            .employees_at_any_status(scope, company_id, None)
            .await?
            .value
            .unwrap_or_default();
        let mut employees: Vec<AssignableEmployee> = employees
            .into_iter()
            .map(|employee| AssignableEmployee {
                id: employee.reference.stable_id.parse().unwrap_or_default(),
                employee_number: employee.employee_number,
                name: employee.display_name,
            })
            .collect();
        employees.sort_by(|a, b| {
            a.employee_number
                .cmp(&b.employee_number)
                .then(a.id.cmp(&b.id))
        });
        Ok(employees)
    }

    // Company, settings and self

    async fn company_currencies(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company_id: i64,
    ) -> Result<Vec<String>> {
        let resolved = self
            .settings
            .get(
                client,
                scope,
                Target::Company(OrganisationId::new(company_id)?),
                CURRENCIES_KEY,
            )
            .await?;
        Ok(resolved
            .value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|code| code.as_str().map(str::to_owned))
            .collect())
    }

    /// Today in the company's time zone (`localization.timezone`; UTC when
    /// unset or unknown), as Bilimbi's `company_today/1`.
    async fn company_today(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company_id: i64,
    ) -> Result<Date> {
        let zone = match self
            .settings
            .get(
                client,
                scope,
                Target::Company(OrganisationId::new(company_id)?),
                bilibi_datetime::TIMEZONE_KEY,
            )
            .await
        {
            Ok(resolved) => resolved
                .value
                .as_str()
                .filter(|zone| bilibi_datetime::valid_timezone(zone))
                .map(str::to_owned),
            Err(Error::Invalid) => None,
            Err(error) => return Err(error.into()),
        };
        let zone = zone
            .and_then(|zone| TimeZone::get(&zone).ok())
            .unwrap_or(TimeZone::UTC);
        Ok(Timestamp::now().to_zoned(zone).date())
    }

    /// Authorizes `people.claims.submit` for the company and resolves the
    /// actor's own working employee there, now.
    async fn self_employee(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company_id: i64,
    ) -> Result<SelfEmployee> {
        authorization::authorize(client, scope, company_id, SUBMIT).await?;
        let employee_id = self
            .workforce
            .authorize_self(scope, company_id, SUBMIT)
            .await?
            .employee_id;
        match self
            .workforce
            .employee(scope, company_id, employee_id)
            .await
        {
            Ok(read) => match read.require_current() {
                Ok(employee) => Ok(SelfEmployee {
                    id: employee_id,
                    display_name: employee.display_name,
                    employee_number: employee.employee_number,
                }),
                Err(_) => Err(ClaimsError::NotLinked),
            },
            Err(WorkforceError::NotFound) => Err(ClaimsError::NotLinked),
            Err(error) => Err(error.into()),
        }
    }
}

/// A company the scope can see, or `NotFound` (missing, deleted, another
/// tenant's and malformed ids are indistinguishable).
async fn company(
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company_id: i64,
) -> Result<organisation::Identity> {
    organisation::identity(client, scope, company_id)
        .await?
        .ok_or(ClaimsError::NotFound)
}

/// Holds the company's writable lock for the rest of the transaction: an
/// archived company is `CompanyArchived`, and the lock serializes the
/// company's writes.
async fn writable(client: &impl GenericClient, scope: &Scope, company_id: i64) -> Result<()> {
    organisation::require_writable(client, scope, company_id, true)
        .await
        .map(|_| ())
        .map_err(|error| match error {
            WriteError::NotFound => ClaimsError::NotFound,
            WriteError::Archived => ClaimsError::CompanyArchived,
            WriteError::Access(error) => error.into(),
            _ => ClaimsError::Unauthorized,
        })
}

/// At most [`MAX_CURRENCIES`] codes, trimmed and uppercased, each three
/// ASCII letters, each once (Bilimbi `normalize_currencies/1`).
fn normalize_currencies(codes: &[String]) -> Result<Vec<String>> {
    if codes.len() > MAX_CURRENCIES {
        return Err(ClaimsError::InvalidCurrencies);
    }
    let mut normalized = Vec::with_capacity(codes.len());
    for code in codes {
        let code = code.trim().to_ascii_uppercase();
        if code.len() != 3 || !code.bytes().all(|byte| byte.is_ascii_uppercase()) {
            return Err(ClaimsError::InvalidCurrencies);
        }
        if !normalized.contains(&code) {
            normalized.push(code);
        }
    }
    Ok(normalized)
}

/// Whether a code has the ISO 4217 shape.
pub(crate) fn currency_shaped(code: &str) -> bool {
    code.len() == 3 && code.bytes().all(|byte| byte.is_ascii_uppercase())
}

/// At most [`MEMBER_LIMIT`] positive ids, each once; `None` otherwise
/// (Bilimbi `normalize_ids/2`).
fn unique_ids(ids: &[i64]) -> Option<Vec<i64>> {
    if ids.len() > MEMBER_LIMIT || ids.iter().any(|id| *id <= 0) {
        return None;
    }
    let mut unique = Vec::with_capacity(ids.len());
    for id in ids {
        if !unique.contains(id) {
            unique.push(*id);
        }
    }
    Some(unique)
}

/// A date column read as ISO text (the workspace's `tokio-postgres` has no
/// date feature, and a Domain must not turn one on).
pub(crate) fn date_column(row: &tokio_postgres::Row, column: &str) -> Result<Option<Date>> {
    row.get::<_, Option<String>>(column)
        .map(|text| text.parse::<Date>().map_err(|_| Error::Internal))
        .transpose()
        .map_err(ClaimsError::Failed)
}

/// A `NOT NULL` date column read as ISO text.
pub(crate) fn required_date(row: &tokio_postgres::Row, column: &str) -> Result<Date> {
    date_column(row, column)?.ok_or(ClaimsError::Failed(Error::Internal))
}

/// Bilimbi's changeset words for a required text of bounded length, trimmed.
pub(crate) fn text_field(
    errors: &mut Vec<FieldError>,
    field: &'static str,
    value: &str,
    max: usize,
) -> String {
    let value = value.trim().to_owned();
    if value.is_empty() {
        errors.push(FieldError::new(field, "can't be blank"));
    } else if value.chars().count() > max {
        errors.push(FieldError::new(
            field,
            format!("should be at most {max} character(s)"),
        ));
    }
    value
}

/// An end on or after the start, in Bilimbi's words.
pub(crate) fn period_field(errors: &mut Vec<FieldError>, from: Date, to: Option<Date>) {
    if to.is_some_and(|to| to < from) {
        errors.push(FieldError::new(
            "effective_to",
            "must not be before the start date",
        ));
    }
}

/// Where the mutation boundary's `MutationTransaction` type is named by the
/// private modules.
pub(crate) type Tx<'a> = MutationTransaction<'a>;
