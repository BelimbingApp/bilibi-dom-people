//! PT7 disposable browser controls; run the default walkthrough first.
use bilibi_authz::grants::Grants;
use bilibi_database::{Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_training::*;
use bilibi_tenancy::Authentication;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let url = std::env::var("DATABASE_URL").map_err(|_| Error::Invalid)?;
    let command = std::env::args().nth(1).ok_or(Error::Invalid)?;
    let db = bilibi_database::pool(&url)?;
    let mut services = Services::default();
    services.provide("bootstrap.database", Arc::new(db.clone()))?;
    services.provide("bootstrap.composition", Arc::new(bilibi_web::composition()))?;
    services.install(&bilibi_module_registry::discover()?)?;
    if std::env::var("BILIBI_ENV").as_deref() == Ok("development") {
        services
            .get::<bilibi_base::step_up::StepUp>(bilibi_base::step_up::SERVICE)?
            .set_enforced(false);
    }
    let client = db.get().await?;
    // Match the Platform walkthrough identities; this binary installs no production rows.
    let admin = services
        .get::<Authentication>("base/tenancy.authentication")?
        .authenticate(&**client, 1, 4, "training-records-proof".into())
        .await?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    match command.as_str() {
        "hub-only" => {
            for (key, enabled) in [(RECORDS_WORKSPACE_VIEW, true), (RECORDS_VIEW, false)] {
                grants.set_direct(&admin, 3, key, Some(enabled)).await?;
            }
        }
        _ => {
            let (enabled, key) = command.split_once('-').ok_or(Error::Invalid)?;
            let enabled = match enabled {
                "deny" => false,
                "allow" => true,
                _ => return Err(Error::Invalid),
            };
            let key = match key {
                "record" => RECORDS_MANAGE,
                "evidence" => EVIDENCE_MANAGE,
                "retention" => RETENTION_MANAGE,
                "read" => RECORDS_VIEW,
                "workspace" => RECORDS_WORKSPACE_VIEW,
                _ => return Err(Error::Invalid),
            };
            grants.set_direct(&admin, 1, key, Some(enabled)).await?;
        }
    }
    println!("PT7 browser condition applied: {command}");
    Ok(())
}
