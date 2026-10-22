//! PA3 browser fixture and permission changes on a disposable database.
//! Run `attendance-settings seed`, then open the four settings routes.
use bilibi_database::{Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_attendance::{ALLOWANCES_MANAGE, RULES_MANAGE};
use bilibi_tenancy::Authentication;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    let url = std::env::var("DATABASE_URL").map_err(|_| Error::Invalid)?;
    let mode = std::env::args().nth(1).unwrap_or_else(|| "seed".into());
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = bilibi_database::pool(&url)?;
    let modules = bilibi_module_registry::discover()?;
    if mode == "seed" {
        bilibi_database::migrate(&db, &modules).await?;
        bilibi_walkthrough_fixture::install(&db).await?;
    }
    let mut services = Services::default();
    services.provide("bootstrap.database", Arc::new(db.clone()))?;
    services.provide("bootstrap.composition", Arc::new(bilibi_web::composition()))?;
    services.install(&modules)?;
    services
        .get::<bilibi_base::step_up::StepUp>(bilibi_base::step_up::SERVICE)?
        .set_enforced(false);
    let client = db.get().await?;
    let auth = services.get::<Authentication>("base/tenancy.authentication")?;
    let admin = auth
        .authenticate(&**client, 1, 4, "pa3-fixture".into())
        .await?;
    let grants = services.get::<bilibi_authz::grants::Grants>("base/authz.grants")?;
    match mode.as_str() {
        "seed" | "restore" => {
            for capability in [RULES_MANAGE, ALLOWANCES_MANAGE] {
                grants.set_direct(&admin, 1, capability, Some(true)).await?;
            }
            // A second account proves the allowance tab is withheld independently.
            grants
                .set_direct(&admin, 2, RULES_MANAGE, Some(true))
                .await?;
            grants
                .set_direct(&admin, 2, ALLOWANCES_MANAGE, Some(false))
                .await?;
        }
        "revoke" => {
            for capability in [RULES_MANAGE, ALLOWANCES_MANAGE] {
                grants
                    .set_direct(&admin, 1, capability, Some(false))
                    .await?;
            }
        }
        _ => return Err(Error::Invalid),
    }
    println!("PA3 fixture {mode}: manager has both settings permissions; viewer has rules only (except while revoked).");
    Ok(())
}
