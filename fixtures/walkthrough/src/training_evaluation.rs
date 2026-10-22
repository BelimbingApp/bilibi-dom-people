//! PT4 executable proof: real facade calls and rollback-only database refusals.
use crate::{COMPANY, MANAGER, STAFF, TENANT};
use bilibi_affiliation::{AffiliationInput, AffiliationService, EffectiveDates};
use bilibi_authz::grants::Grants;
use bilibi_database::{Error, Result};
use bilibi_module_registry::Services;
use bilibi_party_ids::{AffiliationId, OrganisationId, PersonId};
use bilibi_people_training::*;
use bilibi_people_workforce::{EmploymentChange, NewEmployment, Workforce};
use bilibi_person::{PersonInput, PersonKind, PersonService};
use bilibi_tenancy::{Authentication, Scope};
use bilibi_user::UserService;
use serde_json::json;
use std::fmt::Debug;

fn saved<T>(r: bilibi_people_training::Result<T>) -> Result<T> {
    r.map_err(|e| {
        eprintln!("PT4 unexpected: {e:?}");
        Error::Internal
    })
}
fn proof(label: &str, holds: bool) -> Result<()> {
    if !holds {
        eprintln!("PT4 FAILED: {label}");
        return Err(Error::Internal);
    }
    println!("PT4 proved: {label}");
    Ok(())
}
fn refused<T: Debug>(
    label: &str,
    r: bilibi_people_training::Result<T>,
    expected: TrainingError,
) -> Result<()> {
    match r {
        Err(e) if std::mem::discriminant(&e) == std::mem::discriminant(&expected) => {
            println!("PT4 refused {label}: {e}");
            Ok(())
        }
        other => {
            eprintln!("PT4 FAILED {label}: expected {expected:?}, got {other:?}");
            Err(Error::Internal)
        }
    }
}
fn criteria() -> Vec<EvaluationCriterion> {
    vec![
        EvaluationCriterion {
            code: "outcome".into(),
            label: "Observed outcome".into(),
            minimum: -10000,
            maximum: 10000,
        },
        EvaluationCriterion {
            code: "uncertain".into(),
            label: "Unobserved outcome".into(),
            minimum: 0,
            maximum: 5,
        },
    ]
}
fn policy() -> NewEvaluationPolicy {
    NewEvaluationPolicy {
        effective_from: "2026-01-01".into(),
        effective_to: "2026-12-31".into(),
        criteria: criteria(),
        effectiveness_criteria: criteria(),
        reason: "Capture the agreed review criteria".into(),
    }
}
fn settings() -> EvaluationSettings {
    EvaluationSettings {
        evaluation_days: Some(0),
        checkpoints: Some(vec![1, 7]),
        reminder_days: Some(0),
        minimum_cohort: Some(2),
        report_months: Some(3),
        report_grace_days: Some(2),
    }
}
fn attendance(
    session_id: i64,
    employee_id: i64,
    key: &str,
    status: AttendanceStatus,
) -> NewAttendance {
    NewAttendance {
        session_id,
        employee_id,
        status,
        import_key: format!("pt4-{key}"),
        reason: "Checked the session register".into(),
    }
}
pub(crate) async fn run(
    services: &Services,
    client: &tokio_postgres::Client,
    admin: &Scope,
    manager: &Scope,
    employees: (i64, i64),
) -> Result<()> {
    let training = services.get::<Training>(SERVICE)?;
    let workforce = services.get::<Workforce>(bilibi_people_workforce::SERVICE)?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    let users = services.get::<UserService>("core/user.accounts")?;
    let auth = services.get::<Authentication>("base/tenancy.authentication")?;
    for user in [MANAGER, STAFF] {
        for key in [
            EFFECTIVENESS_VIEW,
            EVALUATION_SUBMIT,
            EFFECTIVENESS_ANSWER,
            EFFECTIVENESS_SUMMARY_VIEW,
            EVALUATION_POLICY_MANAGE,
            EVALUATION_REMINDERS_MANAGE,
            REQUESTS_SUBMIT,
        ] {
            grants.set_direct(admin, user, key, Some(true)).await?;
        }
    }
    let employment = workforce
        .employment(manager, COMPANY, employees.1)
        .await
        .map_err(|_| Error::Internal)?
        .require_current()
        .map_err(|_| Error::Internal)?;
    users
        .link_person(
            admin,
            STAFF,
            PersonId::new(employment.person_id).map_err(|_| Error::Invalid)?,
        )
        .await
        .map_err(|_| Error::Internal)?;
    users
        .link_affiliation(
            admin,
            STAFF,
            AffiliationId::new(employment.affiliation_id).map_err(|_| Error::Invalid)?,
        )
        .await
        .map_err(|_| Error::Internal)?;
    let staff = auth
        .authenticate(client, TENANT, STAFF, "walkthrough".into())
        .await?;
    let defaults = saved(training.evaluation_settings(manager, COMPANY).await)?;
    proof(
        "only report months has a default",
        defaults.evaluation_days.is_none()
            && defaults.checkpoints.is_none()
            && defaults.report_months == Some(3),
    )?;
    refused(
        "unconfigured publication",
        training
            .publish_evaluation_policy(manager, COMPANY, &policy())
            .await,
        TrainingError::EvaluationPolicyNotConfigured,
    )?;
    refused(
        "unconfigured report",
        training
            .freeze_effectiveness_summaries(
                manager,
                COMPANY,
                "2026-04-03".parse().map_err(|_| Error::Invalid)?,
            )
            .await,
        TrainingError::EffectivenessReportNotConfigured,
    )?;
    let mut invalid = settings();
    invalid.checkpoints = Some(vec![1, 1]);
    refused(
        "duplicate checkpoints",
        training
            .put_evaluation_settings(manager, COMPANY, &invalid)
            .await,
        TrainingError::InvalidEvaluationSettings,
    )?;
    invalid = settings();
    invalid.report_months = Some(5);
    refused(
        "non-divisor reporting months",
        training
            .put_evaluation_settings(manager, COMPANY, &invalid)
            .await,
        TrainingError::InvalidEvaluationSettings,
    )?;
    invalid = settings();
    invalid.minimum_cohort = Some(1);
    refused(
        "disclosure minimum below two",
        training
            .put_evaluation_settings(manager, COMPANY, &invalid)
            .await,
        TrainingError::InvalidEvaluationSettings,
    )?;
    proof(
        "invalid settings write nothing",
        saved(training.evaluation_settings(manager, COMPANY).await)? == defaults,
    )?;
    saved(
        training
            .put_evaluation_settings(manager, COMPANY, &settings())
            .await,
    )?;
    let mut invalid_policy = policy();
    invalid_policy.criteria[0].code = "Invalid".into();
    refused(
        "invalid criterion",
        training
            .publish_evaluation_policy(manager, COMPANY, &invalid_policy)
            .await,
        TrainingError::InvalidCriteria,
    )?;
    invalid_policy = policy();
    invalid_policy.reason = " ".into();
    refused(
        "blank publication reason",
        training
            .publish_evaluation_policy(manager, COMPANY, &invalid_policy)
            .await,
        TrainingError::EvaluationReasonRequired,
    )?;
    let published = saved(
        training
            .publish_evaluation_policy(manager, COMPANY, &policy())
            .await,
    )?;
    refused(
        "overlap",
        training
            .publish_evaluation_policy(manager, COMPANY, &policy())
            .await,
        TrainingError::EvaluationPolicyOverlap,
    )?;
    proof(
        "next policy version",
        published.version == 1 && published.checkpoints.days == vec![1, 7],
    )?;
    let persons = services.get::<PersonService>(bilibi_person::SERVICE)?;
    let affiliations = services.get::<AffiliationService>(bilibi_affiliation::SERVICE)?;
    let person = persons
        .create(
            manager,
            &PersonInput {
                name: "Evaluation learner".into(),
                kind: PersonKind::Human,
                email: None,
                phone: None,
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    let affiliation = affiliations
        .create(
            manager,
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
    let second = workforce
        .create_employment(
            manager,
            COMPANY,
            &NewEmployment {
                supervisor_employment_id: Some(employees.0),
                ..NewEmployment::new(
                    person.id.get(),
                    affiliation.id.get(),
                    "PT4-002",
                    &employment.employment_type,
                    "active",
                )
            },
        )
        .await
        .map_err(|e| {
            eprintln!("PT4 employee: {e}");
            Error::Internal
        })?;
    let course = saved(
        training
            .create_course(
                manager,
                COMPANY,
                &NewCourse {
                    code: "pt4-evaluation".into(),
                    name: "Evaluation proof course".into(),
                    description: None,
                },
            )
            .await,
    )?;
    let event = saved(
        training
            .create_event(
                manager,
                COMPANY,
                &NewEvent {
                    course_id: course.id,
                    name: "Evaluation proof event".into(),
                    capacity: 3,
                },
            )
            .await,
    )?;
    let session = saved(
        training
            .create_session(
                manager,
                COMPANY,
                &NewSession {
                    event_id: event.id,
                    name: "Completed session".into(),
                    capacity: 3,
                    time_zone: "Asia/Kuala_Lumpur".into(),
                    starts_local: "2026-01-31T22:00".into(),
                    ends_local: "2026-02-01T00:30".into(),
                },
            )
            .await,
    )?;
    for (id, key) in [
        (employees.0, "manager"),
        (employees.1, "staff"),
        (second.id, "second"),
    ] {
        saved(
            training
                .record(
                    manager,
                    COMPANY,
                    &attendance(session.id, id, key, AttendanceStatus::Confirmed),
                )
                .await,
        )?;
    }
    refused(
        "unfinished session",
        training
            .prepare_evaluation_reviews(
                manager,
                COMPANY,
                session.id,
                "2026-01-31T16:29:00Z".parse().map_err(|_| Error::Invalid)?,
            )
            .await,
        TrainingError::SessionNotFinished,
    )?;
    let prepared = saved(
        training
            .prepare_evaluation_reviews(
                manager,
                COMPANY,
                session.id,
                "2026-01-31T16:30:00Z".parse().map_err(|_| Error::Invalid)?,
            )
            .await,
    )?;
    proof(
        "local completion and checkpoints",
        prepared.reviews.len() == 9
            && prepared.unknown == 0
            && prepared
                .reviews
                .iter()
                .filter(|r| r.kind == EvaluationKind::Evaluation)
                .all(|r| r.due_on == "2026-02-01"),
    )?;
    let replay = saved(
        training
            .prepare_evaluation_reviews(
                manager,
                COMPANY,
                session.id,
                "2026-02-01T00:00:00Z".parse().map_err(|_| Error::Invalid)?,
            )
            .await,
    )?;
    proof(
        "prepare is idempotent",
        prepared.reviews.iter().map(|r| r.id).collect::<Vec<_>>()
            == replay.reviews.iter().map(|r| r.id).collect::<Vec<_>>(),
    )?;
    let own = saved(
        training
            .evaluation_reviews(&staff, COMPANY, EvaluationKind::Evaluation)
            .await,
    )?;
    proof(
        "employee sees only own evaluation",
        own.len() == 1 && own[0].employee_id == employees.1,
    )?;
    let own_id = own[0].review.id;
    for value in [
        json!({}),
        json!({"outcome":10001,"uncertain":null}),
        json!({"outcome":1.5,"uncertain":null}),
        json!({"outcome":true,"uncertain":null}),
        json!({"outcome":1,"uncertain":null,"extra":1}),
    ] {
        refused(
            "exact bounded answers",
            training
                .answer_evaluation(&staff, COMPANY, own_id, &value, "Observed")
                .await,
            TrainingError::InvalidAnswers,
        )?;
    }
    refused(
        "blank answer reason",
        training
            .answer_evaluation(
                &staff,
                COMPANY,
                own_id,
                &json!({"outcome":1,"uncertain":null}),
                " ",
            )
            .await,
        TrainingError::EvaluationReasonRequired,
    )?;
    saved(
        training
            .answer_evaluation(
                &staff,
                COMPANY,
                own_id,
                &json!({"outcome":-10000,"uncertain":null}),
                "Observed minimum and explicit unknown",
            )
            .await,
    )?;
    refused(
        "answer once",
        training
            .answer_evaluation(
                &staff,
                COMPANY,
                own_id,
                &json!({"outcome":1,"uncertain":null}),
                "Again",
            )
            .await,
        TrainingError::AlreadyAnswered,
    )?;
    let tasks = saved(
        training
            .evaluation_reviews(manager, COMPANY, EvaluationKind::Effectiveness)
            .await,
    )?;
    proof(
        "only current direct reports",
        tasks.len() == 4 && tasks.iter().all(|t| t.employee_id != employees.0),
    )?;
    for task in tasks.iter().filter(|t| t.review.checkpoint_days == 1) {
        let score = if task.employee_id == employees.1 {
            1
        } else {
            2
        };
        saved(
            training
                .answer_evaluation(
                    manager,
                    COMPANY,
                    task.review.id,
                    &json!({"outcome":score,"uncertain":null}),
                    "Observed in the workplace",
                )
                .await,
        )?;
    }
    let delayed = tasks
        .iter()
        .find(|t| t.employee_id == employees.1 && t.review.checkpoint_days == 7)
        .ok_or(Error::Internal)?
        .review
        .id;
    refused(
        "learner cannot answer supervisor review",
        training
            .answer_evaluation(
                &staff,
                COMPANY,
                delayed,
                &json!({"outcome":1,"uncertain":null}),
                "Not my team",
            )
            .await,
        TrainingError::OutsideDirectTeam,
    )?;
    // The same sealed scope must see grant revocation immediately.
    grants
        .set_direct(admin, MANAGER, EFFECTIVENESS_ANSWER, Some(false))
        .await?;
    refused(
        "revoked grant",
        training
            .answer_evaluation(
                manager,
                COMPANY,
                delayed,
                &json!({"outcome":null,"uncertain":null}),
                "Unknown",
            )
            .await,
        TrainingError::Unauthorized,
    )?;
    grants
        .set_direct(admin, MANAGER, EFFECTIVENESS_ANSWER, Some(true))
        .await?;
    // A latest absent revision hides all earlier confirmed obligations.
    saved(
        training
            .record(
                manager,
                COMPANY,
                &attendance(session.id, employees.1, "absent", AttendanceStatus::Absent),
            )
            .await,
    )?;
    refused(
        "latest attendance absent",
        training
            .answer_evaluation(
                manager,
                COMPANY,
                delayed,
                &json!({"outcome":null,"uncertain":null}),
                "Unknown",
            )
            .await,
        TrainingError::AttendanceUnavailable,
    )?;
    saved(
        training
            .record(
                manager,
                COMPANY,
                &attendance(
                    session.id,
                    employees.1,
                    "reconfirmed",
                    AttendanceStatus::Confirmed,
                ),
            )
            .await,
    )?;
    let corrected = saved(
        training
            .prepare_evaluation_reviews(
                manager,
                COMPANY,
                session.id,
                "2026-02-10T00:00:00Z".parse().map_err(|_| Error::Invalid)?,
            )
            .await,
    )?;
    proof(
        "confirmed correction keeps obligations",
        corrected.reviews.len() == 9
            && corrected.reviews.iter().map(|r| r.id).collect::<Vec<_>>()
                == prepared.reviews.iter().map(|r| r.id).collect::<Vec<_>>(),
    )?;
    let reminders = saved(
        training
            .evaluation_reminders(
                manager,
                COMPANY,
                "2026-02-08".parse().map_err(|_| Error::Invalid)?,
            )
            .await,
    )?;
    proof(
        "worklist distinguishes unaddressable reviews",
        reminders.created == 4 && reminders.unknown == 2,
    )?;
    let replay = saved(
        training
            .evaluation_reminders(
                manager,
                COMPANY,
                "2026-02-08".parse().map_err(|_| Error::Invalid)?,
            )
            .await,
    )?;
    proof("reminders once", replay.created == 0 && replay.unknown == 2)?;
    proof(
        "first fixed reporting period",
        saved(training.effectiveness_period_start(manager, COMPANY).await)?
            == Some("2026-01-01".into()),
    )?;
    proof(
        "strict grace boundary",
        saved(
            training
                .freeze_effectiveness_summaries(
                    manager,
                    COMPANY,
                    "2026-04-02".parse().map_err(|_| Error::Invalid)?,
                )
                .await,
        )?
        .frozen
            == 0,
    )?;
    proof(
        "freeze after grace",
        saved(
            training
                .freeze_effectiveness_summaries(
                    manager,
                    COMPANY,
                    "2026-04-03".parse().map_err(|_| Error::Invalid)?,
                )
                .await,
        )?
        .frozen
            == 1,
    )?;
    let frozen = saved(training.effectiveness_summary(manager, COMPANY).await)?;
    let first = frozen.first().ok_or(Error::Internal)?;
    proof(
        "cohort disclosed",
        first.status == "current" && first.groups.items.len() == 2,
    )?;
    let group = first
        .groups
        .items
        .iter()
        .find(|g| g.checkpoint_days == 1)
        .ok_or(Error::Internal)?;
    proof(
        "exact mean and answered minimum",
        group.criteria[0].mean.as_deref() == Some("1.50") && group.criteria[0].answered == Some(2),
    )?;
    proof(
        "unknown criterion suppresses count and mean",
        group.criteria[1].mean.is_none()
            && group.criteria[1].answered.is_none()
            && group.criteria[1].status == "suppressed",
    )?;
    let mut monthly = settings();
    monthly.report_months = Some(1);
    saved(
        training
            .put_evaluation_settings(manager, COMPANY, &monthly)
            .await,
    )?;
    proof(
        "changed reporting length starts after last frozen period",
        saved(training.effectiveness_period_start(manager, COMPANY).await)?
            == Some("2026-04-01".into()),
    )?;
    proof(
        "empty periods remain contiguous",
        saved(
            training
                .freeze_effectiveness_summaries(
                    manager,
                    COMPANY,
                    "2026-07-02".parse().map_err(|_| Error::Invalid)?,
                )
                .await,
        )?
        .frozen
            == 2,
    )?;
    let all = saved(training.effectiveness_summary(manager, COMPANY).await)?;
    proof(
        "whole-cohort suppression hides all groups",
        all.len() == 3
            && all[..2]
                .iter()
                .all(|s| s.status == "suppressed" && s.groups.items.is_empty()),
    )?;
    saved(
        training
            .answer_evaluation(
                manager,
                COMPANY,
                delayed,
                &json!({"outcome":10000,"uncertain":5}),
                "Late answer retained in history",
            )
            .await,
    )?;
    proof(
        "late answers do not change frozen scores",
        saved(training.effectiveness_summary(manager, COMPANY).await)?.last() == frozen.last(),
    )?;
    proof(
        "repeated freeze is a no-op",
        saved(
            training
                .freeze_effectiveness_summaries(
                    manager,
                    COMPANY,
                    "2026-07-02".parse().map_err(|_| Error::Invalid)?,
                )
                .await,
        )?
        .frozen
            == 0,
    )?;
    let own_manager = saved(
        training
            .evaluation_reviews(manager, COMPANY, EvaluationKind::Evaluation)
            .await,
    )?;
    refused(
        "other learner evaluation",
        training
            .answer_evaluation(
                &staff,
                COMPANY,
                own_manager[0].review.id,
                &json!({"outcome":1,"uncertain":null}),
                "Other learner",
            )
            .await,
        TrainingError::OutsideDirectTeam,
    )?;
    users
        .unlink(admin, STAFF)
        .await
        .map_err(|_| Error::Internal)?;
    refused(
        "unlinked account",
        training
            .answer_evaluation(
                &staff,
                COMPANY,
                own_id,
                &json!({"outcome":1,"uncertain":null}),
                "Old employee",
            )
            .await,
        TrainingError::EmployeeUnavailable,
    )?;
    // Current reporting lines and competing answers are rechecked under the company lock.
    let report = workforce
        .employment(manager, COMPANY, second.id)
        .await
        .map_err(|_| Error::Internal)?
        .require_current()
        .map_err(|_| Error::Internal)?;
    let report = workforce
        .update_employment(
            manager,
            COMPANY,
            second.id,
            &EmploymentChange {
                supervisor_employment_id: Some(None),
                ..EmploymentChange::at(report.version)
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    let second_delayed = tasks
        .iter()
        .find(|t| t.employee_id == second.id && t.review.checkpoint_days == 7)
        .ok_or(Error::Internal)?
        .review
        .id;
    refused(
        "changed reporting line",
        training
            .answer_evaluation(
                manager,
                COMPANY,
                second_delayed,
                &json!({"outcome":null,"uncertain":null}),
                "Unknown",
            )
            .await,
        TrainingError::OutsideDirectTeam,
    )?;
    workforce
        .update_employment(
            manager,
            COMPANY,
            second.id,
            &EmploymentChange {
                supervisor_employment_id: Some(Some(employees.0)),
                ..EmploymentChange::at(report.version)
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    let race_values = json!({"outcome":null,"uncertain":null});
    let (a, b) = tokio::join!(
        training.answer_evaluation(manager, COMPANY, second_delayed, &race_values, "Unknown"),
        training.answer_evaluation(manager, COMPANY, second_delayed, &race_values, "Unknown")
    );
    proof(
        "competing answers commit once",
        matches!(
            (&a, &b),
            (Ok(_), Err(TrainingError::AlreadyAnswered))
                | (Err(TrainingError::AlreadyAnswered), Ok(_))
        ),
    )?;
    // Leavers remain attendance history, but never get new review obligations.
    let current = workforce
        .employment(manager, COMPANY, second.id)
        .await
        .map_err(|_| Error::Internal)?
        .require_current()
        .map_err(|_| Error::Internal)?;
    workforce
        .update_employment(
            manager,
            COMPANY,
            second.id,
            &EmploymentChange {
                status: Some("terminated".into()),
                ..EmploymentChange::at(current.version)
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    let leavers = saved(
        training
            .prepare_evaluation_reviews(
                manager,
                COMPANY,
                session.id,
                "2026-08-01T00:00:00Z".parse().map_err(|_| Error::Invalid)?,
            )
            .await,
    )?;
    proof(
        "leavers counted unknown",
        leavers.unknown == 1 && leavers.reviews.len() == 6,
    )?;
    // The owner tables reject all rewrites and direct out-of-bounds answers.
    for (label,sql,id) in [
        ("immutable policy","UPDATE people_training_evaluation_policies SET reason='rewrite' WHERE id=$1",published.id),
        ("immutable review","DELETE FROM people_training_evaluation_reviews WHERE id=$1",delayed),
        ("immutable answer","DELETE FROM people_training_evaluation_answers WHERE review_id=$1",own_id),
        ("immutable reminder","DELETE FROM people_training_review_reminders WHERE review_id=$1",delayed),
        ("immutable summary","UPDATE people_training_effectiveness_summaries SET status='suppressed' WHERE id=$1",first.id),
        ("database score bounds","INSERT INTO people_training_evaluation_answers (tenant_id,company_id,actor_user_id,review_id,values,reason) SELECT tenant_id,company_id,actor_user_id,id,'{\"outcome\":10001,\"uncertain\":null}'::jsonb,'invalid' FROM people_training_evaluation_reviews WHERE id=$1",own_manager[0].review.id),
        ("duplicate obligation on corrected fact","INSERT INTO people_training_evaluation_reviews (tenant_id,company_id,actor_user_id,policy_id,fact_id,kind,checkpoint_days,due_on) SELECT r.tenant_id,r.company_id,r.actor_user_id,r.policy_id,(SELECT max(f.id) FROM people_training_attendance_facts f JOIN people_training_attendance_facts original ON original.id=r.fact_id AND original.tenant_id=f.tenant_id AND original.company_id=f.company_id WHERE f.tenant_id=r.tenant_id AND f.company_id=r.company_id AND f.session_id=original.session_id AND f.employee_id=original.employee_id),r.kind,r.checkpoint_days,r.due_on FROM people_training_evaluation_reviews r WHERE r.id=$1",own_id),
    ] {
        client.batch_execute("BEGIN").await?;
        let result=client.execute(sql,&[&id]).await;
        client.batch_execute("ROLLBACK").await?;
        proof(label,result.as_ref().err().and_then(|e|e.code()).is_some_and(|c|c.code()=="23514"))?;
    }
    let audit:i64=client.query_one("SELECT count(*) FROM base_audit_mutations WHERE tenant_id=1 AND auditable_type LIKE 'people_training_evaluation_%'",&[]).await?.get(0);
    proof("writes captured by audit", audit > 0)?;
    println!("PT4 evaluation and effectiveness proof complete.");
    Ok(())
}
