//! PL4 browser companions on an already-installed disposable walkthrough DB.
//! `deny`/`allow` change the manager's live grant while its page stays open.
//! `skips` creates >200 real skipped balances through the Leave facade.
use bilibi_authz::grants::Grants;
use bilibi_database::{pool, Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_leave::{Leave, NewEntry, NewLeaveType, NewPolicy, POLICIES_MANAGE};
use bilibi_people_workforce::Workforce;
use bilibi_tenancy::Authentication;
use rust_decimal::Decimal;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = pool(&std::env::var("DATABASE_URL").map_err(|_| Error::Invalid)?)?;
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
    let client = db.get().await?;
    if client
        .query_opt(
            "SELECT 1 FROM users WHERE tenant_id=1 AND id=1 AND email='manager@example.test'",
            &[],
        )
        .await?
        .is_none()
    {
        eprintln!("PL4 needs an installed disposable walkthrough database.");
        return Err(Error::Invalid);
    }
    let authentication = services.get::<Authentication>("base/tenancy.authentication")?;
    let admin = authentication
        .authenticate(&**client, 1, 4, "PL4 walkthrough".into())
        .await?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    match std::env::args().nth(1).as_deref() {
        Some(action @ ("deny" | "allow")) => {
            grants
                .set_direct(&admin, 1, POLICIES_MANAGE, Some(action == "allow"))
                .await?;
            println!("PL4: manager policy access {action}; use its already-open browser page.");
        }
        Some("skips") => {
            let manager = authentication
                .authenticate(&**client, 1, 1, "PL4 walkthrough".into())
                .await?;
            let leave = services.get::<Leave>(bilibi_people_leave::SERVICE)?;
            let workforce = services.get::<Workforce>(bilibi_people_workforce::SERVICE)?;
            let employee = workforce
                .employees(&**client, &manager, 1)
                .await
                .map_err(|_| Error::Invalid)?
                .require_current()
                .map_err(|_| Error::Invalid)?
                .into_iter()
                .next()
                .ok_or(Error::NotFound)?;
            let rules = leave.rules(&manager, 1).await.map_err(|_| Error::Invalid)?;
            let today = leave.today(&manager, 1).await.map_err(|_| Error::Invalid)?;
            let year = rules.leave_year(today) - 1;
            let (from, _) = rules.year_range(year - 1).map_err(|_| Error::Invalid)?;
            for index in 0..201 {
                let kind = leave
                    .create_type(
                        &manager,
                        1,
                        &NewLeaveType::new(
                            format!("pl4-bounded-{index}"),
                            format!("Skipped balance {index:03}"),
                            "day",
                        ),
                    )
                    .await
                    .map_err(|_| Error::Invalid)?;
                leave
                    .add_policy(
                        &manager,
                        1,
                        kind.id,
                        &NewPolicy {
                            effective_from: from,
                            entitlement: Decimal::ONE,
                            carry_forward_cap: Some(Decimal::ONE),
                        },
                    )
                    .await
                    .map_err(|_| Error::Invalid)?;
                leave
                    .record_entry(
                        &manager,
                        1,
                        employee
                            .reference
                            .stable_id
                            .parse()
                            .map_err(|_| Error::Invalid)?,
                        &NewEntry {
                            leave_type_id: kind.id,
                            entry_type: "opening".into(),
                            quantity: Decimal::ONE,
                            occurred_on: from,
                            source: "walkthrough-pl4".into(),
                            entry_key: format!("bounded-{index}"),
                            note: None,
                        },
                    )
                    .await
                    .map_err(|_| Error::Invalid)?;
            }
            let counts = leave
                .carry_forward(&manager, 1, year)
                .await
                .map_err(|_| Error::Invalid)?;
            let report = leave
                .carry_forward_skipped(&manager, 1)
                .await
                .map_err(|_| Error::Invalid)?;
            if report.skips.len() != 200 || report.total <= 200 {
                return Err(Error::Invalid);
            }
            println!("PL4: {} shown of {} skipped balances; {} earlier-year refusals. Refresh the policy page.",report.skips.len(),report.total,counts.previous_year_open);
        }
        _ => return Err(Error::Invalid),
    }
    Ok(())
}
