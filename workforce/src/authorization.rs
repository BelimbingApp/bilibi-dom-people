//! Per-operation authorization for People (Bilimbi
//! `Bilimbi.People.Workforce.Authorization`): the one boundary every People
//! facade authorizes through, at the public API so pages, jobs and other
//! adapters share it.
//!
//! A page rechecks its route capability before acting, and a `can_*` only
//! decides which controls it renders: an administrator can revoke a grant
//! while the page stays connected. Every People write, and every read of
//! private employee facts, therefore calls this module first with the scope
//! it was handed, so a revocation takes effect on the very next operation.
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
//! Self-service (`self_employee`, `authorize_self`, `with_self_employee_lock`)
//! lands with the employment proof in PW1.
use crate::{CompanySummary, Result, WorkforceError};
use bilibi_company::organisation::{self, Directory};
use bilibi_database::Error;
use bilibi_party_ids::OrganisationId;
use bilibi_tenancy::{OrganisationDirectory, ProofClient, Scope};
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
