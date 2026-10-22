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
//! [`Workforce::direct_reports`]) need no capability, as Bilimbi's do: they
//! expose only a live company of the scope's tenant and the employees in that
//! company's working statuses, with names and work contacts read through
//! Core's owners (`directory.rs`). Which statuses count as working staff is
//! the company setting `people.workforce.working_statuses`. Each answers a
//! [`ReadResult`]: native reads are current.
//!
//! An employment references the Core Person and that person's affiliation
//! with the employer company; the row's id is the employee id every other
//! People table keys on. [`Workforce::create_employment`] authorizes
//! `people.workforce.employment.manage` for the company, holds the company's
//! writable lock in its mutation transaction and takes an owner-issued proof
//! of the affiliation before it inserts. Until the Platform's
//! `lock_for_employment_at` lands (port plan row PP2, consumed in PW1), that
//! proof is Core Affiliation's account-binding lock, which proves the same
//! person, company and link under the same owner locks; PW1 swaps it, adds
//! `update_employment`, `employment`, self-service and the position scan.
//!
//! See `README.md`.
mod contributions;
mod directory;
mod employments;
mod read_result;

pub mod authorization;

pub use contributions::{
    CAPABILITIES, EMPLOYMENT_MANAGE, EMPLOYMENT_VIEW, SETTINGS_MANAGE, WORKING_STATUSES_KEY,
};
pub use read_result::{Freshness, NotCurrent, ReadResult};

use bilibi_affiliation::AffiliationService;
use bilibi_company::{organisation, WriteError};
use bilibi_database::{Db, Error, MutationBoundary};
use bilibi_party_ids::{AffiliationId, OrganisationId, PersonId};
use bilibi_person::PersonKind;
use bilibi_principal_directory::PrincipalDirectory;
use bilibi_settings::{Settings, Target};
use bilibi_tenancy::Scope;
use jiff::civil::Date;
use serde::Serialize;
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

pub type Result<T> = std::result::Result<T, WorkforceError>;

/// Why a call did not happen (Bilimbi's refusal atoms and changeset errors).
#[derive(Debug, thiserror::Error)]
pub enum WorkforceError {
    /// A denied or unknown capability, or a company outside the actor's reach.
    #[error("Access denied")]
    Unauthorized,
    /// A company or employee the scope cannot see: missing, archived,
    /// deleted or another tenant's (indistinguishable on purpose).
    #[error("Not found")]
    NotFound,
    #[error("This company is archived and read-only")]
    CompanyArchived,
    /// More than [`LOOKUP_LIMIT`] ids, or an id that is not positive.
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
    /// company (Bilimbi `:not_linked`).
    #[error("Your account is no longer linked to a working employee in this company.")]
    NotLinked,
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReferenceType {
    Company,
    Employee,
    Position,
}
/// A stable reference into the native workforce: the source identity
/// ([`SOURCE_ID`]), what it names and the immutable id as text (Bilimbi
/// `Bilimbi.People.Workforce.Reference`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
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
/// on or after start), an optional supervisor (an employment of the same
/// company) and department (one of Core Company's), and free metadata.
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

/// A company as Core names it, for a page that renders the company it was
/// authorized for or offers the companies an operator may pick.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CompanySummary {
    pub id: i64,
    pub name: String,
    pub code: String,
    pub archived: bool,
}

/// The facade, resolved from the registry as [`SERVICE`].
pub struct Workforce {
    db: Db,
    mutations: Arc<MutationBoundary>,
    settings: Arc<Settings>,
    affiliations: Arc<AffiliationService>,
    directory: Arc<PrincipalDirectory>,
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
        let company = live_company(&**client, scope, company_id).await?;
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
        if employee_id <= 0 {
            return Err(WorkforceError::NotFound);
        }
        let client = self.db.get().await.map_err(Error::from)?;
        let company = live_company(&**client, scope, company_id).await?;
        let statuses = self
            .company_working_statuses(&**client, scope, &company)
            .await?;
        let supervisor = employments::by_ids(
            &**client,
            scope.tenant(),
            company.id,
            &[employee_id],
            &statuses,
        )
        .await?;
        if supervisor.is_empty() {
            return Err(WorkforceError::NotFound);
        }
        let rows = employments::reporting_to(
            &**client,
            scope.tenant(),
            company.id,
            employee_id,
            &statuses,
        )
        .await?;
        let employees = self
            .project(&**client, scope, company.id, &statuses, rows)
            .await?;
        Ok(ReadResult::current(employees))
    }

    /// Creates an employment (`people.workforce.employment.manage`): the
    /// person's affiliation with the company becomes an employee with a
    /// number unique in the company, a type, one of the five statuses, dates
    /// and an optional supervisor and department.
    ///
    /// Inside its mutation transaction the write holds the company's
    /// writable lock (an archived company is `CompanyArchived`), takes the
    /// Affiliation owner's proof of the affiliation (its person, company and
    /// link, under the owner's locks; a proof the owner refuses is
    /// `Unauthorized`) and checks it names this person and this company
    /// (`AffiliationMismatch`), refuses an agent Person (`NotHuman`), an
    /// affiliation already employed (`DuplicateAffiliation`) and, as
    /// `Invalid` field errors, a blank or overlong number or type, an
    /// unknown status, an end before the start, a taken number and a
    /// supervisor who is not an employment of the company.
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
        organisation::require_writable(tx.sql(), scope, company_id, true)
            .await
            .map_err(|error| match error {
                WriteError::NotFound => WorkforceError::NotFound,
                WriteError::Archived => WorkforceError::CompanyArchived,
                WriteError::Access(error) => error.into(),
                _ => WorkforceError::Unauthorized,
            })?;
        let tenant = scope.tenant();
        {
            // TODO(PW1): `AffiliationService::lock_for_employment_at` (PP2)
            // once the Platform pin carries it; the same owner locks and
            // refusals, plus the actor's reach over the named company.
            let proof = self
                .affiliations
                .lock_for_account_binding(&tx, scope, AffiliationId::new(new.affiliation_id)?)
                .await?;
            if proof.organisation().get() != company_id || proof.person().get() != new.person_id {
                return Err(WorkforceError::AffiliationMismatch);
            }
            let person = bilibi_person::identity::lock_for_account_binding(
                &tx,
                scope,
                PersonId::new(new.person_id)?,
            )
            .await?;
            if person.kind() != PersonKind::Human {
                return Err(WorkforceError::NotHuman);
            }
        }
        employments::check_new(tx.sql(), tenant, company_id, new.affiliation_id, &valid).await?;
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

    /// The working employee the signed-in account is linked to in this
    /// company, resolved now (Bilimbi `Authorization.self_employee/2`): the
    /// employment holding the affiliation the account's session carries
    /// (`Scope::actor().affiliation()`), in the explicit company, in one of
    /// the company's working statuses. An account with no affiliation, one
    /// affiliated elsewhere, or a linked employee who is not working here
    /// is `NotLinked`. Resolved on every call, so an account unlinked or
    /// relinked while a page stays connected answers for its former
    /// employee no more.
    ///
    /// Interim seam for the Attendance core (port plan row PA1): PW1 moves
    /// this beside `authorize_self` and `with_self_employee_lock`, which
    /// re-proves the link under the employment lock for self-service writes.
    pub async fn self_employee(&self, scope: &Scope, company_id: i64) -> Result<i64> {
        let client = self.db.get().await.map_err(Error::from)?;
        let Some(affiliation) = scope.actor().affiliation() else {
            return Err(WorkforceError::NotLinked);
        };
        let company = live_company(&**client, scope, company_id)
            .await
            .map_err(|_| WorkforceError::NotLinked)?;
        let statuses = self
            .company_working_statuses(&**client, scope, &company)
            .await?;
        employments::by_affiliation(&**client, scope.tenant(), company.id, affiliation.get())
            .await?
            .filter(|row| statuses.contains(&row.employment.status))
            .map(|row| row.employment.id)
            .ok_or(WorkforceError::NotLinked)
    }

    /// Authorizes a self-service capability for the company, then resolves
    /// the actor's own working employee (Bilimbi `authorize_self/3`):
    /// `Unauthorized` or `NotFound` from the capability check, else the
    /// employee id or `NotLinked`.
    pub async fn authorize_self(
        &self,
        scope: &Scope,
        company_id: i64,
        capability: &str,
    ) -> Result<i64> {
        {
            let client = self.db.get().await.map_err(Error::from)?;
            authorization::authorize(&**client, scope, company_id, capability).await?;
        }
        self.self_employee(scope, company_id).await
    }

    async fn company_working_statuses(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company: &CompanySummary,
    ) -> Result<Vec<String>> {
        let resolved = self
            .settings
            .get(
                client,
                scope,
                Target::Company(OrganisationId::new(company.id)?),
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
            &self.affiliations,
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
