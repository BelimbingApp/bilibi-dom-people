//! PT8: leave the disposable fixture ready for Learning browser interactions.
//! No production data or bypass of the owning facades.
use crate::{COMPANY, MANAGER, STAFF, VIEWER};
use bilibi_authz::grants::Grants;
use bilibi_database::{Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_training::{Training, LEARNING_VIEW, SERVICE};
use bilibi_people_workforce::Workforce;
use bilibi_tenancy::Scope;
use bilibi_user::UserService;

pub async fn run(
    services: &Services,
    admin: &Scope,
    manager: &Scope,
    staff_employee: i64,
) -> Result<()> {
    let grants = services.get::<Grants>("base/authz.grants")?;
    for user in [MANAGER, STAFF, VIEWER] {
        grants
            .set_direct(admin, user, LEARNING_VIEW, Some(true))
            .await?;
    }
    let workforce = services.get::<Workforce>(bilibi_people_workforce::SERVICE)?;
    let employment = workforce
        .employment(manager, COMPANY, staff_employee)
        .await
        .map_err(|_| Error::Internal)?
        .require_current()
        .map_err(|_| Error::Internal)?;
    let users = services.get::<UserService>("core/user.accounts")?;
    users
        .link_person(
            admin,
            STAFF,
            bilibi_party_ids::PersonId::new(employment.person_id).map_err(|_| Error::Invalid)?,
        )
        .await
        .map_err(|_| Error::Internal)?;
    users
        .link_affiliation(
            admin,
            STAFF,
            bilibi_party_ids::AffiliationId::new(employment.affiliation_id)
                .map_err(|_| Error::Invalid)?,
        )
        .await
        .map_err(|_| Error::Internal)?;
    let training = services.get::<Training>(SERVICE)?;
    if !training.allowed(manager, COMPANY, LEARNING_VIEW).await {
        return Err(Error::Internal);
    }
    // The viewer can open the task hub while requests.submit still refuses.
    println!("PT8 Learning pages ready: manager and staff have task access; viewer has only the Learning entry grant. Staff is linked for browser request submission; the independent admin can review and approve.");
    Ok(())
}
