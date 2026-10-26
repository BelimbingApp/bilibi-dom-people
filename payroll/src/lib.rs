//! Company Payroll foundation, mirroring Bilimbi `Bilimbi.People.Payroll`.
//! This facade owns authorization, version validation, snapshots and locking.
mod calculation;
mod contributions;
mod documents;
mod download;
mod error;
mod input;
mod intake;
mod models;
mod replay;
mod setup;
mod snapshot;
mod store;
mod web;
use bilibi_database::{Db, Error, MutationBoundary, MutationTransaction};
use bilibi_party_ids::OrganisationId;
pub use bilibi_people_workforce::FieldError;
use bilibi_people_workforce::{authorization, Workforce};
use bilibi_settings::{Settings, Target};
use bilibi_tenancy::Scope;
pub use contributions::{
    APPROVE, ATTENDANCE_MAPPINGS_MANAGE, CAPABILITIES, COUNTRY_KEY, CURRENCIES_KEY, MANAGE,
    RUN_APPROVED, VIEW,
};
pub use error::PayrollError;
pub use models::*;
use std::sync::Arc;
use tokio_postgres::GenericClient;
pub const MODULE: &str = "people/payroll";
pub const SERVICE: &str = "people/payroll.payroll";
pub type Result<T> = std::result::Result<T, PayrollError>;
/// Resolve through SERVICE. Every call uses the sealed caller's scope.
pub struct Payroll {
    db: Db,
    mutations: Arc<MutationBoundary>,
    settings: Arc<Settings>,
    workforce: Arc<Workforce>,
    leave: Arc<bilibi_people_leave::Leave>,
    claims: Arc<bilibi_people_claims::Claims>,
    attendance: Arc<bilibi_people_attendance::Attendance>,
    events: Arc<bilibi_events::Events>,
    artifacts: Arc<bilibi_artifacts::Artifacts>,
}
impl Payroll {
    /// Presentation only; writes always authorize again in their transaction.
    pub async fn allowed(&self, scope: &Scope, company: i64, capability: &str) -> bool {
        let Ok(c) = self.db.get().await else {
            return false;
        };
        self.authorize(&**c, scope, company, capability)
            .await
            .is_ok()
    }
    async fn authorize(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company: i64,
        capability: &str,
    ) -> Result<()> {
        if scope.impersonator().is_some() {
            return Err(PayrollError::Unauthorized);
        }
        let summary = authorization::authorize_company(client, scope, company, capability).await?;
        if summary.archived {
            return Err(PayrollError::CompanyArchived);
        }
        Ok(())
    }
    async fn begin<'a>(
        &self,
        client: &'a mut tokio_postgres::Client,
        scope: &Scope,
        company: i64,
        capability: &str,
        action: &str,
    ) -> Result<MutationTransaction<'a>> {
        let tx = self
            .mutations
            .begin(client, scope.mutation_context(action))
            .await?;
        self.authorize(tx.sql(), scope, company, capability).await?;
        // All People writers take the Company lock first. Taking it before our
        // private lock also prevents inversion with sibling-source writes.
        bilibi_company::require_writable(tx.sql(), scope, company, true)
            .await
            .map_err(|e| match e {
                bilibi_company::WriteError::Archived => PayrollError::CompanyArchived,
                bilibi_company::WriteError::Access(e) => e.into(),
                bilibi_company::WriteError::NotFound => PayrollError::NotFound,
                _ => PayrollError::Unauthorized,
            })?;
        tx.sql()
            .query_one(
                "SELECT pg_advisory_xact_lock(hashtextextended($1,0))",
                &[&format!("people/payroll:{}:{company}", scope.tenant())],
            )
            .await?;
        // A grant can be revoked while a write waits for a company lock.
        self.authorize(tx.sql(), scope, company, capability).await?;
        Ok(tx)
    }
    async fn configuration(
        &self,
        c: &(impl GenericClient + Sync),
        scope: &Scope,
        company: i64,
    ) -> Result<(String, Vec<String>)> {
        let target =
            Target::Company(OrganisationId::new(company).map_err(|_| PayrollError::NotFound)?);
        let country = self
            .settings
            .get(c, scope, target, COUNTRY_KEY)
            .await?
            .value
            .as_str()
            .unwrap_or("")
            .to_owned();
        let currencies = serde_json::from_value(
            self.settings
                .get(c, scope, target, CURRENCIES_KEY)
                .await?
                .value,
        )
        .map_err(|_| Error::Internal)?;
        Ok((country, currencies))
    }
}
