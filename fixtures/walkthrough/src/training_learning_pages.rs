//! PT8: leave the disposable fixture ready for Learning browser interactions.
//! No production data or bypass of the owning facades.
use crate::{COMPANY, MANAGER, STAFF, VIEWER};
use bilibi_authz::grants::Grants;
use bilibi_database::{Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_training::{
    Training, BUDGETS_VIEW, LEARNING_VIEW, PLANS_VIEW, REQUESTS_VIEW, SERVICE,
};
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
    for cap in [
        "admin.company.tenant-wide.manage",
        REQUESTS_VIEW,
        PLANS_VIEW,
        BUDGETS_VIEW,
    ] {
        grants.set_direct(admin, VIEWER, cap, Some(true)).await?;
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
    println!("PT8 Learning pages ready: manager and staff have task access; viewer has Learning entry and register read grants. Staff is linked for browser request submission; the independent admin can review and approve.");
    Ok(())
}

/// Changes one proof condition through its owner while a browser page stays mounted.
/// Only for the disposable walkthrough database, after `run` prepared it.
pub async fn control(url: &str, action: &str) -> Result<()> {
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = bilibi_database::pool(url)?;
    let modules = bilibi_module_registry::discover()?;
    let mut services = Services::default();
    services.provide("bootstrap.database", std::sync::Arc::new(db.clone()))?;
    services.provide(
        "bootstrap.composition",
        std::sync::Arc::new(bilibi_web::composition()),
    )?;
    services.install(&modules)?;
    if std::env::var("BILIBI_ENV").as_deref() == Ok("development") {
        services
            .get::<bilibi_base::step_up::StepUp>(bilibi_base::step_up::SERVICE)?
            .set_enforced(false);
    }
    let client = db.get().await?;
    let auth = services.get::<bilibi_tenancy::Authentication>("base/tenancy.authentication")?;
    let admin = auth
        .authenticate(&**client, crate::TENANT, crate::ADMIN, "walkthrough".into())
        .await?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    match action {
        "deny-submit" | "allow-submit" => {
            grants
                .set_direct(
                    &admin,
                    STAFF,
                    bilibi_people_training::REQUESTS_SUBMIT,
                    Some(action == "allow-submit"),
                )
                .await?;
        }
        "deny-budget-manage" | "allow-budget-manage" => {
            grants
                .set_direct(
                    &admin,
                    MANAGER,
                    bilibi_people_training::BUDGETS_MANAGE,
                    Some(action == "allow-budget-manage"),
                )
                .await?;
        }
        "unlink-staff" => {
            services
                .get::<UserService>("core/user.accounts")?
                .unlink(&admin, STAFF)
                .await
                .map_err(|_| Error::Internal)?;
        }
        "relink-staff" => {
            let workforce = services.get::<Workforce>(bilibi_people_workforce::SERVICE)?;
            let people = workforce
                .employees_at_any_status(&admin, COMPANY, None)
                .await
                .map_err(|_| Error::Internal)?
                .require_current()
                .map_err(|_| Error::Internal)?;
            let employee = people
                .iter()
                .find(|e| e.display_name == "Sam Staff")
                .ok_or(Error::NotFound)?;
            let id = employee
                .reference
                .stable_id
                .parse::<i64>()
                .map_err(|_| Error::Invalid)?;
            let employment = workforce
                .employment(&admin, COMPANY, id)
                .await
                .map_err(|_| Error::Internal)?
                .require_current()
                .map_err(|_| Error::Internal)?;
            let users = services.get::<UserService>("core/user.accounts")?;
            users
                .link_person(
                    &admin,
                    STAFF,
                    bilibi_party_ids::PersonId::new(employment.person_id)
                        .map_err(|_| Error::Invalid)?,
                )
                .await
                .map_err(|_| Error::Internal)?;
            users
                .link_affiliation(
                    &admin,
                    STAFF,
                    bilibi_party_ids::AffiliationId::new(employment.affiliation_id)
                        .map_err(|_| Error::Invalid)?,
                )
                .await
                .map_err(|_| Error::Internal)?;
        }
        "archive-company-three" => {
            services
                .get::<bilibi_company::CompanyService>(bilibi_company::SERVICE)?
                .archive_company(&admin, 3, Some("PT8 mounted-page archive refusal proof"))
                .await
                .map_err(|_| Error::Internal)?;
        }
        _ => return Err(Error::Invalid),
    }
    println!("PT8 proof condition applied through its owner: {action}");
    Ok(())
}
