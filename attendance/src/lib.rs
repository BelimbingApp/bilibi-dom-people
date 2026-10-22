//! Attendance (`people/attendance`, Bilimbi `Bilimbi.People.Attendance`):
//! company-scoped clock facts, day projections, shift templates, clocking
//! locations and the company's attendance rules. Rosters, adjustments and
//! allowance rules follow in port plan row PA2.
//!
//! This is the module's one facade. Pages, operations and other People
//! modules call [`Attendance`], resolved from the registry as [`SERVICE`];
//! the schema, SQL, validation and projection stay private. Every function
//! takes the sealed [`Scope`] it was handed and an explicit platform company
//! id, and reads companies and employees only through `people/workforce`: a
//! stale or unavailable workforce read refuses the operation
//! ([`AttendanceError::Unavailable`]).
//!
//! Operator writes authorize their capability on every call through the
//! Workforce `Authorization` helpers with the scope's sealed actor, so a
//! grant revoked while a page stays connected refuses the next event.
//! Self-service ([`Attendance::self_days`]) resolves the signed-in account's
//! current linked working employee on every call. No function takes an
//! actor or the employee it acts for from its caller: attribution comes from
//! the scope.
//!
//! See `README.md`.
mod access;
mod clock;
mod contributions;
mod locations;
mod rules;
mod shifts;

pub use clock::{ClockEvent, ClockInput, Day, DayStatus, EventType, DAYS_SHOWN};
pub use contributions::{
    ADJUSTMENTS_APPROVE, ADJUSTMENT_WINDOW_KEY, ALLOWANCES_MANAGE, CAPABILITIES,
    LOCATION_REQUIRED_KEY, MAX_SHIFT_KEY, ROSTER_MANAGE, RULES_MANAGE, RULE_KEYS, SELF_CLOCK_KEY,
    SELF_VIEW, TIMEZONE_KEY,
};
pub use locations::{ClockingLocation, NewClockingLocation, MAX_RADIUS_METERS};
pub use rules::{RuleChanges, Rules};
pub use shifts::{NewShiftTemplate, ShiftTemplate};

/// A refused value in the words a form shows beside the field (Workforce's
/// type, shared across People).
pub use bilibi_people_workforce::FieldError;

use bilibi_database::{Db, Error, MutationBoundary};
use bilibi_people_workforce::{NotCurrent, Workforce, WorkforceError};
use bilibi_settings::Settings;
use bilibi_tenancy::Scope;
use std::sync::Arc;

/// The module ID, Bilimbi's, kept because IDs are persisted data.
pub const MODULE: &str = "people/attendance";
/// The registry service ID of the [`Attendance`] facade.
pub const SERVICE: &str = "people/attendance.attendance";
/// The two states a shift template or clocking location is in: retired,
/// never deleted, so published rosters and recorded facts keep their meaning.
pub const STATUSES: [&str; 2] = ["active", "retired"];

pub type Result<T> = std::result::Result<T, AttendanceError>;

/// Why a call did not happen (Bilimbi's refusal atoms and changeset errors).
#[derive(Debug, thiserror::Error)]
pub enum AttendanceError {
    /// A denied or unknown capability, or a company outside the actor's reach.
    #[error("Access denied")]
    Unauthorized,
    /// A company, employee, template or location the scope cannot see.
    #[error("Not found")]
    NotFound,
    /// The account is not linked to a working employee of the company
    /// (`:not_linked`).
    #[error("Your account is no longer linked to a working employee in this company.")]
    NotLinked,
    /// The workforce read was stale or unavailable (`:stale`, `:unavailable`).
    #[error("The workforce is unavailable")]
    Unavailable(NotCurrent),
    #[error("This company is archived and read-only")]
    CompanyArchived,
    /// A rule outside its range, or a time zone the database does not know
    /// (`:invalid_rules`).
    #[error("Enter a valid time zone, a shift length of 1 to 24 hours, and an adjustment window of 1 to 366 days.")]
    InvalidRules,
    /// A clock event with a bad key, source, coordinates or type
    /// (`:invalid_event`).
    #[error("Invalid clock event")]
    InvalidEvent,
    /// The company requires a clocking location and the event carries no
    /// coordinates (`:location_required`).
    #[error("A valid location is required to clock here.")]
    LocationRequired,
    /// The company requires a clocking location and the point is inside none
    /// of the active ones (`:outside_clocking_location`).
    #[error("You are outside this company's approved clocking locations.")]
    OutsideClockingLocation,
    /// A replay of a recorded key with different facts
    /// (`:event_key_conflict`).
    #[error("This clock event key was already recorded with different facts")]
    EventKeyConflict,
    /// A status other than `active` or `retired` (`:invalid_status`).
    #[error("Invalid status")]
    InvalidStatus,
    /// Refused values, in the words a form shows beside each field.
    #[error("Invalid input")]
    Invalid(Vec<FieldError>),
    /// A database or capture failure; never a refusal.
    #[error(transparent)]
    Failed(Error),
}
impl From<Error> for AttendanceError {
    fn from(error: Error) -> Self {
        match error {
            Error::Forbidden | Error::Unauthorized => Self::Unauthorized,
            Error::NotFound => Self::NotFound,
            other => Self::Failed(other),
        }
    }
}
impl From<WorkforceError> for AttendanceError {
    fn from(error: WorkforceError) -> Self {
        match error {
            WorkforceError::Unauthorized => Self::Unauthorized,
            WorkforceError::NotLinked => Self::NotLinked,
            WorkforceError::CompanyArchived => Self::CompanyArchived,
            WorkforceError::Failed(error) => Self::Failed(error),
            // A refused option or an unknown employee: nothing to act on.
            _ => Self::NotFound,
        }
    }
}
impl From<NotCurrent> for AttendanceError {
    fn from(freshness: NotCurrent) -> Self {
        Self::Unavailable(freshness)
    }
}
impl From<bilibi_party_ids::InvalidId> for AttendanceError {
    fn from(_: bilibi_party_ids::InvalidId) -> Self {
        Self::NotFound
    }
}

/// The facade, resolved from the registry as [`SERVICE`].
pub struct Attendance {
    db: Db,
    mutations: Arc<MutationBoundary>,
    settings: Arc<Settings>,
    workforce: Arc<Workforce>,
}

impl Attendance {
    /// The company's five rules (Bilimbi `rules/2`): no capability, as a
    /// clock event and a self-service page both read them; the company must
    /// be a live company of the scope's tenant.
    pub async fn rules(&self, scope: &Scope, company_id: i64) -> Result<Rules> {
        let client = self.db.get().await.map_err(Error::from)?;
        let company = access::current_company(&self.workforce, scope, company_id).await?;
        rules::read(&**client, &self.settings, scope, &company).await
    }

    /// Stores the given rules (`people.attendance.rules.manage`); omitted
    /// rules keep their current value. A time zone the database does not
    /// know, a shift length outside 1 to 24 hours or a window outside 1 to
    /// 366 days is `InvalidRules`; an archived company `CompanyArchived`.
    /// Returns the rules as they now read.
    pub async fn put_rules(
        &self,
        scope: &Scope,
        company_id: i64,
        changes: &RuleChanges,
    ) -> Result<Rules> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        bilibi_people_workforce::authorization::authorize(
            &**client,
            scope,
            company_id,
            RULES_MANAGE,
        )
        .await?;
        rules::validate(changes)?;
        let company = access::current_company(&self.workforce, scope, company_id).await?;
        if !changes.is_empty() {
            let mut tx = self
                .mutations
                .begin(
                    &mut client,
                    scope.mutation_context("people.attendance.put_rules"),
                )
                .await?;
            access::lock_rules_company(&tx, scope, company_id).await?;
            rules::write(&mut tx, &self.settings, scope, &company, changes).await?;
            tx.commit().await?;
        }
        rules::read(&**client, &self.settings, scope, &company).await
    }

    /// Records one clock fact for a working employee of the company and
    /// projects the employee's day (Bilimbi `record_clock/4`): idempotent by
    /// company, source and key, so a replay with the same facts answers the
    /// recorded event and one with different facts is `EventKeyConflict`.
    ///
    /// The event's local date is its instant in the company's attendance
    /// time zone. A clock-in opens that day; any other event closes the
    /// previous day's open shift instead when it falls within the company's
    /// maximum shift length of that shift's clock-in. A day with a clock-in
    /// and no clock-out is `in_progress`; a clock-in and a later clock-out
    /// within the maximum length is `ready_for_review` with the worked
    /// minutes; anything else is `exception_pending`.
    ///
    /// Coordinates (both or neither, within ±90 and ±180, kept to six
    /// decimals) place the event in the nearest active clocking location
    /// whose radius contains the point. When the company requires a
    /// clocking location, an event without coordinates is
    /// `LocationRequired` and one outside every active location is
    /// `OutsideClockingLocation`. The scope's actor is recorded as the
    /// event's actor.
    ///
    /// No capability of its own, as in Bilimbi: a caller authorizes what it
    /// records (self-clocking proves the signed-in employee, an approved
    /// adjustment its approver, an operation its own key). An employee who
    /// is not a working employee of the company is `NotFound`.
    pub async fn record_clock(
        &self,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
        input: &ClockInput,
    ) -> Result<ClockEvent> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.attendance.record_clock"),
            )
            .await?;
        access::lock_company(&tx, scope, company_id).await?;
        access::current_employee(&self.workforce, scope, company_id, employee_id).await?;
        let company = access::current_company(&self.workforce, scope, company_id).await?;
        // Resolve policy and active locations after taking the company lock:
        // a queued clock must observe a rule change or retirement ahead of it.
        let rules = rules::read(tx.sql(), &self.settings, scope, &company).await?;
        let event = clock::normalize(input, &rules.timezone)?;
        let location = locations::place(
            tx.sql(),
            scope,
            company_id,
            event.point(),
            rules.location_required,
        )
        .await?;
        let recorded = clock::write(
            &mut tx,
            scope,
            company_id,
            employee_id,
            event,
            location,
            rules.max_shift_hours,
        )
        .await?;
        tx.commit().await?;
        Ok(recorded)
    }

    /// The [`DAYS_SHOWN`] most recent days of the signed-in actor's own
    /// linked working employee, resolved now (Bilimbi `self_days/2`): the
    /// actor needs `people.attendance.self.view` for the company, and an
    /// account not linked to a working employee there is `NotLinked`.
    pub async fn self_days(&self, scope: &Scope, company_id: i64) -> Result<Vec<Day>> {
        let employee_id = self
            .workforce
            .authorize_self(scope, company_id, SELF_VIEW)
            .await?
            .employee_id;
        self.list_days(scope, company_id, employee_id).await
    }

    /// The [`DAYS_SHOWN`] most recent days of one working employee of the
    /// company, newest first (Bilimbi `list_days/3`). An open shift older
    /// than the company's maximum shift length reads as
    /// `exception_pending`.
    pub async fn list_days(
        &self,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
    ) -> Result<Vec<Day>> {
        let client = self.db.get().await.map_err(Error::from)?;
        access::current_employee(&self.workforce, scope, company_id, employee_id).await?;
        let rules = self.rules(scope, company_id).await?;
        clock::days(
            &**client,
            scope,
            company_id,
            employee_id,
            rules.max_shift_hours,
        )
        .await
    }

    /// The company's shift templates, active first then by code (Bilimbi
    /// `list_shift_templates/2`); no capability.
    pub async fn list_shift_templates(
        &self,
        scope: &Scope,
        company_id: i64,
    ) -> Result<Vec<ShiftTemplate>> {
        let client = self.db.get().await.map_err(Error::from)?;
        access::current_company(&self.workforce, scope, company_id).await?;
        shifts::list(&**client, scope, company_id).await
    }

    /// Adds a shift template (`people.attendance.rules.manage`): a code
    /// unique in the company, a name, local start and end times (an end at
    /// or before the start crosses midnight) and break minutes shorter than
    /// the shift. Refused values are `Invalid` field errors in Bilimbi's
    /// words.
    pub async fn create_shift_template(
        &self,
        scope: &Scope,
        company_id: i64,
        new: &NewShiftTemplate,
    ) -> Result<ShiftTemplate> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        bilibi_people_workforce::authorization::authorize(
            &**client,
            scope,
            company_id,
            RULES_MANAGE,
        )
        .await?;
        access::current_company(&self.workforce, scope, company_id).await?;
        let valid = shifts::validate(new).map_err(AttendanceError::Invalid)?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.attendance.create_shift_template"),
            )
            .await?;
        access::lock_rules_company(&tx, scope, company_id).await?;
        shifts::check_code(tx.sql(), scope.tenant(), company_id, &valid.code).await?;
        let template = shifts::insert(&mut tx, scope.tenant(), company_id, valid).await?;
        tx.commit().await?;
        Ok(template)
    }

    /// Retires or reactivates a shift template
    /// (`people.attendance.rules.manage`): `status` is `active` or
    /// `retired`, else `InvalidStatus`; a template of another company or
    /// missing is `NotFound`. Templates are never deleted.
    pub async fn set_shift_template_status(
        &self,
        scope: &Scope,
        company_id: i64,
        template_id: i64,
        status: &str,
    ) -> Result<ShiftTemplate> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        bilibi_people_workforce::authorization::authorize(
            &**client,
            scope,
            company_id,
            RULES_MANAGE,
        )
        .await?;
        access::current_company(&self.workforce, scope, company_id).await?;
        let status = valid_status(status)?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.attendance.set_shift_template_status"),
            )
            .await?;
        access::lock_rules_company(&tx, scope, company_id).await?;
        let template = shifts::set_status(&mut tx, scope, company_id, template_id, status).await?;
        tx.commit().await?;
        Ok(template)
    }

    /// The company's clocking locations, active first then by code (Bilimbi
    /// `list_clocking_locations/2`); no capability.
    pub async fn list_clocking_locations(
        &self,
        scope: &Scope,
        company_id: i64,
    ) -> Result<Vec<ClockingLocation>> {
        let client = self.db.get().await.map_err(Error::from)?;
        access::current_company(&self.workforce, scope, company_id).await?;
        locations::list(&**client, scope, company_id).await
    }

    /// Adds a clocking location (`people.attendance.rules.manage`): a code
    /// unique in the company, a name, a point and a radius of 1 to
    /// [`MAX_RADIUS_METERS`] meters. Refused values are `Invalid` field
    /// errors in Bilimbi's words.
    pub async fn create_clocking_location(
        &self,
        scope: &Scope,
        company_id: i64,
        new: &NewClockingLocation,
    ) -> Result<ClockingLocation> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        bilibi_people_workforce::authorization::authorize(
            &**client,
            scope,
            company_id,
            RULES_MANAGE,
        )
        .await?;
        access::current_company(&self.workforce, scope, company_id).await?;
        let valid = locations::validate(new).map_err(AttendanceError::Invalid)?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.attendance.create_clocking_location"),
            )
            .await?;
        access::lock_rules_company(&tx, scope, company_id).await?;
        locations::check_code(tx.sql(), scope.tenant(), company_id, &valid.code).await?;
        let location = locations::insert(&mut tx, scope.tenant(), company_id, valid).await?;
        tx.commit().await?;
        Ok(location)
    }

    /// Retires or reactivates a clocking location
    /// (`people.attendance.rules.manage`): `status` is `active` or
    /// `retired`, else `InvalidStatus`; a location of another company or
    /// missing is `NotFound`. A retired location no longer places a clock
    /// event; the events it placed keep naming it.
    pub async fn set_clocking_location_status(
        &self,
        scope: &Scope,
        company_id: i64,
        location_id: i64,
        status: &str,
    ) -> Result<ClockingLocation> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        bilibi_people_workforce::authorization::authorize(
            &**client,
            scope,
            company_id,
            RULES_MANAGE,
        )
        .await?;
        access::current_company(&self.workforce, scope, company_id).await?;
        let status = valid_status(status)?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.attendance.set_clocking_location_status"),
            )
            .await?;
        access::lock_rules_company(&tx, scope, company_id).await?;
        let location =
            locations::set_status(&mut tx, scope, company_id, location_id, status).await?;
        tx.commit().await?;
        Ok(location)
    }
}

/// `active` or `retired`, as a static string.
fn valid_status(status: &str) -> Result<&'static str> {
    STATUSES
        .iter()
        .copied()
        .find(|known| *known == status)
        .ok_or(AttendanceError::InvalidStatus)
}
