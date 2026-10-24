//! Bilimbi Progression authorization, report reach and owner projections.
use crate::{Progression, ProgressionError as E, Result, Write};
use bilibi_people_workforce::authorization;
use bilibi_tenancy::Scope;
use tokio_postgres::{Client, GenericClient};

impl Progression {
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
                scope.mutation_context(format!("people.progression.{action}")),
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
}
