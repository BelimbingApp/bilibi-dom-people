//! The interim self-service seam: which employment the signed-in account's
//! affiliation is, in one company.
//!
//! Bilimbi resolves the actor's own employee through
//! `Workforce.Authorization.self_employee/2`. Bilibi's Workforce gains that
//! helper (`authorization::authorize_self`, `with_self_employee_lock`) with
//! the employment proof in port plan row PW1, which this slice does not wait
//! for. Until it lands, this module answers the one question the facade
//! needs, by the rule PW1 fixes: the actor's account must be linked to an
//! affiliation with this very company (`Scope::actor().affiliation()` and
//! `organisation()`), and that affiliation must be an employment of the
//! company. The facade then reads that employment through
//! `Workforce::employee`, which applies the company's working statuses and
//! Core's naming, so nothing else here bypasses the Workforce facade.
//!
//! TODO(PW1): replace `resolve` with `bilibi_people_workforce::authorization::authorize_self`
//! and delete this module.
use crate::Result;
use bilibi_database::Error;
use bilibi_tenancy::Scope;
use tokio_postgres::GenericClient;

/// The employment id of the actor's own affiliation with the company, or
/// `None` when the account is unlinked, linked to another company, or its
/// affiliation has no employment.
pub(crate) async fn resolve(
    client: &impl GenericClient,
    scope: &Scope,
    company_id: i64,
) -> Result<Option<i64>> {
    let actor = scope.actor();
    let (Some(organisation), Some(affiliation)) = (actor.organisation(), actor.affiliation())
    else {
        return Ok(None);
    };
    if organisation.get() != company_id {
        return Ok(None);
    }
    let row = client
        .query_opt(
            "SELECT id FROM people_workforce_employments WHERE tenant_id = $1 AND company_id = $2 AND affiliation_id = $3",
            &[&scope.tenant(), &company_id, &affiliation.get()],
        )
        .await
        .map_err(Error::from)?;
    Ok(row.map(|row| row.get(0)))
}
