//! PO2 two-session browser proof: after the disposable People walkthrough,
//! revoke or restore the manager's manage grant through Authz while their
//! Organisation page stays open. Never use against a real installation.
use bilibi_authz::grants::Grants;
use bilibi_database::{pool, Error, Result};
use bilibi_module_registry::{verify_composition, Services};
use bilibi_people_organisation::MANAGE;
use bilibi_tenancy::Authentication;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    let enabled = match std::env::args().nth(1).as_deref() {
        Some("revoke") => false,
        Some("restore") => true,
        _ => return Err(Error::Invalid),
    };
    let url = std::env::var("DATABASE_URL").map_err(|_| Error::Invalid)?;
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = pool(&url)?;
    // Refuse any database that does not have this walkthrough's cast.
    let client = db.get().await?;
    let fixture: bool = client.query_one(
        "SELECT EXISTS(SELECT 1 FROM users WHERE tenant_id=1 AND id=1 AND email='manager@example.test') AND EXISTS(SELECT 1 FROM users WHERE tenant_id=1 AND id=4 AND email='admin@example.test')",
        &[],
    ).await?.get(0);
    if !fixture {
        return Err(Error::Invalid);
    }
    let modules = bilibi_module_registry::discover()?;
    verify_composition(
        &modules,
        bilibi_domains::MODULES,
        bilibi_extensions::MODULES,
    )?;
    let mut services = Services::default();
    services.provide("bootstrap.database", Arc::new(db.clone()))?;
    services.provide("bootstrap.composition", Arc::new(bilibi_web::composition()))?;
    services.install(&modules)?;
    let admin = services
        .get::<Authentication>("base/tenancy.authentication")?
        .authenticate(&**client, 1, 4, "organisation-explorer-proof".into())
        .await?;
    services
        .get::<Grants>("base/authz.grants")?
        .set_direct(&admin, 1, MANAGE, Some(enabled))
        .await?;
    println!(
        "Manager's Organisation manage grant: {}",
        if enabled { "restored" } else { "revoked" }
    );
    Ok(())
}
