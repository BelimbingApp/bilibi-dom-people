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
        "prepare" => prepare(&services, &admin).await?,
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

async fn prepare(services: &Services, admin: &bilibi_tenancy::Scope) -> Result<()> {
    let db = services.get::<bilibi_database::Db>("base/database.pool")?;
    let client = db.get().await?;
    let manager = services
        .get::<Authentication>("base/tenancy.authentication")?
        .authenticate(&**client, 1, 1, "training-records-proof".into())
        .await?;
    drop(client);
    let grants = services.get::<Grants>("base/authz.grants")?;
    for user in [1, 3] {
        for key in [RECORDS_WORKSPACE_VIEW, PASSPORT_MY_VIEW, PASSPORT_GENERATE] {
            grants.set_direct(admin, user, key, Some(true)).await?;
        }
    }
    let workforce =
        services.get::<bilibi_people_workforce::Workforce>(bilibi_people_workforce::SERVICE)?;
    let own = workforce
        .self_employee(&manager, 1)
        .await
        .map_err(|_| Error::Internal)?
        .employee_id;
    let staff = workforce
        .self_employee(
            &services
                .get::<Authentication>("base/tenancy.authentication")?
                .authenticate(&**db.get().await?, 1, 3, "training-records-proof".into())
                .await?,
            1,
        )
        .await
        .map_err(|_| Error::Internal)?
        .employee_id;
    let training = services.get::<Training>(SERVICE)?;
    let course = prepared(
        training
            .create_course(
                &manager,
                1,
                &NewCourse {
                    code: "pt7-browser".into(),
                    name: "Records pagination proof".into(),
                    description: None,
                },
            )
            .await,
    )?;
    let event = prepared(
        training
            .create_event(
                &manager,
                1,
                &NewEvent {
                    course_id: course.id,
                    name: "Records browser cohort".into(),
                    capacity: 20,
                },
            )
            .await,
    )?;
    let mut selected = 0;
    for day in 1..=27 {
        let session = prepared(
            training
                .create_session(
                    &manager,
                    1,
                    &NewSession {
                        event_id: event.id,
                        name: format!("Records browser session {day:02}"),
                        capacity: 20,
                        time_zone: "UTC".into(),
                        starts_local: format!("2026-07-{day:02}T08:00"),
                        ends_local: format!("2026-07-{day:02}T09:00"),
                    },
                )
                .await,
        )?;
        let fact = prepared(
            training
                .record(
                    &manager,
                    1,
                    &NewAttendance {
                        session_id: session.id,
                        employee_id: staff,
                        status: AttendanceStatus::Confirmed,
                        reason: "Browser attendance proof".into(),
                        import_key: format!("pt7-session-{day}"),
                    },
                )
                .await,
        )?;
        if day == 27 {
            selected = session.id;
            prepared(
                training
                    .attach(
                        &manager,
                        1,
                        fact.id,
                        b"%PDF-1.7\nPT7 original revision evidence\n%%EOF\n",
                    )
                    .await,
            )?;
            for revision in 2..=27 {
                prepared(
                    training
                        .record(
                            &manager,
                            1,
                            &NewAttendance {
                                session_id: session.id,
                                employee_id: staff,
                                status: if revision % 2 == 0 {
                                    AttendanceStatus::Absent
                                } else {
                                    AttendanceStatus::Confirmed
                                },
                                reason: format!("Browser correction {revision}"),
                                import_key: format!("pt7-revision-{revision}"),
                            },
                        )
                        .await,
                )?;
            }
            prepared(
                training
                    .record(
                        &manager,
                        1,
                        &NewAttendance {
                            session_id: session.id,
                            employee_id: own,
                            status: AttendanceStatus::Confirmed,
                            reason: "Own passport browser proof".into(),
                            import_key: "pt7-manager".into(),
                        },
                    )
                    .await,
            )?;
        }
    }
    println!("PT7 records: /people/training/records?company_id=1&session_id={selected}");
    println!("PT7 team passport: /people/training/records/team?company_id=1&employee_id={staff}");
    println!("PT7 current manager employee: {own}; staff employee: {staff}");
    Ok(())
}
fn prepared<T>(result: bilibi_people_training::Result<T>) -> Result<T> {
    result.map_err(|e| {
        eprintln!("PT7 fixture refused: {e}");
        Error::Internal
    })
}
