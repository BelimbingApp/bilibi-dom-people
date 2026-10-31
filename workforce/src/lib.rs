//! Workforce (`people/workforce`, Bilimbi `Bilimbi.People.Workforce`): the
//! native workforce boundary every People module builds on, and the
//! employment record People owns.
//!
//! This is the module's one facade. Other People modules read through
//! [`Workforce`], resolved from the registry as [`SERVICE`], and authorize
//! through [`authorization`]; the schema, SQL and validation stay private.
//! Every function takes the sealed [`Scope`] it was handed and an explicit
//! platform company id. Native workforce company identity maps to the same
//! Core Company, but both axes stay separate in the returned values, and an
//! employee identity never implies a login actor.
//!
//! The working-staff reads ([`Workforce::company`], [`Workforce::employees`],
//! [`Workforce::employee`], [`Workforce::employees_by_ids`],
//! [`Workforce::direct_reports`], [`Workforce::supervisor_chain`],
//! [`Workforce::in_reach`]) need no capability, as Bilimbi's do: they expose
//! only a live company of the scope's tenant and the employees in that
//! company's working statuses, with names and work contacts read through
//! Core's owners (`directory.rs`). Which statuses count as working staff is
//! the company setting `people.workforce.working_statuses`. Each answers a
//! [`ReadResult`]: native reads are current.
//!
//! An employment references the Core Person and that person's affiliation
//! with the employer company; the row's id is the employee id every other
//! People table keys on. [`Workforce::create_employment`] takes the
//! Affiliation owner's employment proof (`lock_for_employment_at`) inside its
//! mutation transaction, [`Workforce::update_employment`] is version-fenced,
//! and [`Workforce::employment`] and [`Workforce::employment_summary`] read
//! the private facts under `people.workforce.employment.view`. Self-service
//! (`self_employee`, `authorize_self`, `lock_self_employee`,
//! `with_self_employee_lock`) resolves the signed-in account's employee on
//! every call and proves it again under the employment lock
//! (`authorization.rs`). [`Workforce::positions`] scans a company's
//! positions through the `people/organisation.positions` port
//! (`positions.rs`), `Unavailable` while Organisation is not mounted.
//!
//! See `README.md`.
mod contributions;
mod directory;
mod directory_port;
mod employee_scan;
mod employments;
mod positions;
mod read_result;
mod web;
mod workspace_directory;
pub use workspace_directory::{DirectoryEmployee, EmploymentType};

pub mod authorization;

pub use authorization::SelfEmployee;
pub use contributions::{
    CAPABILITIES, EMPLOYMENT_MANAGE, EMPLOYMENT_VIEW, SETTINGS_MANAGE, WORKING_STATUSES_KEY,
};
pub use directory_port::DIRECTORY_PORT;
pub use employee_scan::{EmployeePage, EmployeeScan, EMPLOYEE_PAGE_SIZE_MAX};
pub use positions::{
    Position, PositionAssignment, PositionPage, PositionScan, POSITIONS_PORT, POSITION_PAGE_SIZE,
    POSITION_PAGE_SIZE_MAX,
};
pub use read_result::{Freshness, NotCurrent, ReadResult};

use bilibi_affiliation::AffiliationService;
use bilibi_company::WriteError;
use bilibi_database::{Db, Error, MutationBoundary};
use bilibi_party_ids::{AffiliationId, OrganisationId, PersonId};
use bilibi_person::PersonKind;
use bilibi_ports::Ports;
use bilibi_principal_directory::PrincipalDirectory;
use bilibi_settings::{Settings, Target};
use bilibi_tenancy::{Authentication, Scope};
use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio_postgres::GenericClient;

/// The module ID, Bilimbi's, kept because IDs are persisted data.
pub const MODULE: &str = "people/workforce";
/// The registry service ID of the [`Workforce`] facade.
pub const SERVICE: &str = "people/workforce.workforce";
/// The stable source identity every native reference carries.
pub const SOURCE_ID: &str = "people/native";
/// The five employment statuses, the one fixed classification in People.
pub const EMPLOYEE_STATUSES: [&str; 5] =
    ["pending", "probation", "active", "inactive", "terminated"];
/// The most ids [`Workforce::employees_by_ids`] looks up at once.
pub const LOOKUP_LIMIT: usize = 1_000;
/// How far up [`Workforce::supervisor_chain`] walks (Bilimbi's reach depth).
pub const CHAIN_DEPTH: i32 = 12;

pub type Result<T> = std::result::Result<T, WorkforceError>;

/// Why a call did not happen (Bilimbi's refusal atoms and changeset errors).
#[derive(Debug, thiserror::Error)]
pub enum WorkforceError {
    /// A denied or unknown capability, a company outside the actor's reach,
    /// or an employment proof the Affiliation owner refused.
    #[error("Access denied")]
    Unauthorized,
    /// A company or employee the scope cannot see: missing, archived,
    /// deleted or another tenant's (indistinguishable on purpose).
    #[error("Not found")]
    NotFound,
    #[error("This company is archived and read-only")]
    CompanyArchived,
    /// More than [`LOOKUP_LIMIT`] ids, an id that is not positive, or a
    /// position page size outside 1 to 100.
    #[error("Invalid options")]
    InvalidOptions,
    /// Not a non-empty subset of the five employment statuses.
    #[error("Invalid statuses")]
    InvalidStatuses,
    /// The affiliation is already an employment.
    #[error("This affiliation already has an employment record")]
    DuplicateAffiliation,
    /// The affiliation is not this person's affiliation with this company.
    #[error("The affiliation does not belong to that person and company")]
    AffiliationMismatch,
    /// An agent Person: employees are humans.
    #[error("An agent is not an employee")]
    NotHuman,
    /// The signed-in account is not linked to a working employee of the
    /// company (Bilimbi `:not_linked`), or the link changed under the lock.
    #[error("Your account is not linked to a working employee of this company")]
    NotLinked,
    /// The version fence: the employment changed since it was read.
    #[error("The record changed. Reload before saving")]
    Stale,
    /// No mounted module answers the positions port (Organisation absent).
    #[error("Positions are unavailable: Organisation is not mounted")]
    Unavailable,
    /// A position scan cursor that is malformed or belongs to another
    /// tenant, company or day (Bilimbi `:invalid_cursor`).
    #[error("Invalid cursor")]
    InvalidCursor,
    /// Refused values, in the words a form shows beside each field.
    #[error("Invalid input")]
    Invalid(Vec<FieldError>),
    /// A database or capture failure; never a refusal.
    #[error(transparent)]
    Failed(Error),
}
impl From<Error> for WorkforceError {
    fn from(error: Error) -> Self {
        match error {
            Error::Forbidden | Error::Unauthorized => Self::Unauthorized,
            Error::NotFound => Self::NotFound,
            other => Self::Failed(other),
        }
    }
}
impl From<bilibi_party_ids::InvalidId> for WorkforceError {
    fn from(_: bilibi_party_ids::InvalidId) -> Self {
        Self::InvalidOptions
    }
}
impl From<WriteError> for WorkforceError {
    fn from(error: WriteError) -> Self {
        match error {
            WriteError::NotFound => Self::NotFound,
            WriteError::Archived => Self::CompanyArchived,
            WriteError::Access(error) => error.into(),
            _ => Self::Unauthorized,
        }
    }
}

/// One refusal of a submitted value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FieldError {
    pub field: &'static str,
    pub message: String,
}
impl FieldError {
    pub fn new(field: &'static str, message: impl Into<String>) -> Self {
        Self {
            field,
            message: message.into(),
        }
    }
}

/// What a native reference points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReferenceType {
    Company,
    Employee,
    Position,
    Assignment,
}
/// A stable reference into the native workforce: the source identity
/// ([`SOURCE_ID`]), what it names and the immutable id as text (Bilimbi
/// `Bilimbi.People.Workforce.Reference`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reference {
    pub source_id: String,
    #[serde(rename = "type")]
    pub kind: ReferenceType,
    pub stable_id: String,
}

/// The public native workforce company read model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Company {
    pub reference: Reference,
    pub platform_company_id: i64,
    pub workforce_company_id: i64,
    pub name: String,
    pub code: String,
}

/// Public workforce employee facts; no login identity and no People history.
/// `display_name` is the Person's name as Core names it; `email` is the
/// affiliation's work email when the actor may list affiliations, else
/// absent; `supervisor_reference` names the supervisor only while they are a
/// working employee of the same company.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Employee {
    pub reference: Reference,
    pub company_reference: Reference,
    pub platform_company_id: i64,
    pub workforce_company_id: i64,
    pub employee_number: String,
    pub display_name: String,
    pub email: Option<String>,
    pub supervisor_reference: Option<Reference>,
}

/// People's employment facts for one employee: the row of
/// `people_workforce_employments`. `id` is the employee id; `person_id` and
/// `affiliation_id` reference the Core Person and the person's affiliation
/// with the employer company. Identity and contact facts stay in Core.
/// `version` is the fence [`Workforce::update_employment`] takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Employment {
    pub id: i64,
    pub platform_company_id: i64,
    pub person_id: i64,
    pub affiliation_id: i64,
    pub employee_number: String,
    pub employment_type: String,
    pub status: String,
    #[serde(serialize_with = "as_optional_text")]
    pub employment_start: Option<Date>,
    #[serde(serialize_with = "as_optional_text")]
    pub employment_end: Option<Date>,
    pub supervisor_employment_id: Option<i64>,
    pub department_id: Option<i64>,
    pub metadata: Option<Value>,
    pub version: i64,
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

/// What creates an employment: the Core Person, that person's affiliation
/// with the company, the number (unique in the company), the type (a
/// company's reference data), one of the five statuses, optional dates (end
/// on or after start), an optional supervisor (a working employee of the
/// same company) and department (one of the company's), and free metadata.
#[derive(Clone, Debug)]
pub struct NewEmployment {
    pub person_id: i64,
    pub affiliation_id: i64,
    pub employee_number: String,
    pub employment_type: String,
    pub status: String,
    pub employment_start: Option<Date>,
    pub employment_end: Option<Date>,
    pub supervisor_employment_id: Option<i64>,
    pub department_id: Option<i64>,
    pub metadata: Option<Value>,
}
impl NewEmployment {
    pub fn new(
        person_id: i64,
        affiliation_id: i64,
        employee_number: impl Into<String>,
        employment_type: impl Into<String>,
        status: impl Into<String>,
    ) -> Self {
        Self {
            person_id,
            affiliation_id,
            employee_number: employee_number.into(),
            employment_type: employment_type.into(),
            status: status.into(),
            employment_start: None,
            employment_end: None,
            supervisor_employment_id: None,
            department_id: None,
            metadata: None,
        }
    }
}

/// What changes an employment (Bilimbi `update_employment/4`, plus the
/// supervisor and department the port moved here): `version` is the row
/// version the caller read, and every other field is `None` to keep the
/// stored value or `Some(value)` to replace it; the nullable facts take
/// `Some(None)` to clear. The person and affiliation never change: an
/// employment at another company is a new employment.
#[derive(Clone, Debug, Default)]
pub struct EmploymentChange {
    pub version: i64,
    pub employee_number: Option<String>,
    pub employment_type: Option<String>,
    pub status: Option<String>,
    pub employment_start: Option<Option<Date>>,
    pub employment_end: Option<Option<Date>>,
    pub supervisor_employment_id: Option<Option<i64>>,
    pub department_id: Option<Option<i64>>,
    pub metadata: Option<Option<Value>>,
}
impl EmploymentChange {
    /// A change to the row at `version` that replaces nothing yet.
    pub fn at(version: i64) -> Self {
        Self {
            version,
            ..Self::default()
        }
    }
}

/// How many employments a company has at each status (Bilimbi
/// `employment_summary/2`, counting by status now that every row is
/// People's): `by_status` lists the five statuses in canonical order with
/// their counts, `working` counts the company's working statuses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EmploymentSummary {
    pub total: i64,
    pub working: i64,
    pub by_status: Vec<StatusCount>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StatusCount {
    pub status: String,
    pub count: i64,
}

/// A company as Core names it, for a page that renders the company it was
/// authorized for or offers the companies an operator may pick.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CompanySummary {
    pub id: i64,
    pub name: String,
    pub code: String,
    pub status: String,
    pub archived: bool,
}

/// The facade, resolved from the registry as [`SERVICE`].
pub struct Workforce {
    db: Db,
    mutations: Arc<MutationBoundary>,
    settings: Arc<Settings>,
    affiliations: Arc<AffiliationService>,
    directory: Arc<PrincipalDirectory>,
    authentication: Arc<Authentication>,
    ports: Arc<Ports>,
}

impl Workforce {
    /// The five employment statuses.
    pub fn employee_statuses() -> &'static [&'static str] {
        &EMPLOYEE_STATUSES
    }

    /// The statuses the company counts as working staff
    /// (`people.workforce.working_statuses`; default `probation`, `active`).
    pub async fn working_statuses(
        &self,
        scope: &Scope,
        company_id: i64,
    ) -> Result<ReadResult<Vec<String>>> {
        let client = self.db.get().await.map_err(Error::from)?;
        let company = live_company(&**client, scope, company_id).await?;
        let statuses = self
            .company_working_statuses(&**client, scope, &company)
            .await?;
        Ok(ReadResult::current(statuses))
    }

    /// Replaces the company's working statuses
    /// (`people.workforce.settings.manage`): a non-empty subset of the five,
    /// stored in their canonical order.
    pub async fn put_working_statuses(
        &self,
        scope: &Scope,
        company_id: i64,
        statuses: &[String],
    ) -> Result<Vec<String>> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, SETTINGS_MANAGE).await?;
        let company = live_company(&**client, scope, company_id).await?;
        let statuses = normalize_statuses(statuses)?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.workforce.put_working_statuses"),
            )
            .await?;
        bilibi_company::require_writable(tx.sql(), scope, company_id, true).await?;
        // A grant may have been revoked while this write waited for the
        // company lock. Judge it on the transaction after that wait.
        authorization::authorize(tx.sql(), scope, company_id, SETTINGS_MANAGE).await?;
        self.settings
            .put(
                &mut tx,
                scope,
                Target::Company(OrganisationId::new(company.id)?),
                WORKING_STATUSES_KEY,
                json!(statuses),
            )
            .await?;
        tx.commit().await?;
        Ok(statuses)
    }

    /// The company's native workforce identity (Bilimbi `company/2`).
    pub async fn company(&self, scope: &Scope, company_id: i64) -> Result<ReadResult<Company>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.company_on(&**client, scope, company_id).await
    }

    /// The company read on a caller's connection or transaction. Ports use
    /// this to share the caller's view without renting another connection.
    pub async fn company_on(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company_id: i64,
    ) -> Result<ReadResult<Company>> {
        let company = live_company(client, scope, company_id).await?;
        Ok(ReadResult::current(Company {
            reference: Reference::company(company.id),
            platform_company_id: company.id,
            workforce_company_id: company.id,
            name: company.name,
            code: company.code,
        }))
    }

    /// The company's working employees by id (Bilimbi `employees/2`): the
    /// employments in the company's working statuses, named through Core.
    pub async fn employees(
        &self,
        scope: &Scope,
        company_id: i64,
    ) -> Result<ReadResult<Vec<Employee>>> {
        let client = self.db.get().await.map_err(Error::from)?;
        let company = live_company(&**client, scope, company_id).await?;
        let statuses = self
            .company_working_statuses(&**client, scope, &company)
            .await?;
        let rows =
            employments::in_statuses(&**client, scope.tenant(), company.id, &statuses).await?;
        let employees = self
            .project(&**client, scope, company.id, &statuses, rows)
            .await?;
        Ok(ReadResult::current(employees))
    }

    /// Employee directory summaries at every employment status, for a
    /// company's assignment pickers (Bilimbi `Core.Employee.list_employees`
    /// and `employees_belong_to_company?` as used by Claims). These contain
    /// names and employee numbers, not private employment facts. With ids,
    /// accepts at most [`LOOKUP_LIMIT`] positive employee ids; otherwise
    /// lists the company by id. Working-staff reads keep using [`Self::employees`].
    pub async fn employees_at_any_status(
        &self,
        scope: &Scope,
        company_id: i64,
        employee_ids: Option<&[i64]>,
    ) -> Result<ReadResult<Vec<Employee>>> {
        let client = self.db.get().await.map_err(Error::from)?;
        let company = live_company(&**client, scope, company_id).await?;
        let statuses = self
            .company_working_statuses(&**client, scope, &company)
            .await?;
        let all_statuses: Vec<String> = EMPLOYEE_STATUSES
            .iter()
            .map(|status| (*status).to_owned())
            .collect();
        let rows = match employee_ids {
            Some(ids) => {
                let ids = lookup_ids(ids)?;
                employments::by_ids(&**client, scope.tenant(), company.id, &ids, &all_statuses)
                    .await?
            }
            None => {
                employments::in_statuses(&**client, scope.tenant(), company.id, &all_statuses)
                    .await?
            }
        };
        let employees = self
            .project(&**client, scope, company.id, &statuses, rows)
            .await?;
        Ok(ReadResult::current(employees))
    }

    /// The working employees among at most [`LOOKUP_LIMIT`] ids (Bilimbi
    /// `employees_by_ids/3`), without reading the whole workforce; ids that
    /// are not positive, or too many, are `InvalidOptions`.
    pub async fn employees_by_ids(
        &self,
        scope: &Scope,
        company_id: i64,
        employee_ids: &[i64],
    ) -> Result<ReadResult<Vec<Employee>>> {
        let client = self.db.get().await.map_err(Error::from)?;
        let company = live_company(&**client, scope, company_id).await?;
        let ids = lookup_ids(employee_ids)?;
        let statuses = self
            .company_working_statuses(&**client, scope, &company)
            .await?;
        let rows =
            employments::by_ids(&**client, scope.tenant(), company.id, &ids, &statuses).await?;
        let employees = self
            .project(&**client, scope, company.id, &statuses, rows)
            .await?;
        Ok(ReadResult::current(employees))
    }

    /// The working ids among at most [`LOOKUP_LIMIT`] employee ids, on the
    /// caller's connection or transaction. Applies the same company and
    /// working-status rules as [`Self::employees_by_ids`], without resolving
    /// names or contacts when a caller needs only membership.
    pub async fn working_employee_ids_on(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company_id: i64,
        employee_ids: &[i64],
    ) -> Result<ReadResult<Vec<i64>>> {
        let company = live_company(client, scope, company_id).await?;
        let ids = lookup_ids(employee_ids)?;
        let statuses = self
            .company_working_statuses(client, scope, &company)
            .await?;
        let rows = employments::by_ids(client, scope.tenant(), company.id, &ids, &statuses).await?;
        Ok(ReadResult::current(
            rows.into_iter().map(|row| row.employment.id).collect(),
        ))
    }

    /// Matches at most [`LOOKUP_LIMIT`] Core staff Person ids to
    /// employment ids in this company, at any status. Identity references
    /// only: this neither exposes employment facts nor grants authority.
    /// Reminder owners combine this with Core User's proven company accounts.
    pub async fn employment_people_on(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company_id: i64,
        person_ids: &[i64],
    ) -> Result<ReadResult<Vec<(i64, i64)>>> {
        let company = live_company(client, scope, company_id).await?;
        let ids = lookup_ids(person_ids)?;
        let links = employments::person_links(client, scope.tenant(), company.id, &ids).await?;
        Ok(ReadResult::current(links))
    }

    /// One working employee of the company (Bilimbi `employee/3`); an
    /// employee at another status, of another company, or missing is
    /// `NotFound`.
    pub async fn employee(
        &self,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
    ) -> Result<ReadResult<Employee>> {
        if employee_id <= 0 {
            return Err(WorkforceError::NotFound);
        }
        let client = self.db.get().await.map_err(Error::from)?;
        let company = live_company(&**client, scope, company_id).await?;
        let statuses = self
            .company_working_statuses(&**client, scope, &company)
            .await?;
        let rows = employments::by_ids(
            &**client,
            scope.tenant(),
            company.id,
            &[employee_id],
            &statuses,
        )
        .await?;
        let mut employees = self
            .project(&**client, scope, company.id, &statuses, rows)
            .await?;
        match employees.pop() {
            Some(employee) => Ok(ReadResult::current(employee)),
            None => Err(WorkforceError::NotFound),
        }
    }

    /// The working employees who report to one working employee of the
    /// company, by id; a supervisor who is not a working employee is
    /// `NotFound`.
    pub async fn direct_reports(
        &self,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
    ) -> Result<ReadResult<Vec<Employee>>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.direct_reports_on(&**client, scope, company_id, employee_id)
            .await
    }

    /// The same direct-report read on the caller's connection. Writes holding
    /// workplace locks use this entry point instead of reopening the pool.
    pub async fn direct_reports_on(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
    ) -> Result<ReadResult<Vec<Employee>>> {
        if employee_id <= 0 {
            return Err(WorkforceError::NotFound);
        }
        let company = live_company(client, scope, company_id).await?;
        let statuses = self
            .company_working_statuses(client, scope, &company)
            .await?;
        let supervisor = employments::by_ids(
            client,
            scope.tenant(),
            company.id,
            &[employee_id],
            &statuses,
        )
        .await?;
        if supervisor.is_empty() {
            return Err(WorkforceError::NotFound);
        }
        let rows =
            employments::reporting_to(client, scope.tenant(), company.id, employee_id, &statuses)
                .await?;
        let employees = self
            .project(client, scope, company.id, &statuses, rows)
            .await?;
        Ok(ReadResult::current(employees))
    }

    /// The working employee's supervisors, nearest first, as far as the
    /// chain stays within the company's working employees and at most
    /// [`CHAIN_DEPTH`] levels (Bilimbi's reach depth); an employee who is
    /// not a working employee is `NotFound`. Capability-free, like the other
    /// working-staff reads: it reveals only ids the reads already expose.
    pub async fn supervisor_chain(
        &self,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
    ) -> Result<ReadResult<Vec<i64>>> {
        if employee_id <= 0 {
            return Err(WorkforceError::NotFound);
        }
        let client = self.db.get().await.map_err(Error::from)?;
        let company = live_company(&**client, scope, company_id).await?;
        let statuses = self
            .company_working_statuses(&**client, scope, &company)
            .await?;
        let chain = employments::supervisor_chain(
            &**client,
            scope.tenant(),
            company.id,
            employee_id,
            &statuses,
        )
        .await?
        .ok_or(WorkforceError::NotFound)?;
        Ok(ReadResult::current(chain))
    }

    /// Whether `employee_id` is one of the people `manager_id` may act on
    /// (Bilimbi `Access.within?/2` for a team reach): a working employee
    /// below the manager in the supervisor chain, directly or through
    /// others, never the manager themselves. An employee the working-staff
    /// reads do not expose is out of reach, not an error.
    pub async fn in_reach(
        &self,
        scope: &Scope,
        company_id: i64,
        manager_id: i64,
        employee_id: i64,
    ) -> Result<ReadResult<bool>> {
        if manager_id <= 0 || employee_id <= 0 || manager_id == employee_id {
            return Ok(ReadResult::current(false));
        }
        let client = self.db.get().await.map_err(Error::from)?;
        let company = live_company(&**client, scope, company_id).await?;
        let statuses = self
            .company_working_statuses(&**client, scope, &company)
            .await?;
        let chain = employments::supervisor_chain(
            &**client,
            scope.tenant(),
            company.id,
            employee_id,
            &statuses,
        )
        .await?;
        Ok(ReadResult::current(
            chain.is_some_and(|chain| chain.contains(&manager_id)),
        ))
    }

    /// One employee's private employment facts at any status (Bilimbi
    /// `employment/3`): `people.workforce.employment.view` now; an employee
    /// of another company or missing is `NotFound`.
    pub async fn employment(
        &self,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
    ) -> Result<ReadResult<Employment>> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, EMPLOYMENT_VIEW).await?;
        let company = live_company(&**client, scope, company_id).await?;
        if employee_id <= 0 {
            return Err(WorkforceError::NotFound);
        }
        let row = employments::find(&**client, scope.tenant(), company.id, employee_id)
            .await?
            .ok_or(WorkforceError::NotFound)?;
        Ok(ReadResult::current(row.employment))
    }

    /// The company's employment counts by status (Bilimbi
    /// `employment_summary/2`): `people.workforce.employment.view` now.
    pub async fn employment_summary(
        &self,
        scope: &Scope,
        company_id: i64,
    ) -> Result<ReadResult<EmploymentSummary>> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, EMPLOYMENT_VIEW).await?;
        let company = live_company(&**client, scope, company_id).await?;
        let statuses = self
            .company_working_statuses(&**client, scope, &company)
            .await?;
        let summary =
            employments::summary(&**client, scope.tenant(), company.id, &statuses).await?;
        Ok(ReadResult::current(summary))
    }

    /// Creates an employment (`people.workforce.employment.manage`): the
    /// person's affiliation with the company becomes an employee with a
    /// number unique in the company, a type, one of the five statuses, dates
    /// and an optional supervisor and department.
    ///
    /// Inside its mutation transaction the write takes the Affiliation
    /// owner's employment proof at this company
    /// (`AffiliationService::lock_for_employment_at`: tenant → Company →
    /// Person → Affiliation locks; the company must be live and within the
    /// actor's reach, the affiliation linked and of that company, the Person
    /// neither retired nor system; a refused proof is `Unauthorized`, an
    /// archived company `CompanyArchived`), holds the company's writable
    /// lock, checks the proof names this person (`AffiliationMismatch`),
    /// refuses an agent Person (`NotHuman`), an affiliation already employed
    /// (`DuplicateAffiliation`) and, as `Invalid` field errors, a blank or
    /// overlong number or type, an unknown status, an end before the start,
    /// a taken number, a supervisor who is not a working employee of the
    /// company and a department that is not the company's.
    pub async fn create_employment(
        &self,
        scope: &Scope,
        company_id: i64,
        new: &NewEmployment,
    ) -> Result<Employment> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, EMPLOYMENT_MANAGE).await?;
        let valid = employments::validate(new).map_err(WorkforceError::Invalid)?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.workforce.create_employment"),
            )
            .await?;
        // Classify an archived company before the proof locks anything: the
        // proof answers every refusal as Forbidden.
        bilibi_company::require_writable(tx.sql(), scope, company_id, false).await?;
        let tenant = scope.tenant();
        let company = OrganisationId::new(company_id)?;
        {
            let proof = self
                .affiliations
                .lock_for_employment_at(
                    &tx,
                    scope,
                    AffiliationId::new(new.affiliation_id)?,
                    company,
                )
                .await?;
            let identity = proof.identity();
            if identity.organisation() != company || identity.person().get() != new.person_id {
                return Err(WorkforceError::AffiliationMismatch);
            }
            let person =
                bilibi_person::lock_for_account_binding(&tx, scope, PersonId::new(new.person_id)?)
                    .await?;
            if person.kind() != PersonKind::Human {
                return Err(WorkforceError::NotHuman);
            }
        }
        // The proof locked the company writable in the global order; this is
        // the explicit company lock every People write holds.
        bilibi_company::require_writable(tx.sql(), scope, company_id, true).await?;
        authorization::authorize(tx.sql(), scope, company_id, EMPLOYMENT_MANAGE).await?;
        let statuses = self
            .working_statuses_of(tx.sql(), scope, company_id)
            .await?;
        employments::check_new(
            tx.sql(),
            tenant,
            company_id,
            new.affiliation_id,
            &valid,
            &statuses,
        )
        .await?;
        let employment = employments::insert(
            &mut tx,
            tenant,
            company_id,
            new.person_id,
            new.affiliation_id,
            valid,
        )
        .await?;
        tx.commit().await?;
        Ok(employment)
    }

    /// Changes an employment's number, type, status, dates, supervisor,
    /// department or metadata (`people.workforce.employment.manage`;
    /// Bilimbi `update_employment/4`), fenced on the version the caller
    /// read: the row is locked inside the mutation transaction under the
    /// company's writable lock, a version that moved is `Stale`, and the
    /// stored row carries `version + 1`. An employee of another company or
    /// missing is `NotFound`. The same `Invalid` field errors as
    /// [`Workforce::create_employment`], plus a supervisor who would report
    /// to this employee, directly or through others ("would create a
    /// reporting cycle").
    pub async fn update_employment(
        &self,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
        change: &EmploymentChange,
    ) -> Result<Employment> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, EMPLOYMENT_MANAGE).await?;
        if employee_id <= 0 {
            return Err(WorkforceError::NotFound);
        }
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.workforce.update_employment"),
            )
            .await?;
        bilibi_company::require_writable(tx.sql(), scope, company_id, true).await?;
        authorization::authorize(tx.sql(), scope, company_id, EMPLOYMENT_MANAGE).await?;
        let tenant = scope.tenant();
        let current = employments::lock(tx.sql(), tenant, company_id, employee_id)
            .await?
            .ok_or(WorkforceError::NotFound)?;
        if current.version != change.version {
            return Err(WorkforceError::Stale);
        }
        let valid =
            employments::validate_change(&current, change).map_err(WorkforceError::Invalid)?;
        let statuses = self
            .working_statuses_of(tx.sql(), scope, company_id)
            .await?;
        employments::check_change(tx.sql(), tenant, company_id, &current, &valid, &statuses)
            .await?;
        let employment = employments::update(&mut tx, current, valid).await?;
        tx.commit().await?;
        Ok(employment)
    }

    /// Reads a bounded page of the company's positions through the
    /// `people/organisation.positions` port (Bilimbi `positions/4` with
    /// `cursor:`): a keyset scan with a high-water mark, each page at most
    /// [`POSITION_PAGE_SIZE_MAX`] rows. Pass `cursor: None` to start and the
    /// page's `next_cursor` to continue, with the same company and `as_of`,
    /// until it is `None`; only then may a consumer treat ids at or below
    /// `high_water_id` that it did not see as gone. `Unavailable` while no
    /// mounted module answers the port, `InvalidCursor` for a cursor of
    /// another scan, `InvalidOptions` for a page size outside 1 to 100.
    pub async fn positions(
        &self,
        scope: &Scope,
        company_id: i64,
        scan: &PositionScan,
    ) -> Result<ReadResult<PositionPage>> {
        let client = self.db.get().await.map_err(Error::from)?;
        let company = live_company(&**client, scope, company_id).await?;
        let page = positions::scan(&self.ports, &**client, scope, company.id, scan).await?;
        Ok(ReadResult::current(page))
    }

    /// Whether a mounted module answers the positions port (Bilimbi
    /// `positions_available?/0`), for a page that withholds a control.
    pub fn positions_available(&self) -> bool {
        self.ports.available(POSITIONS_PORT)
    }

    async fn company_working_statuses(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company: &CompanySummary,
    ) -> Result<Vec<String>> {
        self.working_statuses_of(client, scope, company.id).await
    }

    async fn working_statuses_of(
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
                WORKING_STATUSES_KEY,
            )
            .await?;
        let statuses: Vec<String> = resolved
            .value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|status| status.as_str().map(str::to_owned))
            .collect();
        if statuses.is_empty() {
            // The definition refuses an empty list; a stored value that
            // escaped it still reads as the default rather than hiding everyone.
            return Ok(vec!["probation".into(), "active".into()]);
        }
        Ok(statuses)
    }

    /// Employee records for the rows: names and contacts through Core, the
    /// supervisor reference only for a working supervisor.
    async fn project(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company_id: i64,
        statuses: &[String],
        rows: Vec<employments::Row_>,
    ) -> Result<Vec<Employee>> {
        let person_ids: Vec<i64> = rows.iter().map(|row| row.employment.person_id).collect();
        let affiliation_ids: Vec<i64> = rows
            .iter()
            .map(|row| row.employment.affiliation_id)
            .collect();
        let identities = directory::resolve(
            client,
            scope,
            &self.directory,
            company_id,
            &person_ids,
            &affiliation_ids,
        )
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let supervisor_reference = match (
                    row.employment.supervisor_employment_id,
                    row.supervisor_status.as_deref(),
                ) {
                    (Some(id), Some(status)) if statuses.iter().any(|s| s == status) => {
                        Some(Reference::employee(id))
                    }
                    _ => None,
                };
                Employee {
                    reference: Reference::employee(row.employment.id),
                    company_reference: Reference::company(company_id),
                    platform_company_id: company_id,
                    workforce_company_id: company_id,
                    employee_number: row.employment.employee_number,
                    display_name: identities.name(row.employment.person_id),
                    email: identities.email(row.employment.affiliation_id),
                    supervisor_reference,
                }
            })
            .collect())
    }
}

/// A company the scope can see that is not archived; missing, cross-tenant,
/// archived and malformed ids are indistinguishable (`NotFound`).
async fn live_company(
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company_id: i64,
) -> Result<CompanySummary> {
    if scope.principal().is_none()
        && !bilibi_company::reachable(
            client,
            scope,
            bilibi_party_ids::OrganisationId::new(company_id)?,
        )
        .await?
    {
        return Err(WorkforceError::Unauthorized);
    }
    match authorization::summary(client, scope, company_id).await? {
        Some(company) if !company.archived => Ok(company),
        _ => Err(WorkforceError::NotFound),
    }
}

fn lookup_ids(ids: &[i64]) -> Result<Vec<i64>> {
    if ids.len() > LOOKUP_LIMIT || ids.iter().any(|id| *id <= 0) {
        return Err(WorkforceError::InvalidOptions);
    }
    let mut unique = Vec::with_capacity(ids.len());
    for id in ids {
        if !unique.contains(id) {
            unique.push(*id);
        }
    }
    Ok(unique)
}

/// A non-empty subset of the five statuses, in their canonical order.
fn normalize_statuses(statuses: &[String]) -> Result<Vec<String>> {
    if statuses.is_empty()
        || !statuses
            .iter()
            .all(|status| EMPLOYEE_STATUSES.contains(&status.as_str()))
    {
        return Err(WorkforceError::InvalidStatuses);
    }
    Ok(EMPLOYEE_STATUSES
        .iter()
        .filter(|status| statuses.iter().any(|given| given == *status))
        .map(|status| (*status).to_owned())
        .collect())
}
