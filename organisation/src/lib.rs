//! Organisation (`people/organisation`, Bilimbi `Bilimbi.People.Organisation`):
//! a company's positions, their immutable effective-dated title versions and
//! the effective-dated assignments of employees to them.
//!
//! This is the module's one facade. Other People modules read and write
//! through [`Organisation`], resolved from the registry as [`SERVICE`];
//! Workforce's position scan reaches it through the
//! `people/organisation.positions` port (`port.rs`), not an edge. The schema,
//! SQL and validation stay private. Every function takes the sealed [`Scope`]
//! it was handed and an explicit platform company id.
//!
//! A position is a stable identity; a vacancy leaves it and its title in
//! place. Substantive assignments cannot overlap for one position; acting and
//! concurrent cover can. Both interval ends are inclusive, and PostgreSQL's
//! exclusion constraints enforce the non-overlap rules even for concurrent
//! writers. Parent links are company-bound and cycle-checked under the
//! company lock.
//!
//! Every write ([`Organisation::create_position`], [`Organisation::set_parent`],
//! [`Organisation::record_version`], [`Organisation::assign`],
//! [`Organisation::end_assignment`]) authorizes `people.organisation.manage`
//! for the scope's signed-in actor and the company at the moment it runs,
//! through the Workforce `Authorization` helpers, then holds the company's
//! writable lock inside its mutation transaction. An assignment accepts only
//! an employee the Workforce read seam exposes as working; a new substantive
//! assignment ends an earlier substantive holder who is no longer working on
//! the day before it starts, in the same transaction, recording the audit
//! action `people.organisation.assignment_released`; `end_assignment` only
//! moves an end earlier and records `people.organisation.assignment_ended`.
//!
//! The reads ([`Organisation::positions`], [`Organisation::scan_positions`],
//! [`Organisation::count_positions`]) need no capability, as Bilimbi's do:
//! they expose only a live company of the scope's tenant, and an assignee who
//! has since left the working statuses is omitted from holders and no longer
//! occupies the position, while the assignment row stays as history. A page
//! holds at most 100 positions; the keyset scan fixes a high-water mark on
//! its first page (`paging.rs`).
//!
//! See `README.md`.
mod agent;
mod contributions;
mod paging;
mod port;
mod positions;
mod web;

pub use bilibi_people_workforce::FieldError;
pub use contributions::{CAPABILITIES, MANAGE, POSITIONS_PORT, VIEW};

use bilibi_company::WriteError;
use bilibi_database::{Db, Error, MutationBoundary, MutationTransaction};
use bilibi_people_workforce::{
    authorization, Freshness, NotCurrent, Reference, ReferenceType, Workforce, WorkforceError,
    SOURCE_ID,
};
use bilibi_tenancy::Scope;
use jiff::civil::Date;
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tokio_postgres::GenericClient;

/// The module ID, Bilimbi's, kept because IDs are persisted data.
pub const MODULE: &str = "people/organisation";
/// The registry service ID of the [`Organisation`] facade.
pub const SERVICE: &str = "people/organisation.organisation";
/// The positions a page holds when the caller names no size.
pub const PAGE_SIZE: i64 = 50;
/// The most positions one page or scan step returns.
pub const MAX_PAGE_SIZE: i64 = 100;
/// The most effective assignments one page reads; past it the later
/// positions report `assignments_incomplete`.
pub const ASSIGNMENT_LIMIT: i64 = 500;
/// The three assignment kinds; only `substantive` occupies a position.
pub const ASSIGNMENT_KINDS: [&str; 3] = ["substantive", "acting", "concurrent"];
/// The audit action recorded when a new substantive assignment ends an earlier
/// holder who is no longer working.
pub const ASSIGNMENT_RELEASED: &str = "people.organisation.assignment_released";
/// The audit action recorded when an actor ends an assignment.
pub const ASSIGNMENT_ENDED: &str = "people.organisation.assignment_ended";

pub type Result<T> = std::result::Result<T, OrganisationError>;

/// Why a call did not happen (Bilimbi's refusal atoms and changeset errors).
#[derive(Debug, thiserror::Error)]
pub enum OrganisationError {
    /// A denied or unknown capability, or a company outside the actor's reach.
    #[error("Access denied")]
    Unauthorized,
    /// A company, position, assignment or employee the scope cannot see or
    /// that is not of this company (indistinguishable on purpose).
    #[error("Not found")]
    NotFound,
    #[error("This company is archived and read-only")]
    CompanyArchived,
    /// The parent would make the position its own ancestor.
    #[error("The parent would make the position its own ancestor")]
    Cycle,
    /// A page below 1, or a page size outside 1..=100.
    #[error("Invalid options")]
    InvalidOptions,
    /// A malformed scan cursor, or one minted for another tenant, company or day.
    #[error("Invalid cursor")]
    InvalidCursor,
    /// The Workforce read of the holders was stale or unavailable.
    #[error("The workforce read is not current")]
    NotCurrent(Freshness),
    /// Refused values, in the words a form shows beside each field.
    #[error("Invalid input")]
    Invalid(Vec<FieldError>),
    /// A database or capture failure; never a refusal.
    #[error(transparent)]
    Failed(Error),
}
impl From<Error> for OrganisationError {
    fn from(error: Error) -> Self {
        match error {
            Error::Forbidden | Error::Unauthorized => Self::Unauthorized,
            Error::NotFound => Self::NotFound,
            other => Self::Failed(other),
        }
    }
}
impl From<WorkforceError> for OrganisationError {
    fn from(error: WorkforceError) -> Self {
        match error {
            WorkforceError::Unauthorized => Self::Unauthorized,
            WorkforceError::NotFound => Self::NotFound,
            WorkforceError::CompanyArchived => Self::CompanyArchived,
            WorkforceError::InvalidOptions => Self::InvalidOptions,
            WorkforceError::InvalidCursor => Self::InvalidCursor,
            WorkforceError::Unavailable => {
                Self::NotCurrent(Freshness::Unavailable("workforce".into()))
            }
            WorkforceError::Invalid(errors) => Self::Invalid(errors),
            WorkforceError::Failed(error) => Self::Failed(error),
            // Employment-write refusals; the reads this module makes never
            // answer them.
            WorkforceError::InvalidStatuses
            | WorkforceError::DuplicateAffiliation
            | WorkforceError::AffiliationMismatch
            | WorkforceError::NotHuman
            | WorkforceError::NotLinked
            | WorkforceError::Stale => Self::Failed(Error::Invalid),
        }
    }
}
impl From<NotCurrent> for OrganisationError {
    fn from(NotCurrent(freshness): NotCurrent) -> Self {
        Self::NotCurrent(freshness)
    }
}

fn as_text<S: serde::Serializer>(
    value: &Date,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.collect_str(value)
}
fn as_optional_text<S: serde::Serializer>(
    value: &Option<Date>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    match value {
        Some(value) => serializer.collect_str(value),
        None => serializer.serialize_none(),
    }
}

/// A stable position identity of one company: the row of `people_positions`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Position {
    pub id: i64,
    pub platform_company_id: i64,
    pub code: String,
    pub parent_id: Option<i64>,
}

/// One immutable title version of a position, effective over an inclusive
/// interval: the row of `people_position_revisions`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PositionVersion {
    pub id: i64,
    pub position_id: i64,
    pub version: i32,
    pub title: String,
    #[serde(serialize_with = "as_text")]
    pub effective_from: Date,
    #[serde(serialize_with = "as_optional_text")]
    pub effective_to: Option<Date>,
}

/// One assignment of an employee (by employment id) to a position over an
/// inclusive interval: the row of `people_position_placements`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Assignment {
    pub id: i64,
    pub position_id: i64,
    pub employee_id: i64,
    pub kind: String,
    #[serde(serialize_with = "as_text")]
    pub effective_from: Date,
    #[serde(serialize_with = "as_optional_text")]
    pub effective_to: Option<Date>,
}

/// What creates a position: a code unique in the company and an optional
/// parent position of the same company.
#[derive(Clone, Debug)]
pub struct NewPosition {
    pub code: String,
    pub parent_id: Option<i64>,
}
impl NewPosition {
    pub fn new(code: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            parent_id: None,
        }
    }
}

/// What appends a title version: a positive number unique for the position,
/// a title and an inclusive interval no existing version overlaps.
#[derive(Clone, Debug)]
pub struct NewVersion {
    pub version: i32,
    pub title: String,
    pub effective_from: Date,
    pub effective_to: Option<Date>,
}

/// What appends an assignment: a working employee of the company, one of the
/// three kinds and an inclusive interval.
#[derive(Clone, Debug)]
pub struct NewAssignment {
    pub employee_id: i64,
    pub kind: String,
    pub effective_from: Date,
    pub effective_to: Option<Date>,
}

/// A `people/native` reference to one assignment, beside Workforce's
/// `Reference` for positions, companies and employees.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AssignmentReference {
    pub source_id: String,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub stable_id: String,
}
impl AssignmentReference {
    fn new(id: i64) -> Self {
        Self {
            source_id: SOURCE_ID.to_owned(),
            kind: "assignment",
            stable_id: id.to_string(),
        }
    }
}
fn reference(kind: ReferenceType, id: i64) -> Reference {
    Reference {
        source_id: SOURCE_ID.to_owned(),
        kind,
        stable_id: id.to_string(),
    }
}

/// One holder of a position as of a day (Bilimbi's projected assignment):
/// the assignment, the employee and the kind.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AssignmentProjection {
    pub reference: AssignmentReference,
    pub employee_reference: Reference,
    pub kind: String,
}

/// The public native workforce position projection (Bilimbi
/// `Bilimbi.People.Workforce.Position`), as of a day: the position, its
/// effective title and version (absent when no version covers the day), its
/// working holders, whether the page's assignment bound cut them short, and
/// whether no working substantive holder occupies it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PositionProjection {
    pub reference: Reference,
    pub company_reference: Reference,
    pub platform_company_id: i64,
    pub workforce_company_id: i64,
    pub code: String,
    pub parent_reference: Option<Reference>,
    pub title: Option<String>,
    pub version: Option<i32>,
    pub assignments: Vec<AssignmentProjection>,
    pub assignments_incomplete: bool,
    pub vacant: bool,
}

/// One step of the keyset scan: the page, the cursor that resumes it (`None`
/// once the scan is complete) and the high-water mark every step repeats.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PositionScan {
    pub positions: Vec<PositionProjection>,
    pub next_cursor: Option<String>,
    pub high_water_id: i64,
}

/// The facade, resolved from the registry as [`SERVICE`].
pub struct Organisation {
    db: Db,
    mutations: Arc<MutationBoundary>,
    workforce: Arc<Workforce>,
}

impl Organisation {
    /// Commits the position creation on a reviewed operation's transaction.
    pub(crate) async fn create_position_in(
        &self,
        tx: &mut MutationTransaction<'_>,
        scope: &Scope,
        company_id: i64,
        new: &NewPosition,
    ) -> Result<Position> {
        lock_company(tx.sql(), scope, company_id).await?;
        let valid = positions::validate_position(new).map_err(OrganisationError::Invalid)?;
        if let Some(parent) = valid.parent_id {
            positions::find(tx.sql(), company_id, parent)
                .await?
                .ok_or(OrganisationError::NotFound)?;
        }
        positions::check_code(tx.sql(), company_id, &valid.code).await?;
        positions::insert_position(tx, company_id, valid).await
    }

    /// Commits a parent change on a reviewed operation's transaction.
    pub(crate) async fn set_parent_in(
        &self,
        tx: &mut MutationTransaction<'_>,
        scope: &Scope,
        company_id: i64,
        position_id: i64,
        parent_id: Option<i64>,
    ) -> Result<Position> {
        lock_company(tx.sql(), scope, company_id).await?;
        let position = positions::find(tx.sql(), company_id, position_id)
            .await?
            .ok_or(OrganisationError::NotFound)?;
        if let Some(parent) = parent_id {
            positions::find(tx.sql(), company_id, parent)
                .await?
                .ok_or(OrganisationError::NotFound)?;
        }
        positions::reject_cycle(tx.sql(), company_id, position.id, parent_id).await?;
        positions::update_parent(tx, position, parent_id).await
    }

    /// Commits a title version on a reviewed operation's transaction.
    pub(crate) async fn record_version_in(
        &self,
        tx: &mut MutationTransaction<'_>,
        scope: &Scope,
        company_id: i64,
        position_id: i64,
        new: &NewVersion,
    ) -> Result<PositionVersion> {
        lock_company(tx.sql(), scope, company_id).await?;
        let position = positions::find(tx.sql(), company_id, position_id)
            .await?
            .ok_or(OrganisationError::NotFound)?;
        let valid = positions::validate_version(new).map_err(OrganisationError::Invalid)?;
        positions::check_version(tx.sql(), position.id, &valid).await?;
        positions::insert_version(tx, position.id, valid).await
    }

    /// Creates a stable position identity in one live company
    /// (`people.organisation.manage`): a code unique in the company (trimmed;
    /// "can't be blank", "should be at most 255 character(s)", "has already
    /// been taken") under an optional parent position of the same company
    /// (another company's, or missing, is `NotFound`).
    pub async fn create_position(
        &self,
        scope: &Scope,
        company_id: i64,
        new: &NewPosition,
    ) -> Result<Position> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, MANAGE).await?;
        self.live_company(&**client, scope, company_id).await?;
        let valid = positions::validate_position(new).map_err(OrganisationError::Invalid)?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.organisation.create_position"),
            )
            .await?;
        lock_company(tx.sql(), scope, company_id).await?;
        if let Some(parent) = valid.parent_id {
            positions::find(tx.sql(), company_id, parent)
                .await?
                .ok_or(OrganisationError::NotFound)?;
        }
        positions::check_code(tx.sql(), company_id, &valid.code).await?;
        let position = positions::insert_position(&mut tx, company_id, valid).await?;
        tx.commit().await?;
        Ok(position)
    }

    /// Moves a position below another position of the same company, or to
    /// the root with `None` (`people.organisation.manage`). Under the company
    /// lock it refuses a parent that is the position itself or one of its
    /// descendants (`Cycle`), and a position or parent that is not of the
    /// company (`NotFound`).
    pub async fn set_parent(
        &self,
        scope: &Scope,
        company_id: i64,
        position_id: i64,
        parent_id: Option<i64>,
    ) -> Result<Position> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, MANAGE).await?;
        self.live_company(&**client, scope, company_id).await?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.organisation.set_parent"),
            )
            .await?;
        lock_company(tx.sql(), scope, company_id).await?;
        let position = positions::find(tx.sql(), company_id, position_id)
            .await?
            .ok_or(OrganisationError::NotFound)?;
        let parent = match parent_id {
            Some(id) => Some(
                positions::find(tx.sql(), company_id, id)
                    .await?
                    .ok_or(OrganisationError::NotFound)?
                    .id,
            ),
            None => None,
        };
        positions::reject_cycle(tx.sql(), company_id, position.id, parent).await?;
        let moved = positions::update_parent(&mut tx, position, parent).await?;
        tx.commit().await?;
        Ok(moved)
    }

    /// Appends a title version to a position of the company
    /// (`people.organisation.manage`); existing versions never change. The
    /// number is positive ("must be greater than 0") and unique for the
    /// position ("has already been taken"), the title 1 to 255 characters,
    /// the interval in order ("must be on or after the start") and overlapping
    /// no existing version ("violates an exclusion constraint", on
    /// `effective_from`, as Bilimbi reports the constraint).
    pub async fn record_version(
        &self,
        scope: &Scope,
        company_id: i64,
        position_id: i64,
        new: &NewVersion,
    ) -> Result<PositionVersion> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, MANAGE).await?;
        self.live_company(&**client, scope, company_id).await?;
        let valid = positions::validate_version(new).map_err(OrganisationError::Invalid)?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.organisation.record_version"),
            )
            .await?;
        lock_company(tx.sql(), scope, company_id).await?;
        let position = positions::find(tx.sql(), company_id, position_id)
            .await?
            .ok_or(OrganisationError::NotFound)?;
        positions::check_version(tx.sql(), position.id, &valid).await?;
        let version = positions::insert_version(&mut tx, position.id, valid).await?;
        tx.commit().await?;
        Ok(version)
    }

    /// Appends a substantive, acting or concurrent assignment of a working
    /// employee of the company to one of its positions
    /// (`people.organisation.manage`). An employee the Workforce read seam
    /// does not expose as working (another status, another company, an
    /// agent, missing) is `NotFound`; the kind is one of the three ("is
    /// invalid") and the interval in order.
    ///
    /// Under the company lock, a new substantive assignment first ends every
    /// earlier substantive holder who is no longer working on the day before
    /// it starts, recording `people.organisation.assignment_released` for
    /// each; a still-working substantive holder over the interval refuses it
    /// ("violates an exclusion constraint", on `effective_from`).
    pub async fn assign(
        &self,
        scope: &Scope,
        company_id: i64,
        position_id: i64,
        new: &NewAssignment,
    ) -> Result<Assignment> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, MANAGE).await?;
        self.live_company(&**client, scope, company_id).await?;
        let valid = positions::validate_assignment(new).map_err(OrganisationError::Invalid)?;
        if positions::find(&**client, company_id, position_id)
            .await?
            .is_none()
        {
            return Err(OrganisationError::NotFound);
        }
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.organisation.assign"),
            )
            .await?;
        lock_company(tx.sql(), scope, company_id).await?;
        let position = positions::find(tx.sql(), company_id, position_id)
            .await?
            .ok_or(OrganisationError::NotFound)?;
        // Recheck working membership after acquiring the company lock so a
        // change while this write waited takes effect.
        if valid.employee_id <= 0
            || !self
                .workforce
                .working_employee_ids_on(tx.sql(), scope, company_id, &[valid.employee_id])
                .await?
                .require_current()?
                .contains(&valid.employee_id)
        {
            return Err(OrganisationError::NotFound);
        }
        if valid.kind == "substantive" {
            self.release_seat(
                &mut tx,
                scope,
                company_id,
                position.id,
                valid.effective_from,
            )
            .await?;
            positions::check_substantive(
                tx.sql(),
                position.id,
                valid.effective_from,
                valid.effective_to,
            )
            .await?;
        }
        let assignment = positions::insert_assignment(&mut tx, position.id, valid).await?;
        tx.commit().await?;
        Ok(assignment)
    }

    /// Ends one assignment of the company on an inclusive day, only ever
    /// earlier (`people.organisation.manage`): the day is on or after the
    /// start ("must be on or after the start") and before the current end
    /// ("must be before the current end"). Records
    /// `people.organisation.assignment_ended` as performed by the scope's
    /// actor, in the same transaction.
    pub async fn end_assignment(
        &self,
        scope: &Scope,
        company_id: i64,
        assignment_id: i64,
        effective_to: Date,
    ) -> Result<Assignment> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, MANAGE).await?;
        self.live_company(&**client, scope, company_id).await?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.organisation.end_assignment"),
            )
            .await?;
        lock_company(tx.sql(), scope, company_id).await?;
        let assignment = positions::lock_assignment(tx.sql(), company_id, assignment_id)
            .await?
            .ok_or(OrganisationError::NotFound)?;
        positions::validate_end(&assignment, effective_to).map_err(OrganisationError::Invalid)?;
        let mut ended = positions::end_placements(
            &mut tx,
            scope,
            company_id,
            vec![(assignment, effective_to)],
            ASSIGNMENT_ENDED,
        )
        .await?;
        tx.commit().await?;
        ended
            .pop()
            .ok_or(OrganisationError::Failed(Error::Internal))
    }

    /// One offset page of the company's positions as of a day, vacancies
    /// included, by id (Bilimbi `positions/4` without `cursor:`): `page` from
    /// 1 and `page_size` from 1 to 100, else `InvalidOptions`. No capability:
    /// a company the scope cannot see is `NotFound`.
    pub async fn positions(
        &self,
        scope: &Scope,
        company_id: i64,
        as_of: Date,
        page: i64,
        page_size: i64,
    ) -> Result<Vec<PositionProjection>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.positions_on(&**client, scope, company_id, as_of, page, page_size)
            .await
    }

    /// [`Organisation::positions`] on the caller's client (the port's read).
    pub(crate) async fn positions_on(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company_id: i64,
        as_of: Date,
        page: i64,
        page_size: i64,
    ) -> Result<Vec<PositionProjection>> {
        self.live_company(client, scope, company_id).await?;
        if page < 1 || !(1..=MAX_PAGE_SIZE).contains(&page_size) {
            return Err(OrganisationError::InvalidOptions);
        }
        let rows = positions::page(client, company_id, page, page_size).await?;
        self.project(client, scope, company_id, as_of, rows).await
    }

    /// One step of the high-water keyset scan (Bilimbi `positions/4` with
    /// `cursor:`): `None` starts it and fixes the high-water mark (the last
    /// position id the sequence issued; zero before any position exists);
    /// each `next_cursor` resumes it with the same scope, company and day
    /// until `next_cursor` is `None`. A malformed cursor, or one minted for
    /// another tenant, company or day, is `InvalidCursor`; the page size may
    /// differ between steps. Positions created during the scan lie above the
    /// mark and are never mixed in, so a reconciling consumer deactivates
    /// only the ids at or below `high_water_id` it did not see.
    pub async fn scan_positions(
        &self,
        scope: &Scope,
        company_id: i64,
        as_of: Date,
        page_size: i64,
        cursor: Option<&str>,
    ) -> Result<PositionScan> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.scan_positions_on(&**client, scope, company_id, as_of, page_size, cursor)
            .await
    }

    /// [`Organisation::scan_positions`] on the caller's client (the port's read).
    pub(crate) async fn scan_positions_on(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company_id: i64,
        as_of: Date,
        page_size: i64,
        cursor: Option<&str>,
    ) -> Result<PositionScan> {
        self.live_company(client, scope, company_id).await?;
        if !(1..=MAX_PAGE_SIZE).contains(&page_size) {
            return Err(OrganisationError::InvalidOptions);
        }
        let boundary = paging::Boundary {
            tenant: scope.tenant(),
            company_id,
            day: as_of,
        };
        let bounds = paging::bounds(client, &boundary, cursor).await?;
        let mut rows = positions::keyset(
            client,
            company_id,
            bounds.last_id,
            bounds.high_water_id,
            page_size + 1,
        )
        .await?;
        let next_cursor = if rows.len() as i64 > page_size {
            rows.truncate(page_size as usize);
            rows.last()
                .map(|last| paging::encode(&boundary, last.id, bounds.high_water_id))
        } else {
            None
        };
        let positions = self.project(client, scope, company_id, as_of, rows).await?;
        Ok(PositionScan {
            positions,
            next_cursor,
            high_water_id: bounds.high_water_id,
        })
    }

    /// How many positions the company has, for an explorer's pagination.
    pub async fn count_positions(&self, scope: &Scope, company_id: i64) -> Result<i64> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.live_company(&**client, scope, company_id).await?;
        positions::count(&**client, company_id).await
    }

    /// The company as Workforce exposes it: live, of the scope's tenant.
    async fn live_company(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company_id: i64,
    ) -> Result<()> {
        self.workforce
            .company_on(client, scope, company_id)
            .await?
            .require_current()?;
        Ok(())
    }

    /// Ends, on the day before `start`, every earlier substantive holder of
    /// the position who is no longer a working employee, recording the
    /// release; a holder still working stays, and the exclusion check that
    /// follows refuses the new assignment.
    async fn release_seat(
        &self,
        tx: &mut MutationTransaction<'_>,
        scope: &Scope,
        company_id: i64,
        position_id: i64,
        start: Date,
    ) -> Result<()> {
        let holders = positions::releasable(tx.sql(), position_id, start).await?;
        if holders.is_empty() {
            return Ok(());
        }
        let eve = start.yesterday().map_err(|_| {
            OrganisationError::Invalid(vec![FieldError::new("effective_from", "is invalid")])
        })?;
        let ids: Vec<i64> = holders.iter().map(|holder| holder.employee_id).collect();
        let working = self
            .workforce
            .working_employee_ids_on(tx.sql(), scope, company_id, &ids)
            .await?
            .require_current()?;
        let released = holders
            .into_iter()
            .filter(|holder| !working.contains(&holder.employee_id))
            .map(|holder| (holder, eve))
            .collect::<Vec<_>>();
        if released.is_empty() {
            return Ok(());
        }
        positions::end_placements(tx, scope, company_id, released, ASSIGNMENT_RELEASED).await?;
        Ok(())
    }

    /// Position projections for the rows as of a day: the effective title,
    /// the working holders (checked only for the page's holders through
    /// Workforce, never the whole workforce) and the vacancy.
    async fn project(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company_id: i64,
        as_of: Date,
        rows: Vec<Position>,
    ) -> Result<Vec<PositionProjection>> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<i64> = rows.iter().map(|row| row.id).collect();
        let (placements, truncated_at) =
            positions::effective_assignments(client, &ids, as_of).await?;
        let substantive = positions::substantive_assignments(client, &ids, as_of).await?;
        let versions = positions::effective_versions(client, &ids, as_of).await?;
        let working = self
            .working_ids(
                client,
                scope,
                company_id,
                placements.iter().chain(substantive.iter()),
            )
            .await?;
        let mut holders: HashMap<i64, Vec<AssignmentProjection>> = HashMap::new();
        for placement in placements
            .into_iter()
            .filter(|placement| working.contains(&placement.employee_id))
        {
            holders
                .entry(placement.position_id)
                .or_default()
                .push(AssignmentProjection {
                    reference: AssignmentReference::new(placement.id),
                    employee_reference: reference(ReferenceType::Employee, placement.employee_id),
                    kind: placement.kind,
                });
        }
        let occupied: HashSet<i64> = substantive
            .iter()
            .filter(|assignment| working.contains(&assignment.employee_id))
            .map(|assignment| assignment.position_id)
            .collect();
        Ok(rows
            .into_iter()
            .map(|position| {
                let version = versions.get(&position.id);
                PositionProjection {
                    reference: reference(ReferenceType::Position, position.id),
                    company_reference: reference(ReferenceType::Company, company_id),
                    platform_company_id: company_id,
                    workforce_company_id: company_id,
                    code: position.code,
                    parent_reference: position
                        .parent_id
                        .map(|parent| reference(ReferenceType::Position, parent)),
                    title: version.map(|(title, _)| title.clone()),
                    version: version.map(|(_, number)| *number),
                    assignments: holders.remove(&position.id).unwrap_or_default(),
                    assignments_incomplete: truncated_at.is_some_and(|first| position.id >= first),
                    vacant: !occupied.contains(&position.id),
                }
            })
            .collect())
    }

    /// The employee ids among the assignments that are working employees of
    /// the company now, through Workforce; a read that is not current refuses.
    async fn working_ids<'a>(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company_id: i64,
        assignments: impl Iterator<Item = &'a Assignment>,
    ) -> Result<HashSet<i64>> {
        let mut ids: Vec<i64> = Vec::new();
        for assignment in assignments {
            if !ids.contains(&assignment.employee_id) {
                ids.push(assignment.employee_id);
            }
        }
        if ids.is_empty() {
            return Ok(HashSet::new());
        }
        let employees = self
            .workforce
            .working_employee_ids_on(client, scope, company_id, &ids)
            .await?
            .require_current()?;
        Ok(employees.into_iter().collect())
    }
}

/// Holds the company's writable lock for the transaction: an archived
/// company is `CompanyArchived`, a company the scope cannot see `NotFound`.
async fn lock_company(
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company_id: i64,
) -> Result<()> {
    bilibi_company::require_writable(client, scope, company_id, true)
        .await
        .map_err(|error| match error {
            WriteError::NotFound => OrganisationError::NotFound,
            WriteError::Archived => OrganisationError::CompanyArchived,
            WriteError::Access(error) => error.into(),
            _ => OrganisationError::Unauthorized,
        })?;
    // A grant can be revoked while this transaction waits for the company.
    // Every write rechecks with its own transaction after acquiring the lock.
    authorization::authorize(client, scope, company_id, MANAGE).await?;
    Ok(())
}
