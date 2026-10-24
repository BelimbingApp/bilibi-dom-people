//! PU1 browser accounts on an already installed disposable People walkthrough.
//! Mirrors Bilimbi's manager and viewer permission states, through Base Authz.
use bilibi_authz::grants::Grants;
use bilibi_database::{Error, Result, pool};
use bilibi_module_registry::Services;
use bilibi_tenancy::Authentication;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = pool(&std::env::var("DATABASE_URL").map_err(|_| Error::Invalid)?)?;
    let client = db.get().await?;
    // Refuse an installation without the exact disposable fixture identities.
    for (id, email) in [
        (1_i64, "manager@example.test"),
        (2, "viewer@example.test"),
        (4, "admin@example.test"),
    ] {
        if client
            .query_opt(
                "SELECT id FROM users WHERE tenant_id=1 AND id=$1 AND email=$2",
                &[&id, &email],
            )
            .await?
            .is_none()
        {
            return Err(Error::Invalid);
        }
    }
    let modules = bilibi_module_registry::discover()?;
    let mut services = Services::default();
    services.provide("bootstrap.database", Arc::new(db.clone()))?;
    services.provide("bootstrap.composition", Arc::new(bilibi_web::composition()))?;
    services.install(&modules)?;
    if std::env::var("BILIBI_ENV").as_deref() == Ok("development") {
        services
            .get::<bilibi_base::step_up::StepUp>(bilibi_base::step_up::SERVICE)?
            .set_enforced(false);
    }
    let admin = services
        .get::<Authentication>("base/tenancy.authentication")?
        .authenticate(&**client, 1, 4, "PU1 disposable browser accounts".into())
        .await?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    for key in grants.vocabulary().capabilities.keys() {
        if key.starts_with("people.") {
            grants.set_direct(&admin, 1, key, Some(true)).await?;
            let read = key.ends_with(".view") && !key.ends_with(".self.view");
            grants.set_direct(&admin, 2, key, Some(read)).await?;
        }
    }
    grants
        .set_direct(&admin, 2, bilibi_company::TENANT_WIDE, Some(true))
        .await?;
    println!(
        "manager@example.test: every People permission; viewer@example.test: non-self-service reads only"
    );
    println!(
        "Browser records and employment links are kept; use the Platform walkthrough password."
    );
    Ok(())
}
