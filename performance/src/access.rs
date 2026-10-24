//! Bilimbi Performance authorization, report reach and owner projections.
use crate::{Performance, PerformanceError as E, Result, Write};
use bilibi_people_organisation::PositionProjection;
use bilibi_people_workforce::authorization;
use bilibi_tenancy::Scope;
use jiff::civil::Date;
use tokio_postgres::{Client, GenericClient};

impl Performance {
    pub async fn allowed(&self, scope: &Scope, company: i64, capability: &str) -> bool {
        let Ok(client) = self.db.get().await else {
            return false;
        };
        authorization::allowed(&**client, scope, company, capability).await
    }
    pub(crate) async fn authorize(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company: i64,
        capability: &str,
    ) -> Result<()> {
        authorization::authorize(client, scope, company, capability).await?;
        self.workforce
            .company_on(client, scope, company)
            .await?
            .require_current()?;
        Ok(())
    }
    pub(crate) async fn begin<'a>(
        &self,
        client: &'a mut Client,
        scope: &Scope,
        company: i64,
        capability: &str,
        action: &str,
    ) -> Result<Write<'a>> {
        self.authorize(&*client, scope, company, capability).await?;
        if scope.impersonator().is_some() {
            return Err(E::ImpersonationRefused);
        }
        let tx = self
            .mutations
            .begin(
                client,
                scope.mutation_context(format!("people.performance.{action}")),
            )
            .await?;
        bilibi_company::require_writable(tx.sql(), scope, company, true).await?;
        self.authorize(tx.sql(), scope, company, capability).await?;
        Ok(Write {
            tx,
            tenant: scope.tenant(),
            company,
            actor: scope.user(),
        })
    }
    /// A proposal/evidence author acts as their current linked employee;
    /// re-prove that link inside the employment lock before sibling rows.
    pub(crate) async fn require_report(
        &self,
        w: &Write<'_>,
        scope: &Scope,
        employee: i64,
        capability: &str,
    ) -> Result<()> {
        let own = self
            .workforce
            .lock_self_employee(&w.tx, scope, w.company, capability)
            .await?;
        if own.employee_id == employee {
            return Err(E::OutOfReach);
        }
        self.workforce
            .lock_working_employee(&w.tx, scope, w.company, employee)
            .await?;
        let employee = self
            .workforce
            .employee(scope, w.company, employee)
            .await?
            .require_current()?;
        if employee
            .supervisor_reference
            .as_ref()
            .is_none_or(|r| r.stable_id != own.employee_id.to_string())
        {
            return Err(E::OutOfReach);
        }
        Ok(())
    }
    pub(crate) async fn independent(
        &self,
        w: &Write<'_>,
        scope: &Scope,
        employee: i64,
        author: i64,
        capability: &str,
    ) -> Result<()> {
        if scope.user() == author {
            return Err(E::SelfApproval);
        }
        if self
            .workforce
            .owns_employee(&w.tx, scope, w.company, employee, capability)
            .await?
        {
            return Err(E::SelfApproval);
        }
        Ok(())
    }
    pub(crate) async fn position(
        &self,
        scope: &Scope,
        company: i64,
        id: i64,
        day: Date,
    ) -> Result<PositionProjection> {
        let mut page = 1;
        loop {
            let rows = self
                .organisation
                .positions(scope, company, day, page, 100)
                .await
                .map_err(|e| match e {
                    bilibi_people_organisation::OrganisationError::Failed(error) => E::from(error),
                    _ => E::PositionUnavailable,
                })?;
            if let Some(row) = rows
                .iter()
                .find(|r| r.reference.stable_id == id.to_string())
            {
                return Ok(row.clone());
            }
            if rows.len() < 100 {
                return Err(E::PositionUnavailable);
            }
            page += 1;
        }
    }
}
