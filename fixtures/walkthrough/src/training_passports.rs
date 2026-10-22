//! PT5 executable proof on the disposable People walkthrough database.
use crate::{ADMIN, COMPANY, MANAGER, STAFF, TENANT, VIEWER};
use bilibi_affiliation::{AffiliationInput, AffiliationService, EffectiveDates};
use bilibi_authz::grants::Grants;
use bilibi_database::{Error, Result};
use bilibi_module_registry::Services;
use bilibi_party_ids::{AffiliationId, OrganisationId, PersonId};
use bilibi_people_training::*;
use bilibi_people_workforce::{Employment, EmploymentChange, NewEmployment, Workforce};
use bilibi_person::{PersonInput, PersonKind, PersonService};
use bilibi_tenancy::{Authentication, Scope};
use bilibi_user::UserService;
use std::{collections::HashMap, sync::Arc};

fn saved<T>(result: bilibi_people_training::Result<T>) -> Result<T> {
    result.map_err(|e| {
        eprintln!("PT5 unexpected: {e:?}");
        Error::Internal
    })
}
fn proof(label: &str, holds: bool) -> Result<()> {
    if !holds {
        eprintln!("PT5 FAILED: {label}");
        return Err(Error::Internal);
    }
    println!("PT5 proved: {label}");
    Ok(())
}
fn refused<T: std::fmt::Debug>(
    label: &str,
    result: bilibi_people_training::Result<T>,
    expected: TrainingError,
) -> Result<()> {
    match result {
        Err(e) if std::mem::discriminant(&e) == std::mem::discriminant(&expected) => {
            println!("PT5 refused {label}: {e}");
            Ok(())
        }
        other => {
            eprintln!("PT5 FAILED {label}: expected {expected:?}, got {other:?}");
            Err(Error::Internal)
        }
    }
}
fn params(first: &str, last: &str) -> HashMap<String, String> {
    HashMap::from([("from".into(), first.into()), ("until".into(), last.into())])
}
async fn learner(
    services: &Services,
    scope: &Scope,
    supervisor: i64,
    kind: &str,
    number: &str,
) -> Result<Employment> {
    let person = services
        .get::<PersonService>(bilibi_person::SERVICE)?
        .create(
            scope,
            &PersonInput {
                name: format!("Passport learner {number}"),
                kind: PersonKind::Human,
                email: None,
                phone: None,
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    let affiliation = services
        .get::<AffiliationService>(bilibi_affiliation::SERVICE)?
        .create(
            scope,
            &AffiliationInput {
                person_id: person.id,
                organisation_id: OrganisationId::new(COMPANY).map_err(|_| Error::Invalid)?,
                title: None,
                work_email: None,
                work_phone: None,
                dates: EffectiveDates::new(None, None).map_err(|_| Error::Invalid)?,
            },
        )
        .await?;
    services
        .get::<Workforce>(bilibi_people_workforce::SERVICE)?
        .create_employment(
            scope,
            COMPANY,
            &NewEmployment {
                supervisor_employment_id: Some(supervisor),
                ..NewEmployment::new(
                    person.id.get(),
                    affiliation.id.get(),
                    number,
                    kind,
                    "active",
                )
            },
        )
        .await
        .map_err(|_| Error::Internal)
}
async fn event(training: &Training, scope: &Scope, code: &str, name: &str) -> Result<Event> {
    let course = saved(
        training
            .create_course(
                scope,
                COMPANY,
                &NewCourse {
                    code: code.into(),
                    name: name.into(),
                    description: None,
                },
            )
            .await,
    )?;
    saved(
        training
            .create_event(
                scope,
                COMPANY,
                &NewEvent {
                    course_id: course.id,
                    name: name.into(),
                    capacity: 10,
                },
            )
            .await,
    )
}
async fn session(
    training: &Training,
    scope: &Scope,
    event: i64,
    name: &str,
    start: &str,
    end: &str,
) -> Result<Session> {
    saved(
        training
            .create_session(
                scope,
                COMPANY,
                &NewSession {
                    event_id: event,
                    name: name.into(),
                    capacity: 10,
                    time_zone: "UTC".into(),
                    starts_local: start.into(),
                    ends_local: end.into(),
                },
            )
            .await,
    )
}
async fn record(
    training: &Training,
    scope: &Scope,
    session: i64,
    employee: i64,
    key: &str,
    status: AttendanceStatus,
) -> Result<AttendanceFact> {
    saved(
        training
            .record(
                scope,
                COMPANY,
                &NewAttendance {
                    session_id: session,
                    employee_id: employee,
                    import_key: key.into(),
                    reason: "Verified register".into(),
                    status,
                },
            )
            .await,
    )
}

pub async fn run(url: &str, action: &str) -> Result<()> {
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = bilibi_database::pool(url)?;
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
    let auth = services.get::<Authentication>("base/tenancy.authentication")?;
    let admin = auth
        .authenticate(&**client, TENANT, ADMIN, "walkthrough".into())
        .await?;
    let manager = auth
        .authenticate(&**client, TENANT, MANAGER, "walkthrough".into())
        .await?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    let training = services.get::<Training>(SERVICE)?;
    let workforce = services.get::<Workforce>(bilibi_people_workforce::SERVICE)?;
    let users = services.get::<UserService>("core/user.accounts")?;
    let employees = workforce
        .employees_at_any_status(&manager, COMPANY, None)
        .await
        .map_err(|_| Error::Internal)?
        .require_current()
        .map_err(|_| Error::Internal)?;
    let staff_id = employees
        .iter()
        .find(|e| e.display_name == "Sam Staff")
        .and_then(|e| e.reference.stable_id.parse::<i64>().ok())
        .ok_or(Error::NotFound)?;
    let staff_employment = workforce
        .employment(&manager, COMPANY, staff_id)
        .await
        .map_err(|_| Error::Internal)?
        .require_current()
        .map_err(|_| Error::Internal)?;
    let relink = || async {
        users
            .link_person(
                &admin,
                STAFF,
                PersonId::new(staff_employment.person_id).map_err(|_| Error::Invalid)?,
            )
            .await
            .map_err(|_| Error::Internal)?;
        users
            .link_affiliation(
                &admin,
                STAFF,
                AffiliationId::new(staff_employment.affiliation_id).map_err(|_| Error::Invalid)?,
            )
            .await
            .map_err(|_| Error::Internal)
    };
    match action {
        "deny-my" | "allow-my" => {
            grants
                .set_direct(&admin, STAFF, PASSPORT_MY_VIEW, Some(action == "allow-my"))
                .await?;
            return Ok(());
        }
        "unlink-staff" => {
            users
                .unlink(&admin, STAFF)
                .await
                .map_err(|_| Error::Internal)?;
            return Ok(());
        }
        "relink-staff" => {
            relink().await?;
            return Ok(());
        }
        "run" => (),
        _ => return Err(Error::Invalid),
    }
    for cap in [
        PASSPORT_MY_VIEW,
        PASSPORT_TEAM_VIEW,
        PASSPORT_GENERATE,
        INSIGHTS_VIEW,
    ] {
        grants.set_direct(&admin, MANAGER, cap, Some(true)).await?;
    }
    for cap in [PASSPORT_MY_VIEW, PASSPORT_GENERATE] {
        grants.set_direct(&admin, STAFF, cap, Some(true)).await?;
    }
    grants
        .set_direct(&admin, STAFF, RECORDS_VIEW, Some(false))
        .await?;
    grants
        .set_direct(&admin, VIEWER, INSIGHTS_VIEW, Some(true))
        .await?;
    grants
        .set_direct(&admin, VIEWER, RECORDS_VIEW, Some(false))
        .await?;
    relink().await?;
    let staff = auth
        .authenticate(&**client, TENANT, STAFF, "walkthrough".into())
        .await?;
    let viewer = auth
        .authenticate(&**client, TENANT, VIEWER, "walkthrough".into())
        .await?;
    let own = workforce
        .self_employee(&manager, COMPANY)
        .await
        .map_err(|_| Error::Internal)?
        .employee_id;
    let directory = saved(
        training
            .passport_employees(&manager, COMPANY, PassportAudience::Team)
            .await,
    )?;
    proof(
        "team contains direct report and excludes self",
        directory
            .employees
            .iter()
            .any(|e| e.reference.stable_id == staff_id.to_string())
            && directory
                .employees
                .iter()
                .all(|e| e.reference.stable_id != own.to_string()),
    )?;
    refused(
        "self passport cannot target manager",
        training
            .passport(
                &staff,
                COMPANY,
                PassportAudience::SelfService,
                own,
                &HashMap::new(),
            )
            .await,
        TrainingError::OutsideTeam,
    )?;
    refused(
        "team passport cannot target self",
        training
            .passport(
                &manager,
                COMPANY,
                PassportAudience::Team,
                own,
                &HashMap::new(),
            )
            .await,
        TrainingError::OutsideTeam,
    )?;
    refused(
        "cross-tenant passport",
        training
            .passport(
                &manager,
                crate::OTHER_TENANT_COMPANY,
                PassportAudience::SelfService,
                own,
                &HashMap::new(),
            )
            .await,
        TrainingError::NotFound,
    )?;
    let extra_a = learner(
        &services,
        &manager,
        own,
        &staff_employment.employment_type,
        "PT5-003",
    )
    .await?;
    let extra_b = learner(
        &services,
        &manager,
        own,
        &staff_employment.employment_type,
        "PT5-004",
    )
    .await?;
    let limit_employee = learner(
        &services,
        &manager,
        own,
        &staff_employment.employment_type,
        "PT5-LIMIT",
    )
    .await?;
    let training_event = event(&training, &manager, "pt5-disclosed", "Passport attendance").await?;
    let session = session(
        &training,
        &manager,
        training_event.id,
        "Original register",
        "2026-07-03T08:00",
        "2026-07-03T09:00",
    )
    .await?;
    let original = record(
        &training,
        &manager,
        session.id,
        staff_id,
        "pt5-original",
        AttendanceStatus::Confirmed,
    )
    .await?;
    let evidence = saved(
        training
            .attach(
                &manager,
                COMPANY,
                original.id,
                b"%PDF-1.7\nPT5 original evidence\n%%EOF\n",
            )
            .await,
    )?;
    let first_document = saved(
        training
            .generate_passport(&staff, COMPANY, PassportAudience::SelfService, staff_id)
            .await,
    )?;
    let first_bytes = saved(
        training
            .read_passport(&staff, COMPANY, &first_document.id)
            .await,
    )?
    .bytes;
    let correction = record(
        &training,
        &manager,
        session.id,
        staff_id,
        "pt5-correction",
        AttendanceStatus::Absent,
    )
    .await?;
    let passport = saved(
        training
            .passport(
                &staff,
                COMPANY,
                PassportAudience::SelfService,
                staff_id,
                &HashMap::new(),
            )
            .await,
    )?;
    let row = passport
        .page
        .entries
        .iter()
        .find(|r| r.fact_id == correction.id)
        .ok_or(Error::Internal)?;
    proof(
        "latest revision replaces original and does not inherit evidence",
        row.status == "absent"
            && row.revision == 2
            && row.evidence.is_empty()
            && !passport
                .page
                .entries
                .iter()
                .any(|r| r.fact_id == original.id),
    )?;
    proof(
        "old PDF bytes stay identical after correction",
        saved(
            training
                .read_passport(&staff, COMPANY, &first_document.id)
                .await,
        )?
        .bytes
            == first_bytes,
    )?;
    proof(
        "self viewer can still download original evidence",
        saved(
            training
                .read_passport_evidence(&staff, COMPANY, &evidence.artifact_id)
                .await,
        )?
        .bytes
        .starts_with(b"%PDF-"),
    )?;
    let team_document = saved(
        training
            .generate_passport(&manager, COMPANY, PassportAudience::Team, staff_id)
            .await,
    )?;
    proof(
        "team viewer downloads subject PDF",
        saved(
            training
                .read_passport(&manager, COMPANY, &team_document.id)
                .await,
        )?
        .bytes
        .starts_with(b"%PDF-"),
    )?;
    for (employee, key, status) in [
        (own, "pt5-own", AttendanceStatus::Confirmed),
        (extra_a.id, "pt5-third", AttendanceStatus::Confirmed),
        (extra_b.id, "pt5-fourth", AttendanceStatus::Absent),
    ] {
        record(&training, &manager, session.id, employee, key, status).await?;
    }
    // One disclosed course, one below-cohort course, one complementary suppression.
    let small = event(&training, &manager, "pt5-small", "Small attendance cohort").await?;
    let small_session = session_for(&training, &manager, small.id, "Small cohort").await?;
    record(
        &training,
        &manager,
        small_session.id,
        staff_id,
        "pt5-small-one",
        AttendanceStatus::Confirmed,
    )
    .await?;
    let complement = event(
        &training,
        &manager,
        "pt5-complement",
        "Complementary attendance",
    )
    .await?;
    let complement_session =
        session_for(&training, &manager, complement.id, "Complementary cohort").await?;
    record(
        &training,
        &manager,
        complement_session.id,
        staff_id,
        "pt5-complement-one",
        AttendanceStatus::Confirmed,
    )
    .await?;
    record(
        &training,
        &manager,
        complement_session.id,
        own,
        "pt5-complement-two",
        AttendanceStatus::Confirmed,
    )
    .await?;
    let range = params("2026-07-03", "2026-07-03");
    let summary = saved(training.learning_insights(&manager, COMPANY, &range).await)?;
    let disclosed = summary
        .entries
        .iter()
        .find(|r| r.course_id == training_event.course_id)
        .ok_or(Error::Internal)?;
    proof(
        "distinct cohort and both large counts disclose",
        disclosed.employees == Some(4)
            && disclosed.confirmed == Some(2)
            && disclosed.absent == Some(2)
            && !disclosed.suppressed,
    )?;
    let suppressed = summary
        .entries
        .iter()
        .find(|r| r.course_id == small.course_id)
        .ok_or(Error::Internal)?;
    proof(
        "small cohort hides all three counts",
        suppressed.suppressed
            && suppressed.employees.is_none()
            && suppressed.confirmed.is_none()
            && suppressed.absent.is_none(),
    )?;
    let complementary = summary
        .entries
        .iter()
        .find(|r| r.course_id == complement.course_id)
        .ok_or(Error::Internal)?;
    proof(
        "small complementary count hides both attendance counts",
        !complementary.suppressed
            && complementary.employees == Some(2)
            && complementary.confirmed.is_none()
            && complementary.absent.is_none(),
    )?;
    let drill = saved(
        training
            .learning_insight_drill(&manager, COMPANY, training_event.course_id, &range)
            .await,
    )?;
    proof(
        "drill uses latest revision",
        drill.total_entries == 4
            && drill.entries.iter().any(|r| r.fact_id == correction.id)
            && !drill.entries.iter().any(|r| r.fact_id == original.id),
    )?;
    proof(
        "UTC end date includes whole day",
        saved(
            training
                .learning_insights(&manager, COMPANY, &params("2026-07-02", "2026-07-02"))
                .await,
        )?
        .entries
        .iter()
        .all(|r| r.course_id != training_event.course_id),
    )?;
    saved(
        training
            .learning_insights(&manager, COMPANY, &params("2024-01-01", "2024-12-31"))
            .await,
    )?;
    for range in [
        params("2024-01-01", "2025-01-01"),
        params("2026-07-04", "2026-07-03"),
        params("no-date", "2026-07-03"),
    ] {
        refused(
            "invalid or over-366-day insight window",
            training.learning_insights(&manager, COMPANY, &range).await,
            TrainingError::InvalidInsightRange,
        )?;
    }
    refused(
        "aggregate-only arbitrary window",
        training.learning_insights(&viewer, COMPANY, &range).await,
        TrainingError::ReportPeriodUnavailable,
    )?;
    refused(
        "aggregate-only drill",
        training
            .learning_insight_drill(&viewer, COMPANY, training_event.course_id, &range)
            .await,
        TrainingError::Unauthorized,
    )?;
    let periods = saved(training.learning_insight_periods(&viewer, COMPANY).await)?;
    proof("closed period choices exist", !periods.is_empty())?;
    let mut closed = HashMap::from([("period".into(), periods[0].period_start.clone())]);
    let before = saved(training.learning_insights(&viewer, COMPANY, &closed).await)?;
    closed.extend(params("invalid", "invalid"));
    proof(
        "aggregate period ignores arbitrary window input",
        serde_json::to_value(saved(
            training.learning_insights(&viewer, COMPANY, &closed).await,
        )?)
        .map_err(|_| Error::Internal)?
            == serde_json::to_value(before).map_err(|_| Error::Internal)?,
    )?;
    closed.insert("period".into(), "9999-01-01".into());
    refused(
        "unfrozen period",
        training.learning_insights(&viewer, COMPANY, &closed).await,
        TrainingError::ReportPeriodUnavailable,
    )?;
    grants
        .set_direct(&admin, STAFF, PASSPORT_MY_VIEW, Some(false))
        .await?;
    refused(
        "read after grant revoked with old scope",
        training
            .read_passport(&staff, COMPANY, &first_document.id)
            .await,
        TrainingError::Unauthorized,
    )?;
    grants
        .set_direct(&admin, STAFF, PASSPORT_MY_VIEW, Some(true))
        .await?;
    grants
        .set_direct(&admin, STAFF, PASSPORT_GENERATE, Some(false))
        .await?;
    refused(
        "generate without generation permission",
        training
            .generate_passport(&staff, COMPANY, PassportAudience::SelfService, staff_id)
            .await,
        TrainingError::Unauthorized,
    )?;
    proof(
        "generation revocation preserves authorized download",
        saved(
            training
                .read_passport(&staff, COMPANY, &first_document.id)
                .await,
        )?
        .bytes
            == first_bytes,
    )?;
    grants
        .set_direct(&admin, STAFF, PASSPORT_GENERATE, Some(true))
        .await?;
    users
        .unlink(&admin, STAFF)
        .await
        .map_err(|_| Error::Internal)?;
    refused(
        "unlinked account read with stale scope",
        training
            .read_passport(&staff, COMPANY, &first_document.id)
            .await,
        TrainingError::Unauthorized,
    )?;
    refused(
        "unlinked account evidence with stale scope",
        training
            .read_passport_evidence(&staff, COMPANY, &evidence.artifact_id)
            .await,
        TrainingError::Unauthorized,
    )?;
    relink().await?;
    let current = workforce
        .employment(&manager, COMPANY, staff_id)
        .await
        .map_err(|_| Error::Internal)?
        .require_current()
        .map_err(|_| Error::Internal)?;
    workforce
        .update_employment(
            &manager,
            COMPANY,
            staff_id,
            &EmploymentChange {
                supervisor_employment_id: Some(None),
                ..EmploymentChange::at(current.version)
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    refused(
        "former direct manager loses document access",
        training
            .read_passport(&manager, COMPANY, &team_document.id)
            .await,
        TrainingError::Unauthorized,
    )?;
    workforce
        .update_employment(
            &manager,
            COMPANY,
            staff_id,
            &EmploymentChange {
                supervisor_employment_id: Some(Some(own)),
                ..EmploymentChange::at(current.version + 1)
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    let page = saved(
        training
            .passport(
                &staff,
                COMPANY,
                PassportAudience::SelfService,
                staff_id,
                &HashMap::from([
                    ("page".into(), i64::MAX.to_string()),
                    ("perPage".into(), "999".into()),
                ]),
            )
            .await,
    )?;
    proof(
        "pagination is bounded and clamps",
        page.page.page_size == 25 && page.page.page == page.page.total_pages.max(1),
    )?;
    // Complete-document bound: 1,000 accepted, 1,001 refused with no artifact.
    let limit_event = event(&training, &manager, "pt5-limit", "Passport document limit").await?;
    for n in 1..=1001 {
        let s = session_for(
            &training,
            &manager,
            limit_event.id,
            &format!("Limit session {n:04}"),
        )
        .await?;
        record(
            &training,
            &manager,
            s.id,
            limit_employee.id,
            &format!("pt5-limit-{n}"),
            AttendanceStatus::Confirmed,
        )
        .await?;
        if n == 1000 {
            let artifact = saved(
                training
                    .generate_passport(&manager, COMPANY, PassportAudience::Team, limit_employee.id)
                    .await,
            )?;
            proof(
                "1,000-record PDF succeeds",
                saved(
                    training
                        .read_passport(&manager, COMPANY, &artifact.id)
                        .await,
                )?
                .bytes
                .starts_with(b"%PDF-"),
            )?;
        }
    }
    let artifact_count:i64=client.query_one("SELECT count(*) FROM base_artifacts WHERE tenant_id=$1 AND company_id=$2 AND owner_id='people/training'",&[&TENANT,&COMPANY]).await?.get(0);
    refused(
        "1,001-record PDF refuses",
        training
            .generate_passport(&manager, COMPANY, PassportAudience::Team, limit_employee.id)
            .await,
        TrainingError::PassportTooLarge,
    )?;
    proof("oversized passport stores no partial PDF",client.query_one("SELECT count(*) FROM base_artifacts WHERE tenant_id=$1 AND company_id=$2 AND owner_id='people/training'",&[&TENANT,&COMPANY]).await?.get::<_,i64>(0)==artifact_count)?;
    let passport = saved(
        training
            .passport(
                &manager,
                COMPANY,
                PassportAudience::Team,
                limit_employee.id,
                &HashMap::from([
                    ("page".into(), i64::MAX.to_string()),
                    ("perPage".into(), "300".into()),
                ]),
            )
            .await,
    )?;
    proof(
        "1,001 records remain available with 300-row paging",
        passport.page.total_entries == 1001
            && passport.page.page == 4
            && passport.page.entries.len() == 101,
    )?;
    println!("PT5 browser downloads: my document=/people/training/records/my/document/{COMPANY}/{}; team document=/people/training/records/team/document/{COMPANY}/{}; my evidence=/people/training/records/my/evidence/{COMPANY}/{}; team evidence=/people/training/records/team/evidence/{COMPANY}/{}",first_document.id,team_document.id,evidence.artifact_id,evidence.artifact_id);
    println!("PT5 proof complete.");
    Ok(())
}
async fn session_for(
    training: &Training,
    scope: &Scope,
    event: i64,
    name: &str,
) -> Result<Session> {
    session(
        training,
        scope,
        event,
        name,
        "2026-07-03T08:00",
        "2026-07-03T09:00",
    )
    .await
}
