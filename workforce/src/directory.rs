//! Names and work contacts for the employees a read returns, through Core's
//! owners rather than People's tables: a Person's name through the Principal
//! Directory (what every Base screen names a Person with: capability-free,
//! bounded to the ids asked for, tenant-scoped, a retired Person unresolved),
//! and the affiliation's work email through the Affiliation owner's read,
//! which honours its field restrictions and needs `admin.affiliation.list`.
use crate::{Result, WorkforceError};
use bilibi_affiliation::{AffiliationService, Search};
use bilibi_database::Error;
use bilibi_party_ids::OrganisationId;
use bilibi_principal_directory::{PrincipalDirectory, PrincipalKind, PrincipalRef, Query};
use bilibi_tenancy::Scope;
use std::collections::HashMap;
use tokio_postgres::GenericClient;

/// What Core says about the people behind a set of employments.
#[derive(Default)]
pub(crate) struct Identities {
    /// Person id → name; a Person nobody can name keeps the durable label.
    pub names: HashMap<i64, String>,
    /// Affiliation id → work email, for an actor who may list affiliations.
    pub emails: HashMap<i64, String>,
}

impl Identities {
    pub fn name(&self, person_id: i64) -> String {
        self.names.get(&person_id).cloned().unwrap_or_else(|| {
            PrincipalRef::new(PrincipalKind::Person, person_id)
                .map(|principal| principal.durable_label())
                .unwrap_or_else(|_| format!("Person {person_id}"))
        })
    }
    pub fn email(&self, affiliation_id: i64) -> Option<String> {
        self.emails.get(&affiliation_id).cloned()
    }
}

/// Resolves the names of `person_ids` and, when the actor may list the
/// company's affiliations, their work emails.
pub(crate) async fn resolve(
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    directory: &PrincipalDirectory,
    affiliations: &AffiliationService,
    company_id: i64,
    person_ids: &[i64],
    affiliation_ids: &[i64],
) -> Result<Identities> {
    let mut identities = Identities::default();
    if person_ids.is_empty() {
        return Ok(identities);
    }
    let candidates = person_ids
        .iter()
        .map(|id| PrincipalRef::new(PrincipalKind::Person, *id).map_err(|_| Error::Internal))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let ranked = directory
        .rank(
            client,
            scope,
            &candidates,
            &Query::default().ceiling(bilibi_principal_directory::MAX_CEILING),
        )
        .await
        .map_err(|_| WorkforceError::Failed(Error::Internal))?;
    for named in ranked.named {
        identities.names.insert(named.principal.id(), named.name);
    }
    // A work contact is the Affiliation owner's to show: read it only with the
    // owner's own capability, and leave it out otherwise.
    let may_list = bilibi_authz::decide(client, scope, bilibi_affiliation::contributions::LIST)
        .await?
        .allowed;
    if may_list && !affiliation_ids.is_empty() {
        let organisation_id = OrganisationId::new(company_id)?;
        let mut page = 1;
        loop {
            let found = affiliations
                .search(
                    scope,
                    &Search {
                        organisation_id: Some(organisation_id),
                        include_unlinked: true,
                        page,
                        page_size: 100,
                        ..Search::default()
                    },
                )
                .await?;
            let count = found.entries.len() as i64;
            for affiliation in found.entries {
                let id = affiliation.id.get();
                if let (true, Some(email)) = (
                    affiliation_ids.contains(&id),
                    affiliation.affiliation.work_email,
                ) {
                    identities.emails.insert(id, email);
                }
            }
            if count < 100 || page * 100 >= found.total {
                break;
            }
            page += 1;
        }
    }
    Ok(identities)
}
