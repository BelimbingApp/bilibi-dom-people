//! PT7 standalone disposable browser fixture and current-permission controls.
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
    let modules = bilibi_module_registry::discover()?;
    if command == "prepare" {
        bilibi_database::migrate(&db, &modules).await?;
        bilibi_walkthrough_fixture::install(&db).await?;
    }
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
    // Match the Platform walkthrough identities; this binary installs no production rows.
    let admin = services
        .get::<Authentication>("base/tenancy.authentication")?
        .authenticate(&**client, 1, 4, "training-records-proof".into())
        .await?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    match command.as_str() {
        "prepare" => prepare(&services, &admin).await?,
        "unlink-staff" => {
            services
                .get::<bilibi_user::UserService>("core/user.accounts")?
                .unlink(&admin, 3)
                .await
                .map_err(|_| Error::Internal)?;
        }
        "relink-staff" | "detach-staff" | "restore-staff" => {
            let workforce = services
                .get::<bilibi_people_workforce::Workforce>(bilibi_people_workforce::SERVICE)?;
            let people = workforce
                .employees_at_any_status(&admin, 1, None)
                .await
                .map_err(|_| Error::Internal)?
                .require_current()
                .map_err(|_| Error::Internal)?;
            let employee = people
                .iter()
                .find(|e| e.display_name == "Sam Staff")
                .ok_or(Error::NotFound)?;
            let employee_id = employee
                .reference
                .stable_id
                .parse::<i64>()
                .map_err(|_| Error::Invalid)?;
            let employment = workforce
                .employment(&admin, 1, employee_id)
                .await
                .map_err(|_| Error::Internal)?
                .require_current()
                .map_err(|_| Error::Internal)?;
            if command == "relink-staff" {
                let users = services.get::<bilibi_user::UserService>("core/user.accounts")?;
                users
                    .link_person(
                        &admin,
                        3,
                        bilibi_party_ids::PersonId::new(employment.person_id)
                            .map_err(|_| Error::Invalid)?,
                    )
                    .await
                    .map_err(|_| Error::Internal)?;
                bilibi_people_walkthrough::give_staff_desk(
                    &users,
                    &admin,
                    3,
                    bilibi_party_ids::PersonId::new(employment.person_id)
                        .map_err(|_| Error::Invalid)?,
                    bilibi_party_ids::OrganisationId::new(employment.platform_company_id)
                        .map_err(|_| Error::Invalid)?,
                )
                .await
                .map_err(|_| Error::Internal)?;
            } else {
                let manager = services
                    .get::<Authentication>("base/tenancy.authentication")?
                    .authenticate(&**client, 1, 1, "training-records-proof".into())
                    .await?;
                let manager_id = workforce
                    .self_employee(&manager, 1)
                    .await
                    .map_err(|_| Error::Internal)?
                    .employee_id;
                workforce
                    .update_employment(
                        &admin,
                        1,
                        employee_id,
                        &bilibi_people_workforce::EmploymentChange {
                            supervisor_employment_id: Some(if command == "detach-staff" {
                                None
                            } else {
                                Some(manager_id)
                            }),
                            ..bilibi_people_workforce::EmploymentChange::at(employment.version)
                        },
                    )
                    .await
                    .map_err(|_| Error::Internal)?;
            }
        }
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
    for key in CAPABILITIES {
        grants.set_direct(admin, 1, key, Some(true)).await?;
    }
    for key in [RECORDS_WORKSPACE_VIEW, PASSPORT_MY_VIEW, PASSPORT_GENERATE] {
        grants.set_direct(admin, 3, key, Some(true)).await?;
    }
    grants
        .set_direct(admin, 3, RECORDS_VIEW, Some(false))
        .await?;
    grants
        .set_direct(
            admin,
            1,
            bilibi_people_workforce::EMPLOYMENT_VIEW,
            Some(true),
        )
        .await?;
    let references = services.get::<bilibi_people_reference_data::ReferenceData>(
        bilibi_people_reference_data::SERVICE,
    )?;
    references
        .create_entry(
            admin,
            1,
            &bilibi_people_reference_data::NewEntry::new(
                "employment_type",
                "pt7-learning",
                "Learning proof employment",
            ),
        )
        .await
        .map_err(|_| Error::Internal)?;
    let workforce =
        services.get::<bilibi_people_workforce::Workforce>(bilibi_people_workforce::SERVICE)?;
    let own = workforce
        .create_employment(
            admin,
            1,
            &bilibi_people_workforce::NewEmployment::new(
                1,
                1,
                "PT7-MANAGER",
                "pt7-learning",
                "active",
            ),
        )
        .await
        .map_err(|e| {
            eprintln!("PT7 manager employment: {e}");
            Error::Internal
        })?
        .id;
    let person = services
        .get::<bilibi_person::PersonService>(bilibi_person::SERVICE)?
        .create(
            admin,
            &bilibi_person::PersonInput {
                name: "Sam Staff".into(),
                kind: bilibi_person::PersonKind::Human,
                email: None,
                phone: None,
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    let affiliation = services
        .get::<bilibi_affiliation::AffiliationService>(bilibi_affiliation::SERVICE)?
        .create(
            admin,
            &bilibi_affiliation::AffiliationInput {
                person_id: person.id,
                organisation_id: bilibi_party_ids::OrganisationId::new(1)
                    .map_err(|_| Error::Invalid)?,
                title: None,
                work_email: None,
                work_phone: None,
                dates: bilibi_affiliation::EffectiveDates::new(None, None)
                    .map_err(|_| Error::Invalid)?,
            },
        )
        .await?;
    let staff = workforce
        .create_employment(
            admin,
            1,
            &bilibi_people_workforce::NewEmployment {
                supervisor_employment_id: Some(own),
                ..bilibi_people_workforce::NewEmployment::new(
                    person.id.get(),
                    affiliation.id.get(),
                    "PT7-STAFF",
                    "pt7-learning",
                    "active",
                )
            },
        )
        .await
        .map_err(|_| Error::Internal)?
        .id;
    let users = services.get::<bilibi_user::UserService>("core/user.accounts")?;
    users
        .link_person(admin, 3, person.id)
        .await
        .map_err(|_| Error::Internal)?;
    bilibi_people_walkthrough::give_staff_desk(
        &users,
        admin,
        3,
        affiliation.affiliation.person_id,
        affiliation.affiliation.organisation_id,
    )
    .await
    .map_err(|_| Error::Internal)?;
    let root = std::env::var_os("BILIBI_TRAINING_PROOF_ROOT")
        .map(std::path::PathBuf::from)
        .ok_or(Error::Invalid)?;
    std::fs::create_dir_all(&root).map_err(|_| Error::Internal)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
        .map_err(|_| Error::Internal)?;
    let settings = services.get::<bilibi_settings::Settings>("base/settings.store")?;
    let mutations = services.get::<bilibi_database::MutationBoundary>("base/database.mutations")?;
    for (key, value) in [
        ("artifacts.storage_root", serde_json::json!(root)),
        ("artifacts.retention_days", serde_json::json!(30)),
        ("artifacts.purge_max_attempts", serde_json::json!(1)),
    ] {
        let mut connection = db.get().await?;
        let mut tx = mutations
            .begin(
                &mut connection,
                admin.mutation_context("people.training.browser.configure"),
            )
            .await?;
        settings
            .put(&mut tx, admin, bilibi_settings::Target::Global, key, value)
            .await?;
        tx.commit().await?;
    }
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
