//! Running-database proof: a clock or rules write waiting on the company
//! lock must observe a rule change or capability revocation ahead of it.
use super::{COMPANY, MANAGER, RULES_MANAGE, STAFF};
use bilibi_company::organisation;
use bilibi_database::{Db, Error, MutationBoundary, Result};
use bilibi_module_registry::Services;
use bilibi_people_attendance::{
    Attendance, AttendanceError, ClockInput, EventType, NewShiftTemplate, RuleChanges,
    LOCATION_REQUIRED_KEY,
};
use bilibi_settings::{Settings, Target};
use bilibi_tenancy::Scope;
use std::time::Duration;

async fn queued(db: &Db, holder: i32) -> Result<()> {
    let client = db.get().await?;
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let waiting: bool = client
                .query_one(
                    "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid)))",
                    &[&holder],
                )
                .await?
                .get(0);
            if waiting {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .map_err(|_| Error::Internal)?
}

pub(super) async fn run(
    db: &Db,
    services: &Services,
    admin: &Scope,
    manager: &Scope,
    staff: &Scope,
    employee: i64,
) -> Result<()> {
    let attendance = services.get::<Attendance>(bilibi_people_attendance::SERVICE)?;
    let mutations = services.get::<MutationBoundary>("base/database.mutations")?;
    let settings = services.get::<Settings>("base/settings.store")?;
    let grants = services.get::<bilibi_authz::grants::Grants>("base/authz.grants")?;
    let mut client = db.get().await?;
    let mut tx = mutations
        .begin(
            &mut client,
            admin.mutation_context("walkthrough.attendance.location_rule"),
        )
        .await?;
    organisation::require_writable(tx.sql(), admin, COMPANY, true)
        .await
        .map_err(|_| Error::Internal)?;
    let holder = tx
        .sql()
        .query_one("SELECT pg_backend_pid()", &[])
        .await?
        .get(0);
    settings
        .put(
            &mut tx,
            admin,
            Target::Company(
                bilibi_affiliation::OrganisationId::new(COMPANY).map_err(|_| Error::Invalid)?,
            ),
            LOCATION_REQUIRED_KEY,
            true.into(),
        )
        .await?;
    let waiter = {
        let attendance = attendance.clone();
        let manager = manager.clone();
        tokio::spawn(async move {
            attendance
                .record_clock(
                    &manager,
                    COMPANY,
                    employee,
                    &ClockInput::new(
                        "queued-location",
                        EventType::In,
                        "device",
                        jiff::Timestamp::now(),
                    ),
                )
                .await
        })
    };
    queued(db, holder).await?;
    tx.commit().await?;
    match waiter.await.map_err(|_| Error::Internal)? {
        Err(AttendanceError::LocationRequired) => {
            println!("queued clock observes newly required location: refused (LocationRequired)")
        }
        other => {
            eprintln!("queued clock had unexpected result: {other:?}");
            return Err(Error::Internal);
        }
    }
    attendance
        .put_rules(
            manager,
            COMPANY,
            &RuleChanges {
                location_required: Some(false),
                ..RuleChanges::default()
            },
        )
        .await
        .map_err(|_| Error::Internal)?;

    let mut tx = mutations
        .begin(
            &mut client,
            admin.mutation_context("walkthrough.attendance.revoke"),
        )
        .await?;
    organisation::require_writable(tx.sql(), admin, COMPANY, true)
        .await
        .map_err(|_| Error::Internal)?;
    grants
        .set_direct_in(&mut tx, admin, MANAGER, RULES_MANAGE, Some(false))
        .await?;
    let waiter = {
        let attendance = attendance.clone();
        let manager = manager.clone();
        tokio::spawn(async move {
            attendance
                .create_shift_template(
                    &manager,
                    COMPANY,
                    &NewShiftTemplate {
                        code: "QUEUED".into(),
                        name: "Queued shift".into(),
                        starts_at: jiff::civil::time(8, 0, 0, 0),
                        ends_at: jiff::civil::time(16, 0, 0, 0),
                        break_minutes: 0,
                    },
                )
                .await
        })
    };
    queued(db, holder).await?;
    tx.commit().await?;
    match waiter.await.map_err(|_| Error::Internal)? {
        Err(AttendanceError::Unauthorized) => {
            println!("queued template observes revoked grant: refused (Unauthorized)")
        }
        other => {
            eprintln!("queued template had unexpected result: {other:?}");
            return Err(Error::Internal);
        }
    }
    grants
        .set_direct(admin, MANAGER, RULES_MANAGE, Some(true))
        .await?;
    let absent: bool = client.query_one(
        "SELECT NOT EXISTS (SELECT 1 FROM people_attendance_clock_facts WHERE tenant_id = $1 AND company_id = $2 AND event_key = 'queued-location') AND NOT EXISTS (SELECT 1 FROM people_attendance_shift_definitions WHERE tenant_id = $1 AND company_id = $2 AND code = 'QUEUED')",
        &[&admin.tenant(), &COMPANY],
    ).await?.get(0);
    if !absent {
        return Err(Error::Internal);
    }
    println!("both queued refusals left no clock or template rows");

    let users = services.get::<bilibi_user::UserService>("core/user.accounts")?;
    let person = staff.actor().person().ok_or(Error::Invalid)?;
    let affiliation = staff.actor().affiliation().ok_or(Error::Invalid)?;
    users
        .unlink(admin, STAFF)
        .await
        .map_err(|_| Error::Internal)?;
    match attendance.self_days(staff, COMPANY).await {
        Err(AttendanceError::NotLinked) => {
            println!("account unlinked after scope creation: own days refused (NotLinked)")
        }
        other => {
            eprintln!("unlinked account had unexpected result: {other:?}");
            return Err(Error::Internal);
        }
    }
    users
        .link_person(admin, STAFF, person)
        .await
        .map_err(|_| Error::Internal)?;
    users
        .link_affiliation(admin, STAFF, affiliation)
        .await
        .map_err(|_| Error::Internal)?;
    attendance
        .self_days(staff, COMPANY)
        .await
        .map_err(|_| Error::Internal)?;
    println!("restored account link: own days available again");
    Ok(())
}
