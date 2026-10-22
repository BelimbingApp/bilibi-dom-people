//! Training (`people/training`, Bilimbi `Bilimbi.People.Training`): a
//! company's course catalog, delivery events and session calendar.
//!
//! This is the module's one facade. Pages and other People modules call
//! [`Training`], resolved from the registry as [`SERVICE`]; the schema, SQL
//! and validation stay private. Every operation takes the sealed [`Scope`] it
//! was handed and an explicit platform company id, authorizes through the
//! Workforce `Authorization` helpers (the capability now, the company axis
//! through Core Company), refuses an archived company and a workforce read
//! that is not current, and writes through `MutationBoundary` under the
//! company's writable lock. The sealed actor supplies the author and
//! impersonator recorded on every fact; a caller cannot supply them.
//!
//! Capacity is the most learners an event or session takes; enrolment and
//! attendance are recorded as append-only revisions with import-key replay. Session times are entered as
//! local wall clocks in an explicit IANA zone and stored as UTC instants;
//! a wall clock inside a DST gap or overlap is refused, never guessed
//! (`clock.rs`). The calendar answers sessions overlapping a half-open UTC
//! window of at most 366 days, at most 500 rows, and the company's own zone
//! (Base DateTime's `localization.timezone`) bounds its days.
//!
//! The catalog, participation, evidence and learning governance share this facade. The Courses and
//! Sessions & calendar pages (PT6) and the menu leaves land with them.
//! See `README.md`.
mod evaluation;
mod evaluation_store;
mod evaluation_types;
pub use evaluation::{
    EFFECTIVENESS_ANSWER, EFFECTIVENESS_SUMMARY_VIEW, EFFECTIVENESS_VIEW, EVALUATION_POLICY_MANAGE,
    EVALUATION_REMINDERS_MANAGE, EVALUATION_SUBMIT,
};
pub use evaluation_types::*;

mod catalog;
mod clock;
mod contributions;
mod documents;
mod download;
mod insights;
mod participation;
mod passport;
mod passport_store;
pub use insights::{InsightDrillRow, InsightPeriod, InsightRow, INSIGHTS_VIEW};
pub use passport::{
    Passport, PassportAudience, PassportDirectory, PassportEvidence, PassportRow, TrainingPage,
    PASSPORT_GENERATE, PASSPORT_LIMIT, PASSPORT_MY_VIEW, PASSPORT_TEAM_VIEW,
};

pub use participation::{
    AttendanceFact, AttendanceStatus, EmployeeOption, Evidence, NewAttendance,
};

mod governance;
mod governance_store;
mod governance_types;
pub use governance::{
    BUDGETS_MANAGE, BUDGETS_VIEW, CURRENCIES_KEY, PLANS_APPROVE, PLANS_SUBMIT, PLANS_VIEW,
    REQUESTS_APPROVE, REQUESTS_RECOMMEND, REQUESTS_REVIEW, REQUESTS_SUBMIT, REQUESTS_VIEW,
};
pub use governance_types::*;

pub use bilibi_people_workforce::{FieldError, Freshness};
pub use clock::{day_start, local_instant};
pub use contributions::{
    CAPABILITIES, COURSES_MANAGE, COURSES_VIEW, EVIDENCE_MANAGE, RECORDS_MANAGE, RECORDS_VIEW,
    RETENTION_MANAGE, SESSIONS_MANAGE, SESSIONS_VIEW,
};

use bilibi_company::{organisation, WriteError};
use bilibi_database::{Db, Error, MutationBoundary};
use bilibi_party_ids::OrganisationId;
use bilibi_people_workforce::{authorization, Workforce, WorkforceError};
use bilibi_settings::{Settings, Target};
use bilibi_tenancy::Scope;
use jiff::{civil::Date, Timestamp};
use serde::Serialize;
use std::sync::Arc;
use tokio_postgres::GenericClient;

/// The module ID, Bilimbi's, kept because IDs are persisted data.
pub const MODULE: &str = "people/training";
/// The registry service ID of the [`Training`] facade.
pub const SERVICE: &str = "people/training.training";
/// The most sessions one calendar read answers.
pub const CALENDAR_LIMIT: usize = 500;
/// The longest calendar window, in days.
pub const CALENDAR_MAX_DAYS: i64 = 366;

pub type Result<T> = std::result::Result<T, TrainingError>;

/// Why a call did not happen (Bilimbi's refusal atoms), each in the sentence
/// its pages show (`Training.Web.Support.message/1`).
#[derive(Debug, thiserror::Error)]
pub enum TrainingError {
    #[error("You cannot access or generate this document. Ask an operator to check your access and workforce connection.")]
    DocumentForbidden,
    #[error("This passport exceeds the document safety limit of 1,000 records. Review records on screen.")]
    PassportTooLarge,
    #[error("This document contains characters the available PDF renderer cannot display. The training records remain available on screen.")]
    UnsupportedPassportText,
    #[error("Choose valid dates in order, covering at most 366 days.")]
    InvalidInsightRange,
    #[error("Choose a frozen Effectiveness reporting period. Periods appear once an operator freezes them.")]
    ReportPeriodUnavailable,
    #[error("Save this company's evaluation settings on Effectiveness first.")]
    ReportNotConfigured,
    #[error("Save this company's evaluation settings on Effectiveness before publication.")]
    EvaluationPolicyNotConfigured,
    #[error("Save this company's evaluation settings on Effectiveness first.")]
    EffectivenessReportNotConfigured,
    #[error("Set distinct positive checkpoint day offsets in the evaluation settings.")]
    InvalidCheckpoints,
    #[error("Enter whole-number days, a disclosure minimum of at least 2, a reporting period that divides 12 months, and 1 to 24 distinct checkpoint days.")]
    InvalidEvaluationSettings,
    #[error("Supply unique criterion codes, labels and ordered whole-number score bounds.")]
    InvalidCriteria,
    #[error(
        "Answer every criterion within its score bounds, or leave its outcome explicitly unknown."
    )]
    InvalidAnswers,
    #[error("Choose an effective period that does not overlap a published policy.")]
    EvaluationPolicyOverlap,
    #[error("The effective end date must be on or after the start.")]
    InvalidEvaluationPeriod,
    #[error("Supply an explanation for this permanent record.")]
    EvaluationReasonRequired,
    #[error("Reviews can be prepared after this session ends.")]
    SessionNotFinished,
    #[error("Publish a policy covering the session's local completion date first.")]
    EvaluationPolicyUnavailable,
    #[error("This employee's latest attendance is not confirmed. Check the session record.")]
    AttendanceUnavailable,
    #[error("This review already has a permanent answer.")]
    AlreadyAnswered,
    #[error("This employee is outside your current direct-report team.")]
    OutsideDirectTeam,

    #[error("Enter currency codes as three uppercase letters, separated by commas.")]
    InvalidCurrencies,
    #[error("This currency is not enabled for the company.")]
    CurrencyUnavailable,
    #[error("A budget policy already covers part of that period for this currency.")]
    OverlappingPolicy,
    #[error("Another correction has already replaced this budget policy.")]
    SupersededBudget,
    #[error("Approved requests in that period already exceed this allocation. Enter an allocation at least equal to the committed amount.")]
    BudgetBelowCommitments,
    #[error("Requests approved under this policy fall outside the corrected period. Keep their proposed dates within it.")]
    CommitmentsOutsidePeriod,
    #[error("No budget policy covers this request's currency and proposed date. Ask the budget operator to allocate one.")]
    BudgetUnavailable,
    #[error("This approval exceeds the effective budget. Ask the budget operator to review the allocation.")]
    BudgetExceeded,
    #[error("This employee is outside your current reporting line.")]
    OutsideTeam,
    #[error(
        "No current employee is linked to your login. Ask an operator to check your employee link."
    )]
    EmployeeUnavailable,
    #[error("You cannot review or approve your own learning record.")]
    SelfApproval,
    #[error("Only the accountable owner can do that.")]
    NotOwner,
    #[error("Enter a reason for this decision.")]
    ReasonRequired,
    #[error("A plan needs at least one learning item.")]
    ItemsRequired,
    #[error(
        "That action is unavailable in the record's current state. Refresh and review it again."
    )]
    InvalidTransition,
    #[error("Another amendment has already replaced this plan.")]
    SupersededPlan,
    #[error("The end date must be on or after the start date.")]
    InvalidPeriod,
    #[error("A plan can link only an approved team request.")]
    RequestUnapproved,
    #[error("Check required fields, dates and amounts. Amounts must be nonnegative with up to four decimal places.")]
    InvalidLearningValues,

    /// A denied or unknown capability, or a company outside the actor's reach.
    #[error("You cannot do that for this company.")]
    Unauthorized,
    /// A company or record the scope cannot see.
    #[error("That record is not available to you.")]
    NotFound,
    /// The company is archived.
    #[error("Choose an active company.")]
    CompanyUnavailable,
    /// The workforce read behind the company is stale or unavailable.
    #[error("{}", not_current_message(.0))]
    WorkforceNotCurrent(Freshness),
    /// The event's course is missing or inactive.
    #[error("Choose an active course in this company.")]
    CourseUnavailable,
    /// The session's event is missing.
    #[error("Choose an event in this company.")]
    EventUnavailable,
    /// A session capacity above its event's.
    #[error("Session capacity cannot exceed event capacity.")]
    CapacityExceeded,
    #[error("Choose a session in this company.")]
    SessionUnavailable,
    #[error("Choose a current employee in this company.")]
    AttendanceEmployeeUnavailable,
    #[error(
        "This record key already belongs to different attendance. Use a new key for a correction."
    )]
    ImportConflict,
    #[error("Supply a session, employee, record key and reason.")]
    InvalidRecord,
    #[error("This session has no remaining attendance places. Correct an existing record or ask a session operator to review capacity.")]
    AttendanceCapacityExceeded,
    #[error("Choose a PDF evidence document.")]
    InvalidPdf,
    #[error("Ask an operator to configure private document storage in Operator Settings.")]
    ArtifactPolicy,
    #[error("That could not be completed. Refresh and try again.")]
    ArtifactFailed(bilibi_artifacts::Error),
    /// A local wall clock inside a DST overlap.
    #[error("That local time occurs twice. Choose a time outside the clock overlap.")]
    AmbiguousTime,
    /// A local wall clock inside a DST gap.
    #[error("That local time does not exist. Choose a time after the clock change.")]
    NonexistentTime,
    /// An unknown zone or an unparsable wall clock.
    #[error("Enter valid local times and an IANA time zone.")]
    InvalidTimeZoneOrTime,
    /// A session ending at or before its start.
    #[error("The end must be after the start.")]
    InvalidTimeRange,
    /// A calendar window that is empty, backwards or longer than 366 days.
    #[error("That could not be completed. Refresh and try again.")]
    InvalidCalendarRange,
    /// Refused values, in the words a form shows beside each field.
    #[error("Check the entered values. Codes must be unique and capacity must be a positive whole number.")]
    Invalid(Vec<FieldError>),
    /// A database or capture failure; never a refusal.
    #[error("That could not be completed. Refresh and try again.")]
    Failed(Error),
}

fn not_current_message(freshness: &Freshness) -> String {
    match freshness {
        Freshness::Current => "Workforce data is current.".to_owned(),
        Freshness::Stale(at) => format!(
            "Workforce data is stale; last confirmed {at}. Team access and document generation require current workforce data."
        ),
        Freshness::Unavailable(_) => {
            "Workforce data is unavailable. Ask an operator to check the connection.".to_owned()
        }
    }
}

impl From<Error> for TrainingError {
    fn from(error: Error) -> Self {
        match error {
            Error::Forbidden | Error::Unauthorized => Self::Unauthorized,
            Error::NotFound => Self::NotFound,
            other => Self::Failed(other),
        }
    }
}
impl From<WorkforceError> for TrainingError {
    fn from(error: WorkforceError) -> Self {
        match error {
            WorkforceError::Unauthorized => Self::Unauthorized,
            WorkforceError::NotFound => Self::NotFound,
            WorkforceError::CompanyArchived => Self::CompanyUnavailable,
            WorkforceError::Failed(error) => Self::Failed(error),
            _ => Self::Failed(Error::Internal),
        }
    }
}
impl From<bilibi_party_ids::InvalidId> for TrainingError {
    fn from(_: bilibi_party_ids::InvalidId) -> Self {
        Self::NotFound
    }
}

/// A course of the company's catalog (`people_training_courses`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Course {
    pub id: i64,
    pub platform_company_id: i64,
    pub code: String,
    pub name: String,
    pub description: Option<String>,
    pub active: bool,
    pub actor_user_id: i64,
    pub impersonator_id: Option<i64>,
}

/// An active course as a form offers it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CourseOption {
    pub id: i64,
    pub name: String,
}

/// A delivery of one course to a cohort of at most `capacity` learners
/// (`people_training_events`), with its course's name beside it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Event {
    pub id: i64,
    pub platform_company_id: i64,
    pub course_id: i64,
    pub course_name: String,
    pub name: String,
    pub capacity: i32,
    pub actor_user_id: i64,
    pub impersonator_id: Option<i64>,
}

/// One run of an event (`people_training_session_runs`): UTC instants and
/// the delivery zone they were entered in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Session {
    pub id: i64,
    pub platform_company_id: i64,
    pub event_id: i64,
    pub name: String,
    pub capacity: i32,
    pub time_zone: String,
    #[serde(serialize_with = "as_text")]
    pub starts_at: Timestamp,
    #[serde(serialize_with = "as_text")]
    pub ends_at: Timestamp,
    pub actor_user_id: i64,
    pub impersonator_id: Option<i64>,
}
fn as_text<S: serde::Serializer>(
    value: &Timestamp,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.collect_str(value)
}

/// What creates a course: a code (`^[a-z0-9][a-z0-9_-]*$`, at most 80
/// characters, unique in the company), a name (trimmed, 1 to 160 characters)
/// and an optional description (at most 4,000 characters; empty is none).
/// A new course is active.
#[derive(Clone, Debug)]
pub struct NewCourse {
    pub code: String,
    pub name: String,
    pub description: Option<String>,
}

/// What changes on a course: each `Some` replaces the field, `None` keeps
/// it. An empty description clears it, as Ecto's cast does. The code is
/// stable and never changes.
#[derive(Clone, Debug, Default)]
pub struct CourseUpdate {
    pub name: Option<String>,
    pub description: Option<String>,
    pub active: Option<bool>,
}

/// What creates an event: an active course of the company, a name and an
/// explicit positive whole-number capacity (no default is installed).
#[derive(Clone, Debug)]
pub struct NewEvent {
    pub course_id: i64,
    pub name: String,
    pub capacity: i64,
}

/// What creates a session: an event of the company, a name, a capacity at
/// most the event's, an IANA zone and the local start and end wall clocks in
/// it (`YYYY-MM-DDTHH:MM`, seconds optional), the end after the start.
#[derive(Clone, Debug)]
pub struct NewSession {
    pub event_id: i64,
    pub name: String,
    pub capacity: i64,
    pub time_zone: String,
    pub starts_local: String,
    pub ends_local: String,
}

/// The facade, resolved from the registry as [`SERVICE`].
pub struct Training {
    db: Db,
    mutations: Arc<MutationBoundary>,
    settings: Arc<Settings>,
    workforce: Arc<Workforce>,
    artifacts: Arc<bilibi_artifacts::Artifacts>,
}

impl Training {
    /// Whether the actor currently holds the capability for a live company
    /// with a current workforce read (Bilimbi `allowed?/3`), for deciding
    /// which controls a page renders; every operation authorizes again.
    pub async fn allowed(&self, scope: &Scope, company_id: i64, capability: &str) -> bool {
        match self.db.get().await {
            Ok(client) => self
                .authorize(&**client, scope, company_id, capability)
                .await
                .is_ok(),
            Err(_) => false,
        }
    }

    /// The company's courses, active or not, by name (`people.training.courses.view`).
    pub async fn courses(&self, scope: &Scope, company_id: i64) -> Result<Vec<Course>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company_id, COURSES_VIEW)
            .await?;
        catalog::courses(&**client, scope.tenant(), company_id).await
    }

    /// Adds an active course (`people.training.courses.manage`); a taken
    /// code, a malformed code or an overlong text is `Invalid`.
    pub async fn create_course(
        &self,
        scope: &Scope,
        company_id: i64,
        new: &NewCourse,
    ) -> Result<Course> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company_id, COURSES_MANAGE)
            .await?;
        let valid = catalog::validate_course(new)?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.training.create_course"),
            )
            .await?;
        writable(tx.sql(), scope, company_id).await?;
        let actor = self
            .authorize(tx.sql(), scope, company_id, COURSES_MANAGE)
            .await?;
        let course =
            catalog::insert_course(&mut tx, scope.tenant(), company_id, actor, valid).await?;
        tx.commit().await?;
        Ok(course)
    }

    /// Changes a course's name, description or active flag
    /// (`people.training.courses.manage`); a course of another company or
    /// missing is `NotFound`. Deactivating keeps its events and sessions and
    /// stops new events.
    pub async fn update_course(
        &self,
        scope: &Scope,
        company_id: i64,
        course_id: i64,
        update: &CourseUpdate,
    ) -> Result<Course> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company_id, COURSES_MANAGE)
            .await?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.training.update_course"),
            )
            .await?;
        writable(tx.sql(), scope, company_id).await?;
        let actor = self
            .authorize(tx.sql(), scope, company_id, COURSES_MANAGE)
            .await?;
        let current = catalog::lock_course(tx.sql(), scope.tenant(), company_id, course_id).await?;
        let valid = catalog::validate_course_update(&current, update)?;
        let active = update.active.unwrap_or(current.active);
        let course =
            catalog::update_course(&mut tx, scope.tenant(), current, actor, valid, active).await?;
        tx.commit().await?;
        Ok(course)
    }

    /// Deactivates (or reactivates) a course: the page's Deactivate and
    /// Reactivate actions, through [`Training::update_course`].
    pub async fn set_course_active(
        &self,
        scope: &Scope,
        company_id: i64,
        course_id: i64,
        active: bool,
    ) -> Result<Course> {
        self.update_course(
            scope,
            company_id,
            course_id,
            &CourseUpdate {
                active: Some(active),
                ..CourseUpdate::default()
            },
        )
        .await
    }

    /// The company's events with their course names, by event name
    /// (`people.training.sessions.view`).
    pub async fn events(&self, scope: &Scope, company_id: i64) -> Result<Vec<Event>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company_id, SESSIONS_VIEW)
            .await?;
        catalog::events(&**client, scope.tenant(), company_id).await
    }

    /// The active courses an event may deliver, as choices
    /// (`people.training.sessions.manage`): a session manager picks a course
    /// without course-management permission.
    pub async fn event_courses(&self, scope: &Scope, company_id: i64) -> Result<Vec<CourseOption>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company_id, SESSIONS_MANAGE)
            .await?;
        catalog::active_courses(&**client, scope.tenant(), company_id).await
    }

    /// Adds an event for an active course of the company
    /// (`people.training.sessions.manage`); a missing or inactive course is
    /// `CourseUnavailable`, a non-positive or too large capacity `Invalid`.
    pub async fn create_event(
        &self,
        scope: &Scope,
        company_id: i64,
        new: &NewEvent,
    ) -> Result<Event> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company_id, SESSIONS_MANAGE)
            .await?;
        let valid = catalog::validate_event(new)?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.training.create_event"),
            )
            .await?;
        writable(tx.sql(), scope, company_id).await?;
        let actor = self
            .authorize(tx.sql(), scope, company_id, SESSIONS_MANAGE)
            .await?;
        let event =
            catalog::insert_event(&mut tx, scope.tenant(), company_id, actor, valid).await?;
        tx.commit().await?;
        Ok(event)
    }

    /// Adds a session of an event of the company
    /// (`people.training.sessions.manage`). The local wall clocks are placed
    /// in the named zone first (`InvalidTimeZoneOrTime`, `NonexistentTime`,
    /// `AmbiguousTime`), the end must follow the start (`InvalidTimeRange`),
    /// the event must exist (`EventUnavailable`) and the capacity must not
    /// exceed the event's (`CapacityExceeded`), judged with the event row
    /// locked; PostgreSQL's trigger guards the same bound.
    pub async fn create_session(
        &self,
        scope: &Scope,
        company_id: i64,
        new: &NewSession,
    ) -> Result<Session> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company_id, SESSIONS_MANAGE)
            .await?;
        let valid = catalog::validate_session(new)?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.training.create_session"),
            )
            .await?;
        writable(tx.sql(), scope, company_id).await?;
        let actor = self
            .authorize(tx.sql(), scope, company_id, SESSIONS_MANAGE)
            .await?;
        let session =
            catalog::insert_session(&mut tx, scope.tenant(), company_id, actor, valid).await?;
        tx.commit().await?;
        Ok(session)
    }

    /// The sessions overlapping the half-open UTC window `[from, until)`
    /// (`starts_at < until` and `ends_at > from`), by instant then id, at
    /// most [`CALENDAR_LIMIT`] rows (`people.training.sessions.view`). A
    /// window that is empty, backwards or longer than [`CALENDAR_MAX_DAYS`]
    /// is `InvalidCalendarRange`. A page reports the bound when the count
    /// reaches it.
    pub async fn calendar(
        &self,
        scope: &Scope,
        company_id: i64,
        from: Timestamp,
        until: Timestamp,
    ) -> Result<Vec<Session>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company_id, SESSIONS_VIEW)
            .await?;
        if until <= from
            || until.as_nanosecond() - from.as_nanosecond()
                > i128::from(CALENDAR_MAX_DAYS * 86_400) * 1_000_000_000
        {
            return Err(TrainingError::InvalidCalendarRange);
        }
        catalog::calendar(&**client, scope.tenant(), company_id, from, until).await
    }

    /// The company's own zone, Base DateTime's `localization.timezone` as
    /// Core Company manages it, `UTC` when unset or no longer convertible
    /// (`people.training.sessions.view`): the zone a calendar bounds its days
    /// in and a session form starts with.
    pub async fn company_zone(&self, scope: &Scope, company_id: i64) -> Result<String> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company_id, SESSIONS_VIEW)
            .await?;
        self.zone(&**client, scope, company_id).await
    }

    /// The UTC bounds of the company's local days from `from` up to but not
    /// including `until`, as a half-open window for [`Training::calendar`]:
    /// each bound is the first instant of that local day in the company's
    /// zone ([`day_start`]), so a DST gap or overlap at local midnight never
    /// blanks a month. Returns the zone beside the bounds.
    pub async fn company_day_bounds(
        &self,
        scope: &Scope,
        company_id: i64,
        from: Date,
        until: Date,
    ) -> Result<(Timestamp, Timestamp, String)> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company_id, SESSIONS_VIEW)
            .await?;
        let zone = self.zone(&**client, scope, company_id).await?;
        let start = day_start(from, &zone)?;
        let end = day_start(until, &zone)?;
        Ok((start, end, zone))
    }

    async fn zone(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company_id: i64,
    ) -> Result<String> {
        let resolved = match self
            .settings
            .get(
                client,
                scope,
                Target::Company(OrganisationId::new(company_id)?),
                bilibi_datetime::TIMEZONE_KEY,
            )
            .await
        {
            Ok(resolved) => resolved,
            // A stored zone that no longer converts falls back to the
            // default, as Base DateTime's display policy does.
            Err(Error::Invalid) => return Ok(bilibi_datetime::DEFAULT_TIMEZONE.to_owned()),
            Err(error) => return Err(error.into()),
        };
        Ok(resolved
            .value
            .as_str()
            .filter(|zone| bilibi_datetime::valid_timezone(zone))
            .unwrap_or(bilibi_datetime::DEFAULT_TIMEZONE)
            .to_owned())
    }

    /// Bilimbi `authorize/3`: the capability now and the company axis through
    /// the Workforce helpers (`Unauthorized`, `NotFound`), an archived company
    /// refused (`CompanyUnavailable`), and the Workforce company read required
    /// current (`WorkforceNotCurrent`). Returns the sealed actor's identity
    /// for the facts it writes.
    async fn authorize(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company_id: i64,
        capability: &str,
    ) -> Result<catalog::Actor> {
        if company_id <= 0 {
            return Err(TrainingError::NotFound);
        }
        let company =
            authorization::authorize_company(client, scope, company_id, capability).await?;
        if company.archived {
            return Err(TrainingError::CompanyUnavailable);
        }
        self.workforce
            .company_on(client, scope, company_id)
            .await?
            .require_current()
            .map_err(|not_current| TrainingError::WorkforceNotCurrent(not_current.0))?;
        Ok(catalog::Actor {
            user: scope.user(),
            impersonator: scope.impersonator().map(|operator| operator.user()),
        })
    }
}

/// The company's writable lock every write holds first: an archived company
/// is `CompanyUnavailable`, and the lock serializes the company's writes.
async fn writable(client: &impl GenericClient, scope: &Scope, company_id: i64) -> Result<()> {
    organisation::require_writable(client, scope, company_id, true)
        .await
        .map(|_| ())
        .map_err(|error| match error {
            WriteError::NotFound => TrainingError::NotFound,
            WriteError::Archived => TrainingError::CompanyUnavailable,
            WriteError::Access(error) => error.into(),
            _ => TrainingError::Unauthorized,
        })
}

mod web_learning;
pub use web_learning::LEARNING_VIEW;
