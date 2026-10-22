//! PT2: real facade, database guards and private download proof data.
use crate::{COMPANY, MANAGER, OTHER_TENANT_COMPANY};
use bilibi_authz::grants::Grants;
use bilibi_database::{Error, MutationBoundary, Result};
use bilibi_module_registry::Services;
use bilibi_people_training::{
    AttendanceStatus as Status, NewAttendance, NewCourse, NewEvent, NewSession, Training,
    TrainingError, EVIDENCE_MANAGE, RECORDS_MANAGE, RECORDS_VIEW, RETENTION_MANAGE,
};
use bilibi_settings::{Settings, Target};
use bilibi_tenancy::Scope;
use serde_json::json;
use std::{os::unix::fs::PermissionsExt, path::PathBuf};

fn required<T>(result: bilibi_people_training::Result<T>) -> Result<T> {
    result.map_err(|e| {
        eprintln!("PT2 unexpected refusal: {e}");
        Error::Internal
    })
}
fn refused<T>(label: &str, result: bilibi_people_training::Result<T>) -> Result<()> {
    match result {
        Err(e) => {
            println!("{label}: refused ({e})");
            Ok(())
        }
        Ok(_) => {
            eprintln!("{label}: unexpectedly accepted");
            Err(Error::Internal)
        }
    }
}
fn input(session_id: i64, employee_id: i64, key: &str, status: Status) -> NewAttendance {
    NewAttendance {
        session_id,
        employee_id,
        status,
        reason: "Attendance register checked".into(),
        import_key: key.into(),
    }
}
pub(crate) async fn run(
    services: &Services,
    client: &tokio_postgres::Client,
    admin: &Scope,
    manager: &Scope,
    viewer: &Scope,
    employees: (i64, i64),
) -> Result<()> {
    println!("\n== Training participation and evidence (PT2) ==");
    let training = services.get::<Training>(bilibi_people_training::SERVICE)?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    for key in [
        RECORDS_VIEW,
        RECORDS_MANAGE,
        EVIDENCE_MANAGE,
        RETENTION_MANAGE,
    ] {
        grants.set_direct(admin, MANAGER, key, Some(true)).await?;
    }
    let course = required(
        training
            .create_course(
                manager,
                COMPANY,
                &NewCourse {
                    code: "pt2-register".into(),
                    name: "Participation proof".into(),
                    description: None,
                },
            )
            .await,
    )?;
    let event = required(
        training
            .create_event(
                manager,
                COMPANY,
                &NewEvent {
                    course_id: course.id,
                    name: "Participation proof event".into(),
                    capacity: 2,
                },
            )
            .await,
    )?;
    let session_input = NewSession {
        event_id: event.id,
        name: "Attendance proof".into(),
        capacity: 1,
        time_zone: "UTC".into(),
        starts_local: "2026-10-01T08:00".into(),
        ends_local: "2026-10-01T09:00".into(),
    };
    let session = required(
        training
            .create_session(manager, COMPANY, &session_input)
            .await,
    )?;
    let first = input(session.id, employees.0, "pt2-first", Status::Confirmed);
    let fact = required(training.record(manager, COMPANY, &first).await)?;
    println!("confirmed fact={} revision={}", fact.id, fact.revision);
    let mut replay = first.clone();
    replay.reason = format!("  {}  ", first.reason);
    let replay = required(training.record(manager, COMPANY, &replay).await)?;
    if replay != fact {
        return Err(Error::Internal);
    }
    println!(
        "exact replay with trimmed reason returns original fact {}",
        replay.id
    );
    let mut conflict = first.clone();
    conflict.status = Status::Absent;
    refused(
        "key conflict",
        training.record(manager, COMPANY, &conflict).await,
    )?;
    let mut correction = first.clone();
    correction.import_key = "pt2-confirmed-correction".into();
    correction.reason = "Register signed".into();
    let revised = required(training.record(manager, COMPANY, &correction).await)?;
    if revised.revision != 2 {
        return Err(Error::Internal);
    }
    println!(
        "confirmed correction at full capacity: revision {}",
        revised.revision
    );
    let second = input(session.id, employees.1, "pt2-second", Status::Confirmed);
    match training.record(manager, COMPANY, &second).await {
        Err(TrainingError::AttendanceCapacityExceeded) => {
            println!("new confirmed employee at capacity: refused")
        }
        _ => return Err(Error::Internal),
    }
    let mut absence = first.clone();
    absence.import_key = "pt2-absence".into();
    absence.status = Status::Absent;
    let absent = required(training.record(manager, COMPANY, &absence).await)?;
    let admitted = required(training.record(manager, COMPANY, &second).await)?;
    println!(
        "revision {} releases place; employee {} admitted as fact {}",
        absent.revision, admitted.employee_id, admitted.id
    );
    if required(training.record(manager, COMPANY, &first).await)?.id != fact.id {
        return Err(Error::Internal);
    }
    refused(
        "missing session",
        training
            .record(
                manager,
                COMPANY,
                &input(i64::MAX, employees.0, "pt2-no-session", Status::Absent),
            )
            .await,
    )?;
    refused(
        "missing employee",
        training
            .record(
                manager,
                COMPANY,
                &input(session.id, i64::MAX, "pt2-no-employee", Status::Absent),
            )
            .await,
    )?;
    refused(
        "viewer record",
        training.record(viewer, COMPANY, &first).await,
    )?;
    refused(
        "other tenant",
        training
            .history(manager, OTHER_TENANT_COMPANY, session.id)
            .await,
    )?;
    let mut invalid = first.clone();
    invalid.import_key = "pt2-invalid".into();
    invalid.reason = " ".into();
    refused(
        "blank reason",
        training.record(manager, COMPANY, &invalid).await,
    )?;
    let history = required(training.history(manager, COMPANY, session.id).await)?;
    println!("history: {history:?}");
    println!(
        "sessions={} employees={:?}",
        required(training.sessions(manager, COMPANY).await)?.len(),
        required(training.participation_employees(manager, COMPANY).await)?
    );
    grants
        .set_direct(admin, MANAGER, RECORDS_MANAGE, Some(false))
        .await?;
    refused(
        "record after grant revoked, same scope",
        training.record(manager, COMPANY, &first).await,
    )?;
    grants
        .set_direct(admin, MANAGER, RECORDS_MANAGE, Some(true))
        .await?;

    // Two calls on separate pool connections contend for one remaining place.
    let race = required(
        training
            .create_session(
                manager,
                COMPANY,
                &NewSession {
                    name: "Capacity race".into(),
                    ..session_input
                },
            )
            .await,
    )?;
    let a = input(race.id, employees.0, "pt2-race-a", Status::Confirmed);
    let b = input(race.id, employees.1, "pt2-race-b", Status::Confirmed);
    let (a, b) = tokio::join!(
        training.record(manager, COMPANY, &a),
        training.record(manager, COMPANY, &b)
    );
    println!("simultaneous last-place contenders: {a:?}, {b:?}");
    if !matches!(
        (&a, &b),
        (Ok(_), Err(TrainingError::AttendanceCapacityExceeded))
            | (Err(TrainingError::AttendanceCapacityExceeded), Ok(_))
    ) {
        return Err(Error::Internal);
    }

    // Raw SQL only demonstrates database guards in the disposable fixture.
    for (label, sql) in [
        ("fact update", format!("UPDATE people_training_attendance_facts SET reason='changed' WHERE id={}", fact.id)),
        ("fact delete", format!("DELETE FROM people_training_attendance_facts WHERE id={}", fact.id)),
        ("skipped revision", format!("INSERT INTO people_training_attendance_facts (tenant_id,company_id,session_id,employee_id,revision,status,reason,import_key,actor_user_id) VALUES ({},{},{},{},9,'absent','guard','pt2-skip',{})", manager.tenant(), COMPANY, session.id, employees.0, manager.user())),
        ("capacity trigger", format!("INSERT INTO people_training_attendance_facts (tenant_id,company_id,session_id,employee_id,revision,status,reason,import_key,actor_user_id) VALUES ({},{},{},{},4,'confirmed','guard','pt2-full',{})", manager.tenant(), COMPANY, session.id, employees.0, manager.user())),
        ("facts truncate", "TRUNCATE people_training_attendance_facts CASCADE".into()),
    ] { match client.batch_execute(&sql).await { Err(error) => println!("SQL {label}: refused ({})", error.as_db_error().map_or("database", |e| e.message())), Ok(()) => return Err(Error::Internal) } }

    // Base Settings and private filesystem faults belong only to this proof.
    let root = std::env::var_os("BILIBI_TRAINING_PROOF_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("bilibi-training-proof-{}", std::process::id()))
        });
    std::fs::create_dir_all(&root).map_err(|_| Error::Internal)?;
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
        .map_err(|_| Error::Internal)?;
    let settings = services.get::<Settings>("base/settings.store")?;
    let mutations = services.get::<MutationBoundary>("base/database.mutations")?;
    let db = services.get::<bilibi_database::Db>("base/database.pool")?;
    for (key, value) in [
        ("artifacts.storage_root", json!(root)),
        ("artifacts.retention_days", json!(30)),
        ("artifacts.purge_max_attempts", json!(1)),
    ] {
        let mut connection = db.get().await?;
        let mut tx = mutations
            .begin(
                &mut connection,
                admin.mutation_context("people.training.proof.configure"),
            )
            .await?;
        settings
            .put(&mut tx, admin, Target::Global, key, value)
            .await?;
        tx.commit().await?;
    }
    // A disposable fault trigger revokes Create as the reservation commits.
    // Publication must re-prove authority on its own transaction and refuse.
    client
        .batch_execute(&format!(
            r#"
        CREATE FUNCTION walkthrough_pt2_revoke_create() RETURNS trigger LANGUAGE plpgsql AS $$
        BEGIN
          IF NEW.owner_id = 'people/training' THEN
            UPDATE principal_capabilities SET is_allowed=false
              WHERE tenant_id={} AND user_id={} AND capability='people.training.evidence.manage';
          END IF;
          RETURN NEW;
        END $$;
        CREATE TRIGGER walkthrough_pt2_revoke_create AFTER INSERT ON base_artifacts
          FOR EACH ROW EXECUTE FUNCTION walkthrough_pt2_revoke_create();
    "#,
            manager.tenant(),
            manager.user()
        ))
        .await?;
    let publication = training
        .attach(manager, COMPANY, fact.id, b"%PDF-1.7\nreservation proof")
        .await;
    client.batch_execute("DROP TRIGGER walkthrough_pt2_revoke_create ON base_artifacts; DROP FUNCTION walkthrough_pt2_revoke_create()").await?;
    refused(
        "Create revoked between reservation and publication",
        publication,
    )?;
    grants
        .set_direct(admin, MANAGER, EVIDENCE_MANAGE, Some(true))
        .await?;
    let reserved: i64 = client.query_one("SELECT count(*) FROM base_artifacts WHERE owner_id='people/training' AND ready_at IS NULL AND purged_at IS NULL", &[]).await?.get(0);
    if reserved != 1 || !required(training.evidence(manager, COMPANY, fact.id).await)?.is_empty() {
        return Err(Error::Internal);
    }
    client.execute("UPDATE base_artifacts SET expires_at=created_at+interval '1 microsecond' WHERE owner_id='people/training' AND ready_at IS NULL", &[]).await?;
    let recovered = required(training.purge(manager, COMPANY).await)?;
    if recovered.purged.len() != 1 || !recovered.failed.is_empty() {
        return Err(Error::Internal);
    }
    println!("unpublished reservation stayed tracked and purged; no evidence row was linked");

    refused(
        "non-PDF attach",
        training
            .attach(manager, COMPANY, fact.id, b"not a PDF")
            .await,
    )?;
    refused(
        "missing fact attach",
        training
            .attach(manager, COMPANY, i64::MAX, b"%PDF-1.7\n")
            .await,
    )?;
    refused(
        "viewer attach",
        training
            .attach(viewer, COMPANY, fact.id, b"%PDF-1.7\n")
            .await,
    )?;
    // The upload contract is a header gate, just like Bilimbi (not PdfOwner).
    let bytes = b"%PDF-1.7\nParticipation evidence fixture\n%%EOF\n";
    let evidence = required(training.attach(manager, COMPANY, fact.id, bytes).await)?;
    let document = required(
        training
            .read_evidence(manager, COMPANY, &evidence.artifact_id)
            .await,
    )?;
    if document.bytes != bytes {
        return Err(Error::Internal);
    }
    println!(
        "private evidence round trip: {} bytes; download /people/training/evidence/{}/{}",
        document.bytes.len(),
        COMPANY,
        evidence.artifact_id
    );
    refused(
        "viewer evidence read",
        training
            .read_evidence(viewer, COMPANY, &evidence.artifact_id)
            .await,
    )?;
    refused(
        "wrong company evidence read",
        training
            .read_evidence(manager, OTHER_TENANT_COMPANY, &evidence.artifact_id)
            .await,
    )?;
    grants
        .set_direct(admin, MANAGER, EVIDENCE_MANAGE, Some(false))
        .await?;
    refused(
        "attach after grant revoked, same scope",
        training.attach(manager, COMPANY, fact.id, bytes).await,
    )?;
    grants
        .set_direct(admin, MANAGER, EVIDENCE_MANAGE, Some(true))
        .await?;
    for sql in [
        format!(
            "UPDATE people_training_evidence SET fact_id={} WHERE id={}",
            admitted.id, evidence.id
        ),
        format!(
            "DELETE FROM people_training_evidence WHERE id={}",
            evidence.id
        ),
        "TRUNCATE people_training_evidence".into(),
    ] {
        if client.batch_execute(&sql).await.is_ok() {
            return Err(Error::Internal);
        }
    }
    println!("SQL evidence update/delete/truncate: refused");
    // Keep this first evidence available for the authenticated HTTP proof.
    let expired = required(training.attach(manager, COMPANY, fact.id, bytes).await)?;
    client.execute("UPDATE base_artifacts SET expires_at=created_at+interval '1 microsecond' WHERE public_id=$1", &[&expired.artifact_id]).await?;
    refused(
        "read expired evidence",
        training
            .read_evidence(manager, COMPANY, &expired.artifact_id)
            .await,
    )?;
    refused("viewer purge", training.purge(viewer, COMPANY).await)?;
    // An unsafe root makes Base hold the failed purge after its first attempt.
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o500))
        .map_err(|_| Error::Internal)?;
    let purged = required(training.purge(manager, COMPANY).await)?;
    println!("unavailable storage purge: {purged:?}");
    let holds = required(training.purge_holds(manager, COMPANY).await)?;
    println!("purge holds: {holds:?}");
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
        .map_err(|_| Error::Internal)?;
    if !holds
        .iter()
        .any(|hold| hold.metadata.id == expired.artifact_id)
    {
        return Err(Error::Internal);
    }
    required(
        training
            .retry_purge(manager, COMPANY, &expired.artifact_id)
            .await,
    )?;
    if !required(training.purge_holds(manager, COMPANY).await)?.is_empty() {
        return Err(Error::Internal);
    }
    let provenance = required(training.evidence(manager, COMPANY, fact.id).await)?;
    println!(
        "retry completed; permanent provenance rows={} (purged document still recorded)",
        provenance.len()
    );
    Ok(())
}
