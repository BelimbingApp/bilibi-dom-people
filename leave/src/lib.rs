//! Leave (`people/leave`, Bilimbi `Bilimbi.People.Leave`): a company's leave
//! types, effective-dated entitlement policies, an append-only balance
//! ledger, self-service requests with approval, and ordered year-end
//! carry-forward. The three pages follow in PL3 and PL4.
//!
//! This is the module's one facade. Other People modules and pages reach it
//! through [`Leave`], resolved from the registry as [`SERVICE`]; the schema,
//! SQL and validation stay private. Every function takes the sealed
//! [`Scope`] it was handed and an explicit platform company id. Company and
//! employee identity come from `people/workforce` and must be current: a
//! stale or unavailable workforce read is [`LeaveError::NotCurrent`].
//! Missing, sibling-company and cross-tenant records are indistinguishable
//! (`NotFound`).
//!
//! Every write authorizes the scope's signed-in actor for its self-service,
//! approval or policy capability when it runs, through the Workforce
//! `Authorization` helpers, then holds the company's writable lock inside
//! its mutation transaction, so a revoked grant refuses the next call, an
//! archived company refuses every write and a company's leave writes are
//! serialized. Refusals are typed [`LeaveError`] variants carrying Bilimbi's
//! refusal atoms, never a fatal transport error.
//!
//! See `README.md`.
mod agent;
mod carry_forward;
mod contributions;
mod entries;
mod input;
mod ledger;
mod policies;
mod requests;
mod types;
mod worker;

pub use bilibi_people_workforce::{FieldError, Freshness};
pub use carry_forward::{CarryForwardCounts, CarryForwardSkip, CarryForwardSkipped};
pub use contributions::{
    CAPABILITIES, POLICIES_MANAGE, REQUESTS_APPROVE, REQUEST_BACKDATE_DAYS_KEY, SELF_VIEW,
    WORKING_WEEKDAYS_KEY, YEAR_START_MONTH_KEY,
};
pub use requests::{Decision, NewRequest, PendingRequest, Request, SelfSummary};

use bilibi_company::{organisation, WriteError};
use bilibi_database::{Db, Error, MutationBoundary, MutationTransaction};
use bilibi_party_ids::OrganisationId;
use bilibi_people_workforce::{authorization, Company, Employee, Workforce, WorkforceError};
use bilibi_settings::{Settings, Target};
use bilibi_tenancy::Scope;
use jiff::{civil::Date, ToSpan};
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::json;
use std::{collections::HashMap, sync::Arc};
use tokio_postgres::GenericClient;

/// The module ID, Bilimbi's, kept because IDs are persisted data.
pub const MODULE: &str = "people/leave";
/// The registry service ID of the [`Leave`] facade.
pub const SERVICE: &str = "people/leave.leave";
/// The units a leave type counts in.
pub const UNITS: [&str; 2] = ["day", "hour"];
/// The two states of a leave type.
pub const TYPE_STATUSES: [&str; 2] = ["active", "archived"];
/// Every kind of ledger entry: grants, the two kinds `record_entry` writes,
/// what requests write (`taken`, `cancelled`) and what carry-forward writes
/// (`carried_forward` into the next year, `expired` into the closed one).
pub const ENTRY_TYPES: [&str; 7] = [
    "entitlement",
    "opening",
    "adjustment",
    "taken",
    "cancelled",
    "carried_forward",
    "expired",
];
/// The leave years a grant may name.
pub const GRANT_YEARS: std::ops::RangeInclusive<i32> = 1900..=9998;

pub type Result<T> = std::result::Result<T, LeaveError>;

/// Why a call did not happen: Bilimbi's refusal atoms, each displayed in the
/// words its Policies page shows.
#[derive(Debug, thiserror::Error)]
pub enum LeaveError {
    /// A denied or unknown capability, or a company outside the actor's reach.
    #[error("Access denied")]
    Unauthorized,
    /// A company, employee, type or policy the scope cannot see: missing,
    /// archived, another company's or another tenant's (indistinguishable
    /// on purpose).
    #[error("Not found")]
    NotFound,
    #[error("This company is archived and read-only")]
    CompanyArchived,
    /// The workforce read the operation needs is stale or unavailable.
    #[error("The workforce read is not current")]
    NotCurrent(Freshness),
    /// The leave year start month cannot change once the ledger has entries.
    #[error("The leave year cannot change after balances have been recorded.")]
    YearInUse,
    /// A month outside 1 to 12.
    #[error("Choose a month for the leave year.")]
    InvalidRules,
    /// No working weekday, a weekday outside 1 to 7, or backdating outside 0 to 366.
    #[error("Choose at least one working weekday and a backdating limit of 0 to 366 days.")]
    InvalidRequestRules,
    /// A type status other than `active` or `archived`.
    #[error("The leave type could not be updated.")]
    InvalidStatus,
    /// Refused type values, in the words a form shows beside each field.
    #[error("Enter a unique lowercase code, a name and a unit for the leave type.")]
    Invalid(Vec<FieldError>),
    /// An entitlement or cap outside 0 to 9999.99 or finer than hundredths.
    #[error("Choose an active leave type, a start date, an entitlement of 0 to 9999.99 and an optional carry-forward cap in the same range.")]
    InvalidPolicy,
    #[error("A new version must start after the latest version.")]
    NotAfterLatest,
    #[error("Entitlements were already granted on or after that date; start the version later.")]
    GrantedAfterEffectiveDate,
    /// A leave year outside [`GRANT_YEARS`].
    #[error("Entitlements could not be granted for that year.")]
    InvalidYear,
    /// An entry that is not `opening` or `adjustment`, a reserved or
    /// malformed source or key, a zero or unbounded quantity, or a long note.
    #[error("Invalid entry")]
    InvalidEntry,
    /// A replay under an existing source and key with different facts.
    #[error("Another entry already uses that source and key")]
    EntryKeyConflict,
    /// The leave year is already carried forward for that employee and type.
    #[error("That leave year is closed for the employee and type")]
    YearClosed,
    #[error("Your account is not linked to a working employee in this company.")]
    NotLinked,
    #[error("Choose an active leave type and valid dates; half days and hours cover a single date, and hours apply only to hour-based types.")]
    InvalidRequest,
    #[error("Another request already uses that key.")]
    RequestKeyConflict,
    #[error("A request must fall within one leave year.")]
    SpansLeaveYears,
    #[error("The start date is further back than requests may start.")]
    TooFarBack,
    #[error("The chosen dates contain no working days.")]
    NoWorkingDays,
    #[error("Another pending or approved request already covers part of these dates.")]
    OverlappingRequest,
    #[error("The available balance does not cover this request.")]
    InsufficientBalance,
    #[error("Choose approve or reject and a note of at most 500 characters.")]
    InvalidDecision,
    #[error("Give a reason when rejecting a request.")]
    NoteRequired,
    #[error("That request has already been decided.")]
    NotPending,
    #[error("You cannot decide your own leave request.")]
    SelfApproval,
    #[error("This request can no longer be cancelled.")]
    NotCancellable,
    #[error("The employee is no longer available.")]
    EmployeeUnavailable,
    #[error("The leave type is no longer available.")]
    LeaveTypeUnavailable,
    #[error("The leave year has not ended yet.")]
    YearNotEnded,
    #[error("Invalid carry-forward")]
    InvalidCarryForward,
    #[error("Leave is unavailable.")]
    Unavailable,
    /// A database or capture failure; never a refusal.
    #[error(transparent)]
    Failed(Error),
}
impl From<Error> for LeaveError {
    fn from(error: Error) -> Self {
        match error {
            Error::Forbidden | Error::Unauthorized => Self::Unauthorized,
            Error::NotFound => Self::NotFound,
            other => Self::Failed(other),
        }
    }
}
impl From<WorkforceError> for LeaveError {
    fn from(error: WorkforceError) -> Self {
        match error {
            WorkforceError::Unauthorized => Self::Unauthorized,
            WorkforceError::NotFound | WorkforceError::InvalidOptions => Self::NotFound,
            WorkforceError::NotLinked => Self::NotLinked,
            WorkforceError::CompanyArchived => Self::CompanyArchived,
            WorkforceError::Failed(error) => Self::Failed(error),
            // Refusals of employment writes, which this module never makes.
            _ => Self::Failed(Error::Internal),
        }
    }
}
impl From<WriteError> for LeaveError {
    fn from(error: WriteError) -> Self {
        match error {
            WriteError::NotFound => Self::NotFound,
            WriteError::Archived => Self::CompanyArchived,
            WriteError::Access(error) => error.into(),
            _ => Self::Unauthorized,
        }
    }
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

/// A company's leave type (Bilimbi `type_view/1`): a unique lowercase code,
/// a name, a unit (`day` or `hour`), whether it is paid, whether requests
/// need an available balance, and `active` or `archived`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LeaveType {
    pub id: i64,
    pub code: String,
    pub name: String,
    pub unit: String,
    pub paid: bool,
    pub balance_required: bool,
    pub status: String,
}

/// What creates a leave type; it starts `active`.
#[derive(Clone, Debug)]
pub struct NewLeaveType {
    pub code: String,
    pub name: String,
    pub unit: String,
    pub paid: bool,
    pub balance_required: bool,
}
impl NewLeaveType {
    pub fn new(code: impl Into<String>, name: impl Into<String>, unit: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            name: name.into(),
            unit: unit.into(),
            paid: true,
            balance_required: true,
        }
    }
}

/// One effective-dated entitlement version of a type (Bilimbi
/// `policy_view/1`). `effective_to` is open for the latest version.
/// `carry_forward_cap` is the most of a closing balance carried into the
/// next leave year; `None` means the type does not carry forward.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Policy {
    pub id: i64,
    pub leave_type_id: i64,
    pub version: i32,
    #[serde(serialize_with = "as_text")]
    pub effective_from: Date,
    #[serde(serialize_with = "as_optional_text")]
    pub effective_to: Option<Date>,
    #[serde(serialize_with = "as_text")]
    pub entitlement: Decimal,
    #[serde(serialize_with = "as_optional_text")]
    pub carry_forward_cap: Option<Decimal>,
}

/// What adds a policy version: its first effective day, the entitlement per
/// leave year in the type's unit (0 to 9999.99, hundredths) and the
/// optional carry-forward cap in the same range.
#[derive(Clone, Debug)]
pub struct NewPolicy {
    pub effective_from: Date,
    pub entitlement: Decimal,
    pub carry_forward_cap: Option<Decimal>,
}

/// One movement of an employee's balance (Bilimbi `entry_view/1`), in the
/// type's unit with two decimals; `entry_type` is one of [`ENTRY_TYPES`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LedgerEntry {
    pub id: i64,
    pub employee_id: i64,
    pub leave_type_id: i64,
    pub leave_year: i32,
    pub entry_type: String,
    #[serde(serialize_with = "as_text")]
    pub quantity: Decimal,
    pub unit: String,
    pub policy_version: Option<i32>,
    #[serde(serialize_with = "as_text")]
    pub occurred_on: Date,
    pub source: String,
    pub entry_key: String,
    pub note: Option<String>,
}

/// What records an opening balance or adjustment: the type, `opening` or
/// `adjustment`, a non-zero quantity in hundredths of the type's unit, the
/// date (which fixes the leave year), a source of 1 to 32 bytes that is not
/// one of the module's own (`policy`, `request`, `carry_forward`), a key of
/// 1 to 160 bytes unique under the company and source, and an optional note
/// of at most 500 characters.
#[derive(Clone, Debug)]
pub struct NewEntry {
    pub leave_type_id: i64,
    pub entry_type: String,
    pub quantity: Decimal,
    pub occurred_on: Date,
    pub source: String,
    pub entry_key: String,
    pub note: Option<String>,
}

/// The per-type totals of one employee and leave year (Bilimbi
/// `balance_view/3`). `taken` and `expired` are positive quantities;
/// `balance` is the sum of every entry; `pending` is the quantity pending
/// requests hold and `available` the balance less that reservation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Balance {
    pub leave_type: LeaveType,
    #[serde(serialize_with = "as_text")]
    pub entitlement: Decimal,
    #[serde(serialize_with = "as_text")]
    pub opening: Decimal,
    #[serde(serialize_with = "as_text")]
    pub adjustment: Decimal,
    #[serde(serialize_with = "as_text")]
    pub carried_forward: Decimal,
    #[serde(serialize_with = "as_text")]
    pub taken: Decimal,
    #[serde(serialize_with = "as_text")]
    pub expired: Decimal,
    #[serde(serialize_with = "as_text")]
    pub balance: Decimal,
    #[serde(serialize_with = "as_text")]
    pub pending: Decimal,
    #[serde(serialize_with = "as_text")]
    pub available: Decimal,
}

/// What a grant run did: entitlements granted now, employee-type-years
/// already granted, and employee-type-years skipped because the year is
/// already carried forward.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct GrantCounts {
    pub granted: usize,
    pub existing: usize,
    pub closed: usize,
}

/// The company's leave year: the month it starts in
/// (`people.leave.year_start_month`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Rules {
    pub year_start_month: i32,
}
impl Rules {
    /// The leave year containing `date`, labelled by the calendar year it
    /// starts in (Bilimbi `leave_year/2`).
    pub fn leave_year(&self, date: Date) -> i32 {
        if i32::from(date.month()) >= self.year_start_month {
            i32::from(date.year())
        } else {
            i32::from(date.year()) - 1
        }
    }
    /// The first and last day of a leave year (Bilimbi `year_range/2`); a
    /// year the calendar cannot hold is `InvalidYear`.
    pub fn year_range(&self, year: i32) -> Result<(Date, Date)> {
        let first = i16::try_from(year)
            .ok()
            .and_then(|year| Date::new(year, self.year_start_month.clamp(1, 12) as i8, 1).ok())
            .ok_or(LeaveError::InvalidYear)?;
        let last = first
            .checked_add(1.year())
            .and_then(|date| date.yesterday())
            .map_err(|_| LeaveError::InvalidYear)?;
        Ok((first, last))
    }
}

/// The company's request rules (`people.leave.working_weekdays`,
/// `people.leave.request_backdate_days`): ISO weekday numbers counted as
/// leave days, and how many days before today a request may start.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RequestRules {
    pub working_weekdays: Vec<i32>,
    pub backdate_days: i32,
}

/// The facade, resolved from the registry as [`SERVICE`].
pub struct Leave {
    db: Db,
    mutations: Arc<MutationBoundary>,
    settings: Arc<Settings>,
    workforce: Arc<Workforce>,
    reference_data: Arc<bilibi_people_reference_data::ReferenceData>,
    queue: Arc<bilibi_queue::Queue>,
}

impl Leave {
    // ── Company leave year and request rules ──────────────────────────────

    /// The company's leave year rules (Bilimbi `rules/2`); no capability.
    pub async fn rules(&self, scope: &Scope, company_id: i64) -> Result<Rules> {
        let client = self.db.get().await.map_err(Error::from)?;
        let company = self.current_company(scope, company_id).await?;
        self.read_rules(&**client, scope, &company).await
    }

    /// Sets the month the leave year begins (`people.leave.policies.manage`).
    /// Refused as `YearInUse` once the company has ledger entries, which
    /// carry their leave year; a month outside 1 to 12 is `InvalidRules`.
    pub async fn put_rules(&self, scope: &Scope, company_id: i64, month: i32) -> Result<Rules> {
        if !(1..=12).contains(&month) {
            return Err(LeaveError::InvalidRules);
        }
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, POLICIES_MANAGE).await?;
        let company = self.current_company(scope, company_id).await?;
        let mut tx = self
            .begin(&mut client, scope, company_id, "people.leave.put_rules")
            .await?;
        let tenant = scope.tenant();
        types::lock_all(tx.sql(), tenant, company_id).await?;
        let current = self.read_rules(tx.sql(), scope, &company).await?;
        if current.year_start_month == month {
            tx.rollback().await?;
            return Ok(current);
        }
        if ledger::in_use(tx.sql(), tenant, company_id).await? {
            return Err(LeaveError::YearInUse);
        }
        self.settings
            .put(
                &mut tx,
                scope,
                target(company_id)?,
                YEAR_START_MONTH_KEY,
                json!(month),
            )
            .await?;
        tx.commit().await?;
        Ok(Rules {
            year_start_month: month,
        })
    }

    /// The company's request rules (Bilimbi `Requests.request_rules/2`);
    /// no capability.
    pub async fn request_rules(&self, scope: &Scope, company_id: i64) -> Result<RequestRules> {
        let client = self.db.get().await.map_err(Error::from)?;
        let company = self.current_company(scope, company_id).await?;
        self.read_request_rules(&**client, scope, &company).await
    }

    /// Saves the request rules (`people.leave.policies.manage`): one or more
    /// ISO weekdays 1 to 7, stored distinct and sorted, and a backdating
    /// limit of 0 to 366 days; anything else is `InvalidRequestRules`.
    pub async fn put_request_rules(
        &self,
        scope: &Scope,
        company_id: i64,
        rules: &RequestRules,
    ) -> Result<RequestRules> {
        if rules.working_weekdays.is_empty()
            || !rules
                .working_weekdays
                .iter()
                .all(|day| (1..=7).contains(day))
            || !(0..=366).contains(&rules.backdate_days)
        {
            return Err(LeaveError::InvalidRequestRules);
        }
        let mut weekdays = rules.working_weekdays.clone();
        weekdays.sort_unstable();
        weekdays.dedup();
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, POLICIES_MANAGE).await?;
        self.current_company(scope, company_id).await?;
        let mut tx = self
            .begin(
                &mut client,
                scope,
                company_id,
                "people.leave.put_request_rules",
            )
            .await?;
        let target = target(company_id)?;
        self.settings
            .put(
                &mut tx,
                scope,
                target,
                WORKING_WEEKDAYS_KEY,
                json!(weekdays),
            )
            .await?;
        self.settings
            .put(
                &mut tx,
                scope,
                target,
                REQUEST_BACKDATE_DAYS_KEY,
                json!(rules.backdate_days),
            )
            .await?;
        tx.commit().await?;
        Ok(RequestRules {
            working_weekdays: weekdays,
            backdate_days: rules.backdate_days,
        })
    }

    // ── Types ─────────────────────────────────────────────────────────────

    /// The company's types, active first then by name (Bilimbi
    /// `list_types/2`); no capability.
    pub async fn list_types(&self, scope: &Scope, company_id: i64) -> Result<Vec<LeaveType>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.current_company(scope, company_id).await?;
        types::list(&**client, scope.tenant(), company_id).await
    }

    /// Adds an active type (`people.leave.policies.manage`). Refused values
    /// are `Invalid` field errors: a blank, overlong or malformed code, a
    /// code the company already uses, a blank or overlong name, an unknown
    /// unit.
    pub async fn create_type(
        &self,
        scope: &Scope,
        company_id: i64,
        new: &NewLeaveType,
    ) -> Result<LeaveType> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, POLICIES_MANAGE).await?;
        self.current_company(scope, company_id).await?;
        let valid = types::validate(new).map_err(LeaveError::Invalid)?;
        let mut tx = self
            .begin(&mut client, scope, company_id, "people.leave.create_type")
            .await?;
        if types::code_taken(tx.sql(), company_id, &valid.code).await? {
            return Err(LeaveError::Invalid(vec![FieldError::new(
                "code",
                "has already been taken",
            )]));
        }
        let leave_type = types::insert(&mut tx, scope.tenant(), company_id, valid).await?;
        tx.commit().await?;
        Ok(leave_type)
    }

    /// Archives or reactivates a type (`people.leave.policies.manage`). An
    /// archived type takes no new policy versions, grants or entries but
    /// keeps its history and balances. A status other than `active` or
    /// `archived` is `InvalidStatus`; a type of another company is `NotFound`.
    pub async fn set_type_status(
        &self,
        scope: &Scope,
        company_id: i64,
        type_id: i64,
        status: &str,
    ) -> Result<LeaveType> {
        if !types::status_valid(status) {
            return Err(LeaveError::InvalidStatus);
        }
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, POLICIES_MANAGE).await?;
        self.current_company(scope, company_id).await?;
        let mut tx = self
            .begin(
                &mut client,
                scope,
                company_id,
                "people.leave.set_type_status",
            )
            .await?;
        let before = types::lock(tx.sql(), scope.tenant(), company_id, type_id)
            .await?
            .ok_or(LeaveError::NotFound)?;
        let after = types::set_status(&mut tx, before, status.to_owned()).await?;
        tx.commit().await?;
        Ok(after)
    }

    // ── Policies ──────────────────────────────────────────────────────────

    /// Every policy version of the company, by type and newest version
    /// first (Bilimbi `list_policies/2`); no capability.
    pub async fn list_policies(&self, scope: &Scope, company_id: i64) -> Result<Vec<Policy>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.current_company(scope, company_id).await?;
        policies::list(&**client, scope.tenant(), company_id).await
    }

    /// Adds the next policy version of an active type
    /// (`people.leave.policies.manage`). It must start after the latest
    /// version, which it closes the day before (`NotAfterLatest`), and after
    /// every entitlement already granted for that type
    /// (`GrantedAfterEffectiveDate`), so no grant is left under a policy that
    /// no longer covers it. Versions are never edited. An entitlement or cap
    /// outside 0 to 9999.99, or finer than hundredths, is `InvalidPolicy`; an
    /// archived or missing type is `NotFound`.
    pub async fn add_policy(
        &self,
        scope: &Scope,
        company_id: i64,
        type_id: i64,
        new: &NewPolicy,
    ) -> Result<Policy> {
        let entitlement =
            input::non_negative_quantity(new.entitlement).ok_or(LeaveError::InvalidPolicy)?;
        let carry_forward_cap = new
            .carry_forward_cap
            .map(|cap| input::non_negative_quantity(cap).ok_or(LeaveError::InvalidPolicy))
            .transpose()?;
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, POLICIES_MANAGE).await?;
        self.current_company(scope, company_id).await?;
        let mut tx = self
            .begin(&mut client, scope, company_id, "people.leave.add_policy")
            .await?;
        let tenant = scope.tenant();
        let leave_type = types::lock_active(tx.sql(), tenant, company_id, type_id)
            .await?
            .ok_or(LeaveError::NotFound)?;
        let latest = policies::lock_latest(tx.sql(), tenant, leave_type.id).await?;
        if latest
            .as_ref()
            .is_some_and(|latest| new.effective_from <= latest.effective_from)
        {
            return Err(LeaveError::NotAfterLatest);
        }
        if ledger::granted_on_or_after(
            tx.sql(),
            tenant,
            company_id,
            leave_type.id,
            new.effective_from,
        )
        .await?
        {
            return Err(LeaveError::GrantedAfterEffectiveDate);
        }
        let policy = policies::insert(
            &mut tx,
            tenant,
            company_id,
            leave_type.id,
            latest,
            NewPolicy {
                effective_from: new.effective_from,
                entitlement,
                carry_forward_cap,
            },
            scope.actor().user(),
        )
        .await?;
        tx.commit().await?;
        Ok(policy)
    }

    /// The type's policy version in force on `date` (Bilimbi `policy_on/4`);
    /// none is `NotFound`. No capability.
    pub async fn policy_on(
        &self,
        scope: &Scope,
        company_id: i64,
        type_id: i64,
        date: Date,
    ) -> Result<Policy> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.current_company(scope, company_id).await?;
        policies::effective(&**client, scope.tenant(), company_id, type_id, date)
            .await?
            .ok_or(LeaveError::NotFound)
    }

    // ── Ledger ────────────────────────────────────────────────────────────

    /// Grants every current workforce employee the entitlement of every
    /// active type whose policy is in force on the first day of `leave_year`
    /// (`people.leave.policies.manage`), from that version and recording it.
    /// A grant happens once per employee, type and year: a repeated run
    /// counts the existing grants, and an employee and type whose year is
    /// already carried forward is counted as `closed` and not granted. A
    /// year outside [`GRANT_YEARS`] is `InvalidYear`; a workforce read that
    /// is not current is `NotCurrent`.
    pub async fn grant_entitlements(
        &self,
        scope: &Scope,
        company_id: i64,
        leave_year: i32,
    ) -> Result<GrantCounts> {
        if !GRANT_YEARS.contains(&leave_year) {
            return Err(LeaveError::InvalidYear);
        }
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, POLICIES_MANAGE).await?;
        let company = self.current_company(scope, company_id).await?;
        let employees = self
            .workforce
            .employees(scope, company_id)
            .await?
            .require_current()
            .map_err(|not_current| LeaveError::NotCurrent(not_current.0))?;
        let employee_ids = employee_ids(&employees)?;
        let mut tx = self
            .begin(
                &mut client,
                scope,
                company_id,
                "people.leave.grant_entitlements",
            )
            .await?;
        let tenant = scope.tenant();
        let active: Vec<LeaveType> = types::lock_all(tx.sql(), tenant, company_id)
            .await?
            .into_iter()
            .filter(|leave_type| leave_type.status == "active")
            .collect();
        let rules = self.read_rules(tx.sql(), scope, &company).await?;
        let (first_day, _) = rules.year_range(leave_year)?;
        let mut counts = GrantCounts::default();
        let mut planned = Vec::new();
        for leave_type in &active {
            let Some(policy) =
                policies::effective(tx.sql(), tenant, company_id, leave_type.id, first_day).await?
            else {
                continue;
            };
            for &employee_id in &employee_ids {
                let key = format!("entitlement:{}:{employee_id}:{leave_year}", leave_type.id);
                if ledger::find(tx.sql(), tenant, company_id, ledger::GRANT_SOURCE, &key)
                    .await?
                    .is_some()
                {
                    counts.existing += 1;
                } else if ledger::closed(
                    tx.sql(),
                    tenant,
                    company_id,
                    employee_id,
                    leave_type.id,
                    leave_year,
                )
                .await?
                {
                    counts.closed += 1;
                } else {
                    planned.push(ledger::NewRow {
                        employee_id,
                        leave_type_id: leave_type.id,
                        leave_year,
                        entry_type: "entitlement".into(),
                        quantity: policy.entitlement,
                        unit: leave_type.unit.clone(),
                        policy_id: Some(policy.id),
                        policy_version: Some(policy.version),
                        occurred_on: first_day,
                        source: ledger::GRANT_SOURCE.into(),
                        entry_key: key,
                        actor_user_id: scope.actor().user(),
                        note: None,
                    });
                }
            }
        }
        if planned.is_empty() {
            tx.rollback().await?;
            return Ok(counts);
        }
        let wanted = planned.len();
        let inserted = ledger::insert(&mut tx, tenant, company_id, planned).await?;
        tx.commit().await?;
        counts.granted = inserted.len();
        counts.existing += wanted - inserted.len();
        Ok(counts)
    }

    /// Records an opening balance or adjustment for one working employee
    /// (`people.leave.policies.manage`; Bilimbi `record_entry/4`), in the
    /// leave year of its date. Idempotent by company, source and key: a
    /// replay with the same facts returns the entry, one with different
    /// facts is `EntryKeyConflict`. A year already carried forward for that
    /// employee and type is `YearClosed`; an archived or missing type is
    /// `NotFound`; malformed input is `InvalidEntry`.
    pub async fn record_entry(
        &self,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
        new: &NewEntry,
    ) -> Result<LedgerEntry> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, POLICIES_MANAGE).await?;
        let mut tx = self
            .begin(&mut client, scope, company_id, "people.leave.record_entry")
            .await?;
        let entry = self
            .record_entry_in(&mut tx, scope, company_id, employee_id, new)
            .await?;
        if entry.created {
            tx.commit().await?;
        } else {
            tx.rollback().await?;
        }
        Ok(entry.entry)
    }

    /// Per-type totals of one working employee and leave year, derived from
    /// the ledger (Bilimbi `balances/4`): every active type, and every
    /// archived type with entries. `pending` is the quantity held by the
    /// employee's pending requests of the year and `available` the balance
    /// less that reservation. No capability; an employee who is not a
    /// working employee of the company is `NotFound`.
    pub async fn balances(
        &self,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
        leave_year: i32,
    ) -> Result<Vec<Balance>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.current_employee(scope, company_id, employee_id)
            .await?;
        let tenant = scope.tenant();
        let totals = ledger::totals(&**client, tenant, company_id, employee_id, leave_year).await?;
        let pending =
            ledger::pending_totals(&**client, tenant, company_id, employee_id, leave_year).await?;
        Ok(types::by_name(&**client, tenant, company_id)
            .await?
            .into_iter()
            .filter(|leave_type| {
                leave_type.status == "active" || totals.contains_key(&leave_type.id)
            })
            .map(|leave_type| {
                let sums = totals.get(&leave_type.id);
                let pending = pending.get(&leave_type.id).copied().unwrap_or_default();
                balance(leave_type, sums, pending)
            })
            .collect())
    }

    /// One working employee's entries of a leave year, latest date first
    /// (Bilimbi `entries/4`); no capability.
    pub async fn entries(
        &self,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
        leave_year: i32,
    ) -> Result<Vec<LedgerEntry>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.current_employee(scope, company_id, employee_id)
            .await?;
        ledger::list(
            &**client,
            scope.tenant(),
            company_id,
            employee_id,
            leave_year,
        )
        .await
    }

    // ── Private ───────────────────────────────────────────────────────────

    /// Begins the write's mutation transaction and holds the company's
    /// writable lock: an archived company is `CompanyArchived`, and a
    /// company's leave writes run one at a time.
    async fn begin<'a>(
        &self,
        client: &'a mut tokio_postgres::Client,
        scope: &Scope,
        company_id: i64,
        action: &str,
    ) -> Result<MutationTransaction<'a>> {
        let tx = self
            .mutations
            .begin(client, scope.mutation_context(action))
            .await?;
        entries::serialize(tx.sql(), scope, company_id).await?;
        organisation::require_writable(tx.sql(), scope, company_id, true).await?;
        Ok(tx)
    }

    async fn current_company(&self, scope: &Scope, company_id: i64) -> Result<Company> {
        self.workforce
            .company(scope, company_id)
            .await?
            .require_current()
            .map_err(|not_current| LeaveError::NotCurrent(not_current.0))
    }

    async fn current_employee(
        &self,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
    ) -> Result<Employee> {
        self.workforce
            .employee(scope, company_id, employee_id)
            .await?
            .require_current()
            .map_err(|not_current| LeaveError::NotCurrent(not_current.0))
    }

    async fn read_rules(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company: &Company,
    ) -> Result<Rules> {
        let resolved = self
            .settings
            .get(
                client,
                scope,
                target(company.platform_company_id)?,
                YEAR_START_MONTH_KEY,
            )
            .await?;
        let month = resolved
            .value
            .as_i64()
            .and_then(|month| i32::try_from(month).ok())
            .filter(|month| (1..=12).contains(month))
            // The definition bounds the value; a stored value that escaped
            // it reads as the default rather than breaking every year.
            .unwrap_or(1);
        Ok(Rules {
            year_start_month: month,
        })
    }

    async fn read_request_rules(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company: &Company,
    ) -> Result<RequestRules> {
        let target = target(company.platform_company_id)?;
        let weekdays = self
            .settings
            .get(client, scope, target, WORKING_WEEKDAYS_KEY)
            .await?;
        let backdate = self
            .settings
            .get(client, scope, target, REQUEST_BACKDATE_DAYS_KEY)
            .await?;
        let mut working_weekdays: Vec<i32> = weekdays
            .value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|day| day.as_i64().and_then(|day| i32::try_from(day).ok()))
            .filter(|day| (1..=7).contains(day))
            .collect();
        working_weekdays.sort_unstable();
        working_weekdays.dedup();
        if working_weekdays.is_empty() {
            working_weekdays = vec![1, 2, 3, 4, 5];
        }
        let backdate_days = backdate
            .value
            .as_i64()
            .and_then(|days| i32::try_from(days).ok())
            .filter(|days| (0..=366).contains(days))
            .unwrap_or(30);
        Ok(RequestRules {
            working_weekdays,
            backdate_days,
        })
    }
}

fn target(company_id: i64) -> Result<Target> {
    Ok(Target::Company(
        OrganisationId::new(company_id).map_err(|_| LeaveError::NotFound)?,
    ))
}

/// The employment ids behind a workforce read.
fn employee_ids(employees: &[Employee]) -> Result<Vec<i64>> {
    employees
        .iter()
        .map(|employee| {
            employee
                .reference
                .stable_id
                .parse::<i64>()
                .map_err(|_| LeaveError::Failed(Error::Internal))
        })
        .collect()
}

/// Bilimbi `normalize_entry/1`: the bounded, non-zero quantity and the
/// trimmed note of a valid entry.
fn validate_entry(new: &NewEntry) -> Result<(Decimal, Option<String>)> {
    let source_ok = (1..=32).contains(&new.source.len())
        && !ledger::RESERVED_SOURCES.contains(&new.source.as_str());
    let key_ok = (1..=160).contains(&new.entry_key.len());
    let type_ok = new.entry_type == "opening" || new.entry_type == "adjustment";
    if !(source_ok && key_ok && type_ok && new.leave_type_id > 0) {
        return Err(LeaveError::InvalidEntry);
    }
    let quantity = input::bounded_quantity(new.quantity)
        .filter(|quantity| !quantity.is_zero())
        .ok_or(LeaveError::InvalidEntry)?;
    let note =
        input::optional_text(new.note.as_deref(), 500).map_err(|()| LeaveError::InvalidEntry)?;
    Ok((quantity, note))
}

/// Bilimbi `balance_view/3`.
fn balance(
    leave_type: LeaveType,
    sums: Option<&HashMap<String, Decimal>>,
    pending: Decimal,
) -> Balance {
    let zero = Decimal::new(0, 2);
    let sum = |kinds: &[&str]| -> Decimal {
        kinds
            .iter()
            .filter_map(|kind| sums.and_then(|sums| sums.get(*kind)))
            .fold(zero, |total, value| total + value)
    };
    let balance = sums
        .into_iter()
        .flat_map(|sums| sums.values())
        .fold(zero, |total, value| total + value);
    // Scale 2 even when no request is pending (`0.00`, not `0`).
    let pending = pending + zero;
    Balance {
        leave_type,
        entitlement: sum(&["entitlement"]),
        opening: sum(&["opening"]),
        adjustment: sum(&["adjustment"]),
        carried_forward: sum(&["carried_forward"]),
        // Taken leave net of cancelled approvals, as a positive quantity.
        taken: zero - sum(&["taken", "cancelled"]),
        expired: zero - sum(&["expired"]),
        balance,
        pending,
        available: balance - pending,
    }
}
