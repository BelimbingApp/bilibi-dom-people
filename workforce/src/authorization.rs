//! Per-operation authorization for People (Bilimbi
//! `Bilimbi.People.Workforce.Authorization`): the one boundary every People
//! facade authorizes through, at the public API so pages, jobs and other
//! adapters share it, and the signed-in actor's own employee.
//!
//! A page rechecks its route capability before acting, and a `can_*` only
//! decides which controls it renders: an administrator can revoke a grant,
//! or unlink the login account from its employee, while the page stays
//! connected. Every People write, and every read of private employee facts,
//! therefore calls this module first with the scope it was handed, so a
//! revocation takes effect on the very next operation.
//!
//! Pass the sealed [`Scope`], never an actor or an actor ID: the actor is
//! read from the scope the authentication edge sealed. A system principal
//! acts through `bilibi_authz::SystemScope`, a different type, so a system
//! scope cannot reach these helpers at all, which is Bilimbi's "system scope
//! refused". The capability is evaluated now, by Base Authz, and the company
//! axis by Core Company's `organisation::authorize_target` with that same
//! actor: the actor's own company, or any other company of the tenant with
//! `admin.company.tenant-wide.manage`.
//!
//! Self-service operations act on the employee the login account is linked
//! to *now*. [`Workforce::self_employee`] re-proves the account through
//! Base Tenancy on every call and reads `Scope::actor().affiliation()` from
//! that fresh scope (Bilimbi read `User.get_user/3`'s employee link), and
//! [`Workforce::lock_self_employee`] proves it again inside the transaction
//! that holds the employment row's lock, so a self-service write cannot
//! commit for an employee the account no longer belongs to.
use crate::{CompanySummary, Result, Workforce, WorkforceError};
use bilibi_company::organisation::{self, Directory};
use bilibi_database::{Error, MutationTransaction};
use bilibi_party_ids::OrganisationId;
use bilibi_tenancy::{OrganisationDirectory, ProofClient, Scope};
use serde::Serialize;
use std::{future::Future, pin::Pin};
use tokio_postgres::GenericClient;

/// Authorizes the scope's signed-in actor for one capability in one company.
///
/// Returns the company id. `Unauthorized` covers a denied or unknown
/// capability and a company outside the actor's reach; `NotFound` covers a
/// company the scope cannot see at all (missing, deleted or another tenant's).
pub async fn authorize(
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company_id: i64,
    capability: &str,
) -> Result<i64> {
    organisation::authorize_target(client, scope, company_id, capability)
        .await
        .map_err(refusal)
}

/// Company authorization on the caller's transaction for object-safe owner
/// callbacks (Artifacts). Uses Core Company's same target proof; never opens
/// a second connection while the owner holds its reservation/publication lock.
pub async fn authorize_company_proof(
    client: &dyn ProofClient,
    scope: &Scope,
    company_id: i64,
    capability: &str,
) -> Result<bilibi_tenancy::NamedOrganisation> {
    Directory
        .target(client, scope, OrganisationId::new(company_id)?, capability)
        .await
        .map_err(refusal)
}

/// Authorizes as [`authorize`] does and returns the company summary, for a
/// caller that renders the company it was authorized for.
pub async fn authorize_company(
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company_id: i64,
    capability: &str,
) -> Result<CompanySummary> {
    authorize(client, scope, company_id, capability).await?;
    summary(client, scope, company_id)
        .await?
        .ok_or(WorkforceError::NotFound)
}

/// The live, unarchived companies the scope's signed-in actor may target for
/// one capability (Bilimbi `Company.list_selectable_companies/2`), sorted by
/// name: none without the capability, the actor's own company, or every
/// company of the tenant with the tenant-wide reach.
pub async fn selectable_companies(
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    capability: &str,
) -> Result<Vec<CompanySummary>> {
    let named = Directory
        .selectable(client as &dyn ProofClient, scope, capability)
        .await
        .map_err(refusal)?;
    let mut companies = Vec::with_capacity(named.len());
    for company in named {
        if let Some(summary) = summary(client, scope, company.id.get()).await? {
            companies.push(summary);
        }
    }
    Ok(companies)
}

/// Whether the actor currently holds the capability for the company, for
/// deciding which controls to render. It is not authority for an operation:
/// the operation calls [`authorize`] itself when it runs.
pub async fn allowed(
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company_id: i64,
    capability: &str,
) -> bool {
    authorize(client, scope, company_id, capability)
        .await
        .is_ok()
}

/// One company of the scope's tenant as Core Company names it, or `None`
/// for a missing, deleted or other-tenant id.
pub(crate) async fn summary(
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company_id: i64,
) -> Result<Option<CompanySummary>> {
    let Some(identity) = organisation::identity(client, scope, company_id).await? else {
        return Ok(None);
    };
    let Some(named) = Directory
        .lookup(
            client as &dyn ProofClient,
            scope,
            OrganisationId::new(company_id)?,
        )
        .await?
    else {
        return Ok(None);
    };
    Ok(Some(CompanySummary {
        id: company_id,
        name: identity.display_name,
        code: identity.code,
        archived: named.archived,
    }))
}

/// Core's answers in Bilimbi's two refusal words.
fn refusal(error: Error) -> WorkforceError {
    match error {
        Error::Forbidden | Error::Unauthorized => WorkforceError::Unauthorized,
        Error::NotFound => WorkforceError::NotFound,
        other => WorkforceError::Failed(other),
    }
}

/// The signed-in actor's own employee in one company (Bilimbi's
/// `%{actor, employee_id}` self): the employment id every People table keys
/// on, with the Core Person and affiliation it holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SelfEmployee {
    pub employee_id: i64,
    pub person_id: i64,
    pub affiliation_id: i64,
}

/// A self-service write: the caller's work inside the transaction that
/// holds the employment lock (see [`Workforce::with_self_employee_lock`]).
pub type SelfWrite<'b, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'b>>;

impl Workforce {
    /// The signed-in account's employee at any employment status, or None
    /// when unlinked. Re-proves the account on the supplied client, so sibling
    /// workflows can refuse subject self-approval inside their transaction.
    /// This is an identity read, not permission to act on the employee.
    pub async fn linked_employee(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company_id: i64,
    ) -> Result<Option<i64>> {
        let company = summary(client, scope, company_id)
            .await?
            .filter(|c| !c.archived)
            .ok_or(WorkforceError::NotFound)?;
        let affiliation = match self.fresh_affiliation(client, scope).await {
            Ok(id) => id,
            Err(WorkforceError::NotLinked) => return Ok(None),
            Err(error) => return Err(error),
        };
        let statuses: Vec<String> = crate::EMPLOYEE_STATUSES
            .iter()
            .map(|s| (*s).into())
            .collect();
        Ok(crate::employments::by_affiliation(
            client,
            scope.tenant(),
            company.id,
            affiliation,
            &statuses,
        )
        .await?
        .map(|row| row.employment.id))
    }

    /// Locks a workflow subject at any employment status without exposing
    /// private employment facts. Use before checking subject independence.
    /// The company lock also serializes account relinking. The caller must
    /// authorize its own capability, and the company must be writable.
    pub async fn lock_employee_subject(
        &self,
        tx: &MutationTransaction<'_>,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
    ) -> Result<()> {
        organisation::require_writable(tx.sql(), scope, company_id, true).await?;
        crate::employments::lock(tx.sql(), scope.tenant(), company_id, employee_id)
            .await?
            .ok_or(WorkforceError::NotFound)?;
        Ok(())
    }

    /// Locks a sibling workflow's subject before its own rows. The company
    /// must be writable and the employee must still be working. Callers
    /// authorize their own capability; this exposes no employment facts.
    pub async fn lock_working_employee(
        &self,
        tx: &MutationTransaction<'_>,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
    ) -> Result<()> {
        organisation::require_writable(tx.sql(), scope, company_id, true).await?;
        let locked = crate::employments::lock(tx.sql(), scope.tenant(), company_id, employee_id)
            .await?
            .ok_or(WorkforceError::NotFound)?;
        let statuses = self
            .working_statuses_of(tx.sql(), scope, company_id)
            .await?;
        if !statuses.contains(&locked.status) {
            return Err(WorkforceError::NotFound);
        }
        Ok(())
    }

    /// The working employee the signed-in account is linked to in this
    /// company, resolved now (Bilimbi `self_employee/2`): the account is
    /// re-proved through Base Tenancy on this call, its affiliation facet
    /// read from the fresh scope, and the employment holding that
    /// affiliation in this company found in the company's working statuses.
    /// `NotLinked` covers a frozen account, an account without an
    /// affiliation link, an affiliation of another company, and a linked
    /// employee who is not a current working employee; a company the scope
    /// cannot see is `NotFound`.
    pub async fn self_employee(&self, scope: &Scope, company_id: i64) -> Result<SelfEmployee> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.self_employee_on(&**client, scope, company_id).await
    }

    /// Authorizes the self-service capability for the company, then resolves
    /// the actor's own employee (Bilimbi `authorize_self/3`).
    pub async fn authorize_self(
        &self,
        scope: &Scope,
        company_id: i64,
        capability: &str,
    ) -> Result<SelfEmployee> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorize(&**client, scope, company_id, capability).await?;
        self.self_employee_on(&**client, scope, company_id).await
    }

    /// Proves the actor's own employee inside the caller's mutation
    /// transaction (the half of Bilimbi `with_self_employee_lock/4` a sibling
    /// facade calls from its own transaction): authorizes the capability,
    /// resolves the linked employee now, holds the company's writable lock
    /// (an archived company is `CompanyArchived`), locks that employment row
    /// (`FOR NO KEY UPDATE`, the lock account relinking and employment
    /// changes wait on), then re-proves the account's link and the working
    /// status under the lock. A link that changed between the resolution and
    /// the lock is `NotLinked`. Call it before the sibling's own row locks;
    /// the lock order is Company → Employment → the sibling's rows.
    pub async fn lock_self_employee(
        &self,
        tx: &MutationTransaction<'_>,
        scope: &Scope,
        company_id: i64,
        capability: &str,
    ) -> Result<SelfEmployee> {
        authorize(tx.sql(), scope, company_id, capability).await?;
        let me = self.self_employee_on(tx.sql(), scope, company_id).await?;
        organisation::require_writable(tx.sql(), scope, company_id, true).await?;
        let locked = crate::employments::lock(tx.sql(), scope.tenant(), company_id, me.employee_id)
            .await?
            .ok_or(WorkforceError::NotLinked)?;
        // The link and the working status, proved again under the lock.
        let fresh = self.fresh_affiliation(tx.sql(), scope).await?;
        let statuses = self
            .working_statuses_of(tx.sql(), scope, company_id)
            .await?;
        if fresh != locked.affiliation_id || !statuses.contains(&locked.status) {
            return Err(WorkforceError::NotLinked);
        }
        Ok(me)
    }

    /// Whether this account is currently linked to a named employee, at any
    /// employment status. Decision owners use this after their capability
    /// check to refuse acting on their own records. Company → Employment
    /// locks serialize relinking; the account is re-proved under that lock.
    /// An unlinked account returns false; a missing employee is NotFound.
    pub async fn owns_employee(
        &self,
        tx: &MutationTransaction<'_>,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
        capability: &str,
    ) -> Result<bool> {
        authorize(tx.sql(), scope, company_id, capability).await?;
        organisation::require_writable(tx.sql(), scope, company_id, true).await?;
        let employee = crate::employments::lock(tx.sql(), scope.tenant(), company_id, employee_id)
            .await?
            .ok_or(WorkforceError::NotFound)?;
        let fresh = self.authentication.reprove(tx.sql(), scope).await?;
        Ok(fresh
            .actor()
            .affiliation()
            .is_some_and(|id| id.get() == employee.affiliation_id))
    }

    /// Runs a self-service write under the actor's own employment lock
    /// (Bilimbi `with_self_employee_lock/4`): opens a mutation transaction
    /// named `action` (the caller's audited action, `people.<module>.<verb>`),
    /// takes [`Workforce::lock_self_employee`], calls `write` with the
    /// transaction and the proven employee, and commits what it returns; an
    /// error rolls everything back. `write` records its rows through
    /// `tx.mutate`, as every People write does.
    pub async fn with_self_employee_lock<T, F>(
        &self,
        scope: &Scope,
        company_id: i64,
        capability: &str,
        action: &str,
        write: F,
    ) -> Result<T>
    where
        F: for<'b, 'c> FnOnce(
            &'b mut MutationTransaction<'c>,
            &'b SelfEmployee,
        ) -> SelfWrite<'b, T>,
    {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .mutations
            .begin(&mut client, scope.mutation_context(action))
            .await?;
        let me = self
            .lock_self_employee(&tx, scope, company_id, capability)
            .await?;
        let value = write(&mut tx, &me).await?;
        tx.commit().await?;
        Ok(value)
    }

    /// The account's affiliation facet, re-proved now on this client.
    async fn fresh_affiliation(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
    ) -> Result<i64> {
        let fresh = self
            .authentication
            .reprove(client, scope)
            .await
            .map_err(|_| WorkforceError::NotLinked)?;
        fresh
            .actor()
            .affiliation()
            .map(|affiliation| affiliation.get())
            .ok_or(WorkforceError::NotLinked)
    }

    pub(crate) async fn self_employee_on(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company_id: i64,
    ) -> Result<SelfEmployee> {
        let company = match summary(client, scope, company_id).await? {
            Some(company) if !company.archived => company,
            _ => return Err(WorkforceError::NotFound),
        };
        let affiliation = self.fresh_affiliation(client, scope).await?;
        let statuses = self.working_statuses_of(client, scope, company.id).await?;
        let row = crate::employments::by_affiliation(
            client,
            scope.tenant(),
            company.id,
            affiliation,
            &statuses,
        )
        .await?
        .ok_or(WorkforceError::NotLinked)?;
        Ok(SelfEmployee {
            employee_id: row.employment.id,
            person_id: row.employment.person_id,
            affiliation_id: row.employment.affiliation_id,
        })
    }
}

impl Workforce {
    /// Object-safe subject proof for an Artifacts owner callback. The caller
    /// authorizes its own capability and passes the live scope Artifacts just
    /// re-proved. Uses this transaction for the account's employment and target
    /// relation, locking both against relinking, status and supervisor changes.
    /// Working-status configuration is resolved afresh through its owner.
    pub async fn prove_self_or_direct_report(
        &self,
        client: &dyn ProofClient,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
        direct_report: bool,
    ) -> Result<()> {
        let affiliation = scope
            .actor()
            .affiliation()
            .ok_or(WorkforceError::NotLinked)?
            .get();
        // ProofClient only offers single-row reads; Settings needs ReadClient.
        // Resolve configuration on a short separate checkout, refusing pool
        // saturation rather than waiting while holding an Artifacts checkout.
        let mut timeouts = self.db.timeouts();
        timeouts.wait = Some(std::time::Duration::ZERO);
        let settings_client = self.db.timeout_get(&timeouts).await.map_err(Error::from)?;
        let statuses = self
            .working_statuses_of(&**settings_client, scope, company_id)
            .await?;
        drop(settings_client);
        let row = client.query_opt(
            "SELECT me.id FROM people_workforce_employments me
             JOIN people_workforce_employments subject ON subject.tenant_id=me.tenant_id AND subject.company_id=me.company_id
             WHERE me.tenant_id=$1 AND me.company_id=$2 AND me.affiliation_id=$3
             AND me.status=ANY($4) AND subject.status=ANY($4) AND subject.id=$5
             AND (($6 AND subject.supervisor_employment_id=me.id AND subject.id<>me.id) OR (NOT $6 AND subject.id=me.id))
             FOR NO KEY UPDATE OF me,subject",
            &[&scope.tenant(),&company_id,&affiliation,&statuses,&employee_id,&direct_report],
        ).await?;
        row.ok_or(WorkforceError::NotLinked)?;
        Ok(())
    }
}
