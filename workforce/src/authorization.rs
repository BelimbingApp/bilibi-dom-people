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
//! axis by Core Company's `bilibi_company::authorize_work_target`: an explicit desk
//! plus the existing operation capability.
//!
//! Self-service operations act on the employee the login account is linked
//! to *now*. [`Workforce::self_employee`] re-proves the account through
//! Base Tenancy on every call and reads `Scope::actor().person()` from
//! that fresh scope (Bilimbi read `User.get_user/3`'s employee link), and
//! [`Workforce::lock_self_employee`] proves it again inside the transaction
//! that holds the employment row's lock, so a self-service write cannot
//! commit for an employee the account no longer belongs to.
use crate::{CompanySummary, Result, Workforce, WorkforceError};
use bilibi_company::Directory;
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
    bilibi_company::authorize_work_target(client, scope, company_id, capability)
        .await
        .map_err(refusal)
}

/// Company authorization on the caller's transaction for object-safe owner
/// callbacks (Artifacts). Uses Core Company's operational desk proof; never opens
/// a second connection while the owner holds its reservation/publication lock.
pub async fn authorize_company_proof(
    client: &dyn ProofClient,
    scope: &Scope,
    company_id: i64,
    capability: &str,
) -> Result<bilibi_tenancy::NamedOrganisation> {
    bilibi_company::authorize_work_target(client, scope, company_id, capability)
        .await
        .map_err(refusal)?;
    Directory
        .lookup(client, scope, OrganisationId::new(company_id)?)
        .await
        .map_err(refusal)?
        .ok_or(WorkforceError::NotFound)
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
/// name: none without the capability, otherwise the actor's assigned desks.
pub async fn selectable_companies(
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    capability: &str,
) -> Result<Vec<CompanySummary>> {
    bilibi_authz::authorize_key(client, scope, capability)
        .await
        .map_err(refusal)?;
    let named = Directory
        .reachable(client as &dyn ProofClient, scope)
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

/// Active companies for a People chooser, retaining its already-selected
/// pending or suspended company so the page does not jump. An archived company
/// is never offered. Selection is presentation, never authority: Core rechecks
/// the capability and company reach before retaining that company. Record
/// discovery uses [`selectable_companies`] instead, so pending and suspended
/// companies do not hide existing records.
pub async fn company_choices(
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    capability: &str,
    selected: Option<i64>,
) -> Result<Vec<CompanySummary>> {
    let mut companies = selectable_companies(client, scope, capability).await?;
    companies.retain(|company| company.status == "active");
    if let Some(id) = selected.filter(|id| !companies.iter().any(|company| company.id == *id)) {
        match authorize_company(client, scope, id, capability).await {
            Ok(company) if !company.archived => {
                companies.push(company);
                companies.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
            }
            Ok(_) | Err(WorkforceError::Unauthorized | WorkforceError::NotFound) => {}
            Err(error) => return Err(error),
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
    let Some(identity) = bilibi_company::identity(client, scope, company_id).await? else {
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
        status: named.status,
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
        let person = match self.fresh_person(client, scope).await {
            Ok(id) => id,
            Err(WorkforceError::NotLinked) => return Ok(None),
            Err(error) => return Err(error),
        };
        let statuses: Vec<String> = crate::EMPLOYEE_STATUSES
            .iter()
            .map(|s| (*s).into())
            .collect();
        Ok(
            crate::employments::by_person(client, scope.tenant(), company.id, person, &statuses)
                .await?
                .map(|row| row.employment.id),
        )
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
        bilibi_company::require_writable(tx.sql(), scope, company_id, true).await?;
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
        bilibi_company::require_writable(tx.sql(), scope, company_id, true).await?;
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

    /// Authorizes the self-service capability for the company, then resolves
    /// the actor's own employee (Bilimbi `authorize_self/3`).
    pub async fn authorize_self(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company_id: i64,
        capability: &str,
    ) -> Result<SelfEmployee> {
        authorize_company_proof(client, scope, company_id, capability).await?;
        self.self_employee(client, scope, company_id).await
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
        let me = self.self_employee(tx.sql(), scope, company_id).await?;
        bilibi_company::require_writable(tx.sql(), scope, company_id, true).await?;
        let locked = crate::employments::lock(tx.sql(), scope.tenant(), company_id, me.employee_id)
            .await?
            .ok_or(WorkforceError::NotLinked)?;
        // The link and the working status, proved again under the lock.
        let fresh = self.fresh_person(tx.sql(), scope).await?;
        let statuses = self
            .working_statuses_of(tx.sql(), scope, company_id)
            .await?;
        if fresh != locked.person_id || !statuses.contains(&locked.status) {
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
        bilibi_company::require_writable(tx.sql(), scope, company_id, true).await?;
        let employee = crate::employments::lock(tx.sql(), scope.tenant(), company_id, employee_id)
            .await?
            .ok_or(WorkforceError::NotFound)?;
        let fresh = self.authentication.reprove(tx.sql(), scope).await?;
        Ok(fresh
            .actor()
            .person()
            .is_some_and(|id| id.get() == employee.person_id))
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

    /// The staff account's Person, re-proved now on this client.
    async fn fresh_person(
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
            .person()
            .map(|person| person.get())
            .ok_or(WorkforceError::NotLinked)
    }

    /// The working employee the signed-in account is linked to in this
    /// company, resolved now (Bilimbi `self_employee/2`): the account is
    /// re-proved through Base Tenancy on this call, its Person identity
    /// read from the fresh scope, and the employment holding that
    /// affiliation in this company found in the company's working statuses.
    /// `NotLinked` covers a frozen account, an account without an
    /// Person link, an affiliation of another company, and a linked
    /// employee who is not a current working employee; a company the scope
    /// cannot see is `NotFound`.
    pub async fn self_employee(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company_id: i64,
    ) -> Result<SelfEmployee> {
        let company = match summary(client, scope, company_id).await? {
            Some(company) if !company.archived => company,
            _ => return Err(WorkforceError::NotFound),
        };
        if !bilibi_company::reachable(client, scope, OrganisationId::new(company_id)?)
            .await
            .map_err(refusal)?
        {
            return Err(WorkforceError::NotLinked);
        }
        let person = self.fresh_person(client, scope).await?;
        let statuses = self.working_statuses_of(client, scope, company.id).await?;
        let row =
            crate::employments::by_person(client, scope.tenant(), company.id, person, &statuses)
                .await?
                .ok_or(WorkforceError::NotLinked)?;
        let affiliation = bilibi_party_ids::AffiliationId::new(row.employment.affiliation_id)?;
        if bilibi_affiliation::identity::account_standing(client, scope.tenant(), affiliation)
            .await?
            != Some((
                bilibi_party_ids::PersonId::new(person)?,
                OrganisationId::new(company_id)?,
            ))
        {
            return Err(WorkforceError::NotLinked);
        }
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
    /// re-proved. The caller holds the company's writable lock across its file
    /// operation, and its self employment lock for a self-service operation.
    /// This checks current membership on the callback transaction without taking
    /// those locks again on a second connection. Settings is resolved afresh.
    pub async fn prove_self_or_direct_report(
        &self,
        client: &dyn ProofClient,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
        direct_report: bool,
    ) -> Result<()> {
        let person = scope
            .actor()
            .person()
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
             WHERE me.tenant_id=$1 AND me.company_id=$2 AND me.person_id=$3
             AND me.status=ANY($4) AND subject.status=ANY($4) AND subject.id=$5
             AND (($6 AND subject.supervisor_employment_id=me.id AND subject.id<>me.id) OR (NOT $6 AND subject.id=me.id))",
            &[&scope.tenant(),&company_id,&person,&statuses,&employee_id,&direct_report],
        ).await?;
        row.ok_or(WorkforceError::NotLinked)?;
        Ok(())
    }
}
