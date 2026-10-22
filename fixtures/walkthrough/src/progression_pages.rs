//! PG2 disposable page proof. Business records and grants use their owners.
use bilibi_authz::grants::Grants;
use bilibi_database::{pool, Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_performance as pm;
use bilibi_people_progression as pg;
use bilibi_people_skills as sk;
use bilibi_tenancy::Authentication;
use std::sync::Arc;

pub async fn run(url: &str, command: &str) -> Result<()> {
    let db = pool(url)?;
    let modules = bilibi_module_registry::discover()?;
    let mut services = Services::default();
    services.provide("bootstrap.database", Arc::new(db.clone()))?;
    services.provide("bootstrap.composition", Arc::new(bilibi_web::composition()))?;
    services.install(&modules)?;
    let client = db.get().await?;
    let auth = services.get::<Authentication>("base/tenancy.authentication")?;
    let admin = auth
        .authenticate(&**client, 1, 4, "walkthrough".into())
        .await?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    match command {
        "prepare" => {
            for cap in [
                pg::POLICY_VIEW,
                pg::POLICY_MANAGE,
                pg::SELF_VIEW,
                pm::SELF_VIEW,
                sk::SELF_VIEW,
            ] {
                grants.set_direct(&admin, 1, cap, Some(true)).await?;
            }
            for cap in [pg::SELF_VIEW, pm::SELF_VIEW, sk::SELF_VIEW] {
                grants.set_direct(&admin, 3, cap, Some(true)).await?;
            }
            grants
                .set_direct(&admin, 2, pg::POLICY_VIEW, Some(true))
                .await?;
            grants
                .set_direct(&admin, 2, pg::POLICY_MANAGE, Some(false))
                .await?;
            println!("PG2 prepared: manager can draft and publish, viewer has read-only history, staff can read both standing sections; no progression policies yet.");
        }
        "revoke-manage" | "restore-manage" => {
            grants
                .set_direct(
                    &admin,
                    1,
                    pg::POLICY_MANAGE,
                    Some(command == "restore-manage"),
                )
                .await?;
            println!("PG2 manager policy management: {command}");
        }
        "both" | "performance-only" | "progression-only" | "neither" => {
            grants
                .set_direct(
                    &admin,
                    3,
                    pg::SELF_VIEW,
                    Some(matches!(command, "both" | "progression-only")),
                )
                .await?;
            grants
                .set_direct(
                    &admin,
                    3,
                    pm::SELF_VIEW,
                    Some(matches!(command, "both" | "performance-only")),
                )
                .await?;
            println!("PG2 staff standing permissions: {command}");
        }
        "deny-evidence" | "restore-evidence" => {
            grants
                .set_direct(
                    &admin,
                    3,
                    sk::SELF_VIEW,
                    Some(command == "restore-evidence"),
                )
                .await?;
            println!("PG2 staff competency evidence permission: {command}");
        }
        "history" => {
            let manager = auth
                .authenticate(&**client, 1, 1, "walkthrough".into())
                .await?;
            let progression = services.get::<pg::Progression>(pg::SERVICE)?;
            for version in 1..=26 {
                progression
                    .draft(
                        &manager,
                        1,
                        &pg::DraftInput {
                            code: "pg2_history".into(),
                            version,
                            name: format!("PG2 history version {version}"),
                            effective_from: "2020-01-01".into(),
                            profile: None,
                            performance: Some(pg::PerformanceRule {
                                periods: vec![pg::Period {
                                    start: "2020-01-01".into(),
                                    end: "2020-01-31".into(),
                                }],
                                accepted_outcomes: vec!["Corrected outcome".into()],
                                missing_evidence: pg::MissingEvidence::Unknown,
                            }),
                        },
                    )
                    .await
                    .map_err(|_| Error::Internal)?;
            }
            println!("PG2 recorded 26 history drafts through Progression for pagination proof.");
        }
        _ => return Err(Error::Invalid),
    }
    Ok(())
}
