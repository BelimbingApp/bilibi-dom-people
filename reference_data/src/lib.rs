//! Reference data (`people/reference_data`, Bilimbi
//! `Bilimbi.People.ReferenceData`): a company's People reference entries,
//! their aliases and its calendar exceptions.
//!
//! This is the module's one facade. Other People modules read through
//! [`ReferenceData`], resolved from the registry as [`SERVICE`]; the schema,
//! SQL and validation stay private. Every function takes the sealed [`Scope`]
//! it was handed and an explicit platform company id.
//!
//! The reads ([`ReferenceData::list`], [`ReferenceData::list_entries`],
//! [`ReferenceData::list_aliases`], [`ReferenceData::list_calendar_exceptions`])
//! are company configuration and need no capability, so Leave can read the
//! calendar exceptions and Workforce the `employment_type` kind without a
//! grant; they answer `NotFound` for a company the scope cannot see. Every
//! write authorizes `people.references.manage` for the scope's actor and the
//! target company when it runs, through Workforce's `authorization` helpers,
//! so a revoked grant refuses the next write. A write holds the company's
//! writable lock in its mutation transaction (an archived company is
//! `CompanyArchived`) and records each row it adds as a created change.
//!
//! The page's `can_manage` only decides which controls render; see
//! `README.md`.
mod contributions;
mod port;
mod store;
mod web;

pub use contributions::{MANAGE, REFERENCES_PATH};

use bilibi_company::WriteError;
use bilibi_database::{Db, Error, MutationBoundary};
use bilibi_people_workforce::{authorization, Result, WorkforceError};
use bilibi_tenancy::Scope;
use jiff::civil::Date;
use std::sync::Arc;

/// The module ID, Bilimbi's, kept because IDs are persisted data.
pub const MODULE: &str = "people/reference_data";
/// The registry service ID of the [`ReferenceData`] facade.
pub const SERVICE: &str = "people/reference_data.reference_data";

/// One reference value of a kind: `code` is the stable key, `label` what is
/// shown. An inactive entry stays listed so a record that names it still reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: i64,
    pub kind: String,
    pub code: String,
    pub label: String,
    pub active: bool,
}

/// Another label an entry answers to; unique per kind within a company.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alias {
    pub id: i64,
    pub entry_id: i64,
    pub label: String,
}

/// A day the company's calendar treats specially, with its label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalendarException {
    pub id: i64,
    pub on_date: Date,
    pub label: String,
}

/// What creates an entry: a kind (at most 80 characters), a code (100) unique
/// within the company and kind, and a label (200). The values are a company's
/// data; nothing here names a kind.
#[derive(Clone, Debug)]
pub struct NewEntry {
    pub kind: String,
    pub code: String,
    pub label: String,
    pub active: bool,
}
impl NewEntry {
    pub fn new(kind: impl Into<String>, code: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            code: code.into(),
            label: label.into(),
            active: true,
        }
    }
}

/// What creates a calendar exception: a date and a label (at most 200
/// characters), unique together within the company.
#[derive(Clone, Debug)]
pub struct NewCalendarException {
    pub on_date: Date,
    pub label: String,
}
impl NewCalendarException {
    pub fn new(on_date: Date, label: impl Into<String>) -> Self {
        Self {
            on_date,
            label: label.into(),
        }
    }
}

/// The facade, resolved from the registry as [`SERVICE`].
pub struct ReferenceData {
    db: Db,
    mutations: Arc<MutationBoundary>,
}

impl ReferenceData {
    pub(crate) fn new(db: Db, mutations: Arc<MutationBoundary>) -> Self {
        Self { db, mutations }
    }

    /// The company's entries of one kind, ordered by label, active or not
    /// (Bilimbi `list_entries/2` narrowed to a kind); a blank kind lists none.
    pub async fn list(&self, scope: &Scope, company_id: i64, kind: &str) -> Result<Vec<Entry>> {
        let client = self.db.get().await.map_err(Error::from)?;
        Self::list_on(&**client, scope, company_id, kind).await
    }

    /// The same company configuration read on a caller's connection or transaction.
    pub async fn list_on(
        client: &(impl tokio_postgres::GenericClient + Sync),
        scope: &Scope,
        company_id: i64,
        kind: &str,
    ) -> Result<Vec<Entry>> {
        known_company(client, scope, company_id).await?;
        store::entries(client, scope.tenant(), company_id, Some(kind.trim())).await
    }

    /// Every entry of the company, ordered by kind and label.
    pub async fn list_entries(&self, scope: &Scope, company_id: i64) -> Result<Vec<Entry>> {
        let client = self.db.get().await.map_err(Error::from)?;
        known_company(&**client, scope, company_id).await?;
        store::entries(&**client, scope.tenant(), company_id, None).await
    }

    /// Every alias of the company, ordered by entry and label.
    pub async fn list_aliases(&self, scope: &Scope, company_id: i64) -> Result<Vec<Alias>> {
        let client = self.db.get().await.map_err(Error::from)?;
        known_company(&**client, scope, company_id).await?;
        store::aliases(&**client, scope.tenant(), company_id).await
    }

    /// The company's calendar exceptions in date order, for Leave's counted
    /// dates; capability-free company configuration.
    pub async fn list_calendar_exceptions(
        &self,
        scope: &Scope,
        company_id: i64,
    ) -> Result<Vec<CalendarException>> {
        let client = self.db.get().await.map_err(Error::from)?;
        known_company(&**client, scope, company_id).await?;
        store::exceptions(&**client, scope.tenant(), company_id).await
    }

    /// Adds an entry (`people.references.manage`): `Invalid` field errors for
    /// a blank or overlong kind, code or label, and for a code already taken
    /// by the company's kind.
    pub async fn create_entry(
        &self,
        scope: &Scope,
        company_id: i64,
        new: &NewEntry,
    ) -> Result<Entry> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, MANAGE).await?;
        let valid = store::validate_entry(new)?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.reference_data.create_entry"),
            )
            .await?;
        writable(tx.sql(), scope, company_id).await?;
        let entry = store::insert_entry(&mut tx, scope.tenant(), company_id, valid).await?;
        tx.commit().await?;
        Ok(entry)
    }

    /// Adds an alias to one of the company's entries
    /// (`people.references.manage`); the alias takes the entry's kind. An
    /// entry of another company or none is `Invalid` on `entry_id`, a blank or
    /// overlong label is `Invalid` on `label`, and a label already used for
    /// that kind in the company is `has already been taken`.
    pub async fn add_alias(
        &self,
        scope: &Scope,
        company_id: i64,
        entry_id: i64,
        label: &str,
    ) -> Result<Alias> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, MANAGE).await?;
        let label = store::validate_alias(label)?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.reference_data.add_alias"),
            )
            .await?;
        writable(tx.sql(), scope, company_id).await?;
        let alias =
            store::insert_alias(&mut tx, scope.tenant(), company_id, entry_id, label).await?;
        tx.commit().await?;
        Ok(alias)
    }

    /// Adds a calendar exception (`people.references.manage`): `Invalid` for
    /// a blank or overlong label and for a date and label the company already
    /// has.
    pub async fn create_calendar_exception(
        &self,
        scope: &Scope,
        company_id: i64,
        new: &NewCalendarException,
    ) -> Result<CalendarException> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, MANAGE).await?;
        let valid = store::validate_exception(new)?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.reference_data.create_calendar_exception"),
            )
            .await?;
        writable(tx.sql(), scope, company_id).await?;
        let exception = store::insert_exception(&mut tx, scope.tenant(), company_id, valid).await?;
        tx.commit().await?;
        Ok(exception)
    }
}

/// A company of the scope's tenant, archived or not (Bilimbi
/// `Company.get_company/2`); a missing, deleted or other tenant's id is
/// `NotFound`.
async fn known_company(
    client: &(impl tokio_postgres::GenericClient + Sync),
    scope: &Scope,
    company_id: i64,
) -> Result<()> {
    match bilibi_company::identity(client, scope, company_id).await? {
        Some(_) => Ok(()),
        None => Err(WorkforceError::NotFound),
    }
}

/// The company's writable lock, taken inside the mutation transaction: an
/// archived company refuses the write and a missing one is `NotFound`.
async fn writable(
    sql: &(impl tokio_postgres::GenericClient + Sync),
    scope: &Scope,
    company_id: i64,
) -> Result<()> {
    bilibi_company::require_writable(sql, scope, company_id, true)
        .await
        .map(|_| ())
        .map_err(|error| match error {
            WriteError::NotFound => WorkforceError::NotFound,
            WriteError::Archived => WorkforceError::CompanyArchived,
            WriteError::Access(error) => error.into(),
            _ => WorkforceError::Unauthorized,
        })?;
    authorization::authorize(sql, scope, company_id, MANAGE).await?;
    Ok(())
}
