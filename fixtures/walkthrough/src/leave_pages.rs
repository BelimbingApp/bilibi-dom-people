//! PL3: a small standalone fixture for the two Leave pages, through facades.
use bilibi_authz::grants::Grants;
use bilibi_database::{pool, Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_leave::{
    Leave, NewEntry, NewLeaveType, NewRequest, POLICIES_MANAGE, REQUESTS_APPROVE, SELF_VIEW,
};
use bilibi_people_workforce::{NewEmployment, Workforce};
use bilibi_tenancy::Authentication;
use jiff::ToSpan;
use rust_decimal::Decimal;
use std::sync::Arc;

pub(super) async fn run(url: &str) -> Result<()> {
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = pool(url)?;
    let modules = bilibi_module_registry::discover()?;
    bilibi_database::migrate(&db, &modules).await?;
    bilibi_walkthrough_fixture::install(&db).await?;
    let mut services = Services::default();
    services.provide("bootstrap.database", Arc::new(db.clone()))?;
    services.provide("bootstrap.composition", Arc::new(bilibi_web::composition()))?;
    services.install(&modules)?;
    services
        .get::<bilibi_base::step_up::StepUp>(bilibi_base::step_up::SERVICE)?
        .set_enforced(false);
    let grants = services.get::<Grants>("base/authz.grants")?;
    grants.reconcile().await?;
    let auth = services.get::<Authentication>("base/tenancy.authentication")?;
    let client = db.get().await?;
    let admin = auth
        .authenticate(&**client, 1, 4, "walkthrough".into())
        .await?;
    for key in [SELF_VIEW, REQUESTS_APPROVE, POLICIES_MANAGE] {
        grants.set_direct(&admin, 1, key, Some(true)).await?;
    }
    grants.set_direct(&admin, 3, SELF_VIEW, Some(true)).await?;
    grants.set_direct(&admin, 7, SELF_VIEW, Some(true)).await?;
    let manager = auth
        .authenticate(&**client, 1, 1, "walkthrough".into())
        .await?;
    let workforce = services.get::<Workforce>(bilibi_people_workforce::SERVICE)?;
    let employee = workforce
        .create_employment(
            &admin,
            1,
            &NewEmployment::new(1, 1, "PL3-001", "employment", "active"),
        )
        .await
        .map_err(|error| {
            eprintln!("PL3 employment: {error}");
            Error::Internal
        })?;
    let leave = services.get::<Leave>(bilibi_people_leave::SERVICE)?;
    // A second linked account has no types yet, matching the parity capture.
    workforce
        .create_employment(
            &admin,
            2,
            &NewEmployment::new(2, 2, "PL3-002", "employment", "active"),
        )
        .await
        .map_err(|error| {
            eprintln!("PL3 empty-company employment: {error}");
            Error::Internal
        })?;
    let today = leave.today(&manager, 1).await.map_err(failed)?;
    let mut future = today.checked_add(8.days()).map_err(|_| Error::Internal)?;
    while future.weekday().to_monday_one_offset() > 5 {
        future = future.tomorrow().map_err(|_| Error::Internal)?;
    }
    for (code, name, unit) in [
        ("pl3-day", "PL3 day leave", "day"),
        ("pl3-hour", "PL3 hour leave", "hour"),
    ] {
        let kind = leave
            .create_type(&manager, 1, &NewLeaveType::new(code, name, unit))
            .await
            .map_err(failed)?;
        leave
            .record_entry(
                &manager,
                1,
                employee.id,
                &NewEntry {
                    leave_type_id: kind.id,
                    entry_type: "opening".into(),
                    quantity: Decimal::new(2000, 2),
                    occurred_on: today,
                    source: "walkthrough-pl3".into(),
                    entry_key: code.into(),
                    note: Some("PL3 browser opening balance".into()),
                },
            )
            .await
            .map_err(failed)?;
        if unit == "day" {
            let mut request = NewRequest::new(kind.id, future, "pl3-browser-pending");
            request.reason = Some("PL3 approval interaction".into());
            leave
                .submit_request(&manager, 1, &request)
                .await
                .map_err(failed)?;
        }
    }
    println!("PL3 browser fixture ready: manager@example.test has own balances and a pending request; admin@example.test can decide independently; staff@example.test has self-service access without an employee link; viewer@example.test cannot open either page.");
    Ok(())
}
fn failed(error: bilibi_people_leave::LeaveError) -> Error {
    eprintln!("PL3 fixture: {error}");
    Error::Internal
}

walkthrough_command!(
    "leave-pages",
    |url, _argument| async move { run(url).await }
);
