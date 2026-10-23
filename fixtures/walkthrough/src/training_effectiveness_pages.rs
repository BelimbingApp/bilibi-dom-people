//! PT9 mounted-page proof and browser records, on a disposable walkthrough database.
use crate::{ADMIN, COMPANY, MANAGER, STAFF, TENANT, VIEWER};
use bilibi_authz::grants::Grants;
use bilibi_database::{Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_training::*;
use bilibi_people_workforce::Workforce;
use bilibi_tenancy::{Authentication, Scope};
use bilibi_ui::page::{LiveBody, LiveInput, LiveRequest, LiveState, PageRequest, PageService};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc};
type Mounted = Box<dyn LiveState<Scope, Error>>;
fn proof(ok: bool, message: &str) -> Result<()> {
    if !ok {
        eprintln!("PT9 FAILED: {message}");
        return Err(Error::Internal);
    }
    println!("PT9 proved: {message}");
    Ok(())
}
fn saved<T>(result: bilibi_people_training::Result<T>) -> Result<T> {
    result.map_err(|e| {
        eprintln!("PT9 unexpected: {e:?}");
        Error::Internal
    })
}
async fn mount(
    page: &PageService<Scope, Error>,
    scope: &Scope,
    operation: &'static str,
    query: &HashMap<String, String>,
) -> Result<Mounted> {
    page.0
        .mount(PageRequest {
            operation,
            scope: Some(scope),
            path: &HashMap::new(),
            query,
            body: &[],
            framed: false,
        })
        .await?
        .ok_or(Error::Internal)
}
async fn event(state: &mut Mounted, scope: &Scope, value: Value) -> Result<LiveBody> {
    state
        .update(LiveRequest {
            scope,
            credential: "walkthrough",
            input: LiveInput::Event(&value),
        })
        .await
}
async fn refresh(state: &mut Mounted, scope: &Scope) -> Result<LiveBody> {
    state
        .update(LiveRequest {
            scope,
            credential: "walkthrough",
            input: LiveInput::Refresh,
        })
        .await
}

pub async fn run(url: &str, action: &str) -> Result<()> {
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = bilibi_database::pool(url)?;
    let modules = bilibi_module_registry::discover()?;
    if action == "setup" {
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
    let auth = services.get::<Authentication>("base/tenancy.authentication")?;
    let admin = auth
        .authenticate(&**client, TENANT, ADMIN, "walkthrough".into())
        .await?;
    let manager = auth
        .authenticate(&**client, TENANT, MANAGER, "walkthrough".into())
        .await?;
    let viewer = auth
        .authenticate(&**client, TENANT, VIEWER, "walkthrough".into())
        .await?;
    let training = services.get::<Training>(SERVICE)?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    if action == "setup" {
        setup(&services, &admin, &manager).await?;
    }
    let staff = auth
        .authenticate(&**client, TENANT, STAFF, "walkthrough".into())
        .await?;
    if action == "deny-answer" || action == "allow-answer" {
        grants
            .set_direct(
                &admin,
                MANAGER,
                EFFECTIVENESS_ANSWER,
                Some(action == "allow-answer"),
            )
            .await?;
        return Ok(());
    }
    if action == "deny-policy" || action == "allow-policy" {
        grants
            .set_direct(
                &admin,
                MANAGER,
                EVALUATION_POLICY_MANAGE,
                Some(action == "allow-policy"),
            )
            .await?;
        return Ok(());
    }
    if matches!(
        action,
        "unlink-staff" | "relink-staff" | "archive-company-three"
    ) {
        return crate::training_learning_control(url, action).await;
    }
    for cap in [
        INSIGHTS_VIEW,
        RECORDS_VIEW,
        EFFECTIVENESS_VIEW,
        EFFECTIVENESS_ANSWER,
        EVALUATION_POLICY_MANAGE,
        EVALUATION_REMINDERS_MANAGE,
        EFFECTIVENESS_SUMMARY_VIEW,
    ] {
        grants.set_direct(&admin, MANAGER, cap, Some(true)).await?;
    }
    for cap in [
        "admin.company.tenant-wide.manage",
        INSIGHTS_VIEW,
        EFFECTIVENESS_VIEW,
        EFFECTIVENESS_SUMMARY_VIEW,
        LEARNING_VIEW,
    ] {
        grants.set_direct(&admin, VIEWER, cap, Some(true)).await?;
    }
    for cap in [
        RECORDS_VIEW,
        EFFECTIVENESS_ANSWER,
        EVALUATION_POLICY_MANAGE,
        EVALUATION_REMINDERS_MANAGE,
    ] {
        grants.set_direct(&admin, VIEWER, cap, Some(false)).await?;
    }
    for cap in [LEARNING_VIEW, REQUESTS_SUBMIT, EVALUATION_SUBMIT] {
        grants.set_direct(&admin, STAFF, cap, Some(true)).await?;
    }
    let workforce = services.get::<Workforce>(bilibi_people_workforce::SERVICE)?;
    let staff_id = workforce
        .self_employee(&staff, COMPANY)
        .await
        .map_err(|_| Error::Internal)?
        .employee_id;
    let page = services.get::<PageService<Scope, Error>>("people/training.effectiveness_pages")?;
    let insights = services.get::<PageService<Scope, Error>>("people/training.insights_pages")?;
    let learning = services.get::<PageService<Scope, Error>>("people/training.learning_pages")?;
    let query = HashMap::from([("company_id".into(), COMPANY.to_string())]);
    let proposal_query = HashMap::from([("company_id".into(), "3".into())]);
    let mut proposed = mount(&page, &manager, "effectiveness", &proposal_query).await?;
    let body = refresh(&mut proposed, &manager).await?;
    proof(
        body.html.contains("value=\"30, 90\"")
            && body.html.contains("nothing applies until you save"),
        "unsaved company displays proposed settings",
    )?;
    proof(
        saved(training.evaluation_settings(&manager, 3).await)?
            .evaluation_days
            .is_none(),
        "displaying proposed values writes no company setting",
    )?;
    let mut state = mount(&page, &manager, "effectiveness", &query).await?;
    let body = refresh(&mut state, &manager).await?;
    proof(
        body.html.contains("effectiveness-reviews")
            && body.html.contains("effectiveness-summary")
            && body.html.contains("evaluation-policies")
            && body.html.contains("evaluation-settings-form"),
        "HOD, HR and policy FlexTables render together",
    )?;
    let mut restricted = mount(&page, &viewer, "effectiveness", &query).await?;
    let body = refresh(&mut restricted, &viewer).await?;
    proof(
        body.html.contains("effectiveness-summary")
            && !body.html.contains("evaluation-settings-form")
            && !body.html.contains("prepare-review-form"),
        "HR reader sees frozen summaries with withheld write controls",
    )?;
    let body = event(&mut restricted, &viewer, json!({"type":"open_policy"})).await?;
    proof(
        body.html.contains("You cannot publish this company")
            && !body.html.contains("evaluation-policy-modal"),
        "forged policy event is a recoverable refusal",
    )?;
    // Add enough criteria to cross the browser form_values limit, then cancel publication.
    event(&mut state, &manager, json!({"type":"open_policy"})).await?;
    for _ in 1..9 {
        event(
            &mut state,
            &manager,
            json!({"type":"add_criterion","value":"criteria"}),
        )
        .await?;
        event(
            &mut state,
            &manager,
            json!({"type":"add_criterion","value":"effectiveness_criteria"}),
        )
        .await?;
    }
    let mut publication = json!({"type":"save_policy","entry[effective_from]":"2027-01-01","entry[effective_to]":"2027-12-31","entry[reason]":"PT9 broad policy page proof"});
    for kind in ["criteria", "effectiveness_criteria"] {
        for i in 1..=9 {
            for (key, value) in [
                ("code", format!("criterion_{i}")),
                ("label", format!("Observed result {i}")),
                ("minimum", "0".into()),
                ("maximum", "5".into()),
            ] {
                publication[format!("entry[{kind}][{i}][{key}]")] = json!(value);
            }
        }
    }
    let body = event(&mut state, &manager, publication.clone()).await?;
    proof(
        body.html.contains("evaluation-policy-confirm"),
        "policy form reads all 76 fields and waits for confirmation",
    )?;
    let before = saved(training.evaluation_policies(&manager, COMPANY).await)?.len();
    event(&mut state, &manager, json!({"type":"cancel_confirm"})).await?;
    proof(
        saved(training.evaluation_policies(&manager, COMPANY).await)?.len() == before,
        "cancelled publication writes nothing",
    )?;
    event(&mut state, &manager, json!({"type":"open_policy"})).await?;
    for _ in 1..9 {
        event(
            &mut state,
            &manager,
            json!({"type":"add_criterion","value":"criteria"}),
        )
        .await?;
        event(
            &mut state,
            &manager,
            json!({"type":"add_criterion","value":"effectiveness_criteria"}),
        )
        .await?;
    }
    event(&mut state, &manager, publication).await?;
    let body = event(
        &mut state,
        &manager,
        json!({"type":"confirm_policy","entry[reason]":"tampered","company_id":"100"}),
    )
    .await?;
    proof(
        body.html.contains("Policy version published."),
        "confirmed publication uses server-held input",
    )?;
    let policy = saved(training.evaluation_policies(&manager, COMPANY).await)?.remove(0);
    proof(
        policy.criteria.items.len() == 9 && policy.effectiveness_criteria.items.len() == 9,
        "every criterion survives the 64-field boundary",
    )?;
    let course = saved(
        training
            .create_course(
                &manager,
                COMPANY,
                &NewCourse {
                    code: "pt9_page_proof".into(),
                    name: "Review practice".into(),
                    description: None,
                },
            )
            .await,
    )?;
    let delivery = saved(
        training
            .create_event(
                &manager,
                COMPANY,
                &NewEvent {
                    course_id: course.id,
                    name: "Review practice".into(),
                    capacity: 10,
                },
            )
            .await,
    )?;
    let session = saved(
        training
            .create_session(
                &manager,
                COMPANY,
                &NewSession {
                    event_id: delivery.id,
                    name: "Completed review practice".into(),
                    capacity: 10,
                    time_zone: "UTC".into(),
                    starts_local: "2026-09-01T09:00".into(),
                    ends_local: "2026-09-01T10:00".into(),
                },
            )
            .await,
    )?;
    saved(
        training
            .record(
                &manager,
                COMPANY,
                &NewAttendance {
                    session_id: session.id,
                    employee_id: staff_id,
                    status: AttendanceStatus::Confirmed,
                    import_key: "pt9_attended".into(),
                    reason: "Observed attendance".into(),
                },
            )
            .await,
    )?;
    let body = event(
        &mut state,
        &manager,
        json!({"type":"prepare_reviews","entry[session_id]":session.id.to_string()}),
    )
    .await?;
    proof(
        body.html.contains("evaluation-prepare-confirm"),
        "Prepare reviews is confirmed before creating obligations",
    )?;
    event(&mut state, &manager, json!({"type":"cancel_confirm"})).await?;
    proof(
        !saved(
            training
                .evaluation_reviews(&staff, COMPANY, EvaluationKind::Evaluation)
                .await,
        )?
        .iter()
        .any(|t| t.session_id == session.id),
        "cancelled prepare leaves no reviews",
    )?;
    event(
        &mut state,
        &manager,
        json!({"type":"prepare_reviews","entry[session_id]":session.id.to_string()}),
    )
    .await?;
    event(
        &mut state,
        &manager,
        json!({"type":"confirm_maintenance","entry[session_id]":"9999999"}),
    )
    .await?;
    let task = saved(
        training
            .evaluation_reviews(&manager, COMPANY, EvaluationKind::Effectiveness)
            .await,
    )?
    .into_iter()
    .find(|t| t.session_id == session.id)
    .ok_or(Error::Internal)?;
    event(
        &mut state,
        &manager,
        json!({"type":"open_answer","value":task.review.id.to_string()}),
    )
    .await?;
    let answer = json!({"type":"save_answer","entry[values][outcome]":"4","entry[values][uncertain]":"","entry[reason]":"Observed improvement; one outcome remains unknown"});
    let body = event(&mut state, &manager, answer.clone()).await?;
    proof(
        body.html.contains("evaluation-answer-confirm"),
        "HOD answer waits for irreversible confirmation",
    )?;
    grants
        .set_direct(&admin, MANAGER, EFFECTIVENESS_ANSWER, Some(false))
        .await?;
    let body = event(&mut state, &manager, json!({"type":"confirm_answer"})).await?;
    proof(
        body.html.contains("You cannot answer this company"),
        "mounted review refuses a revoked grant without clearing the page",
    )?;
    grants
        .set_direct(&admin, MANAGER, EFFECTIVENESS_ANSWER, Some(true))
        .await?;
    event(
        &mut state,
        &manager,
        json!({"type":"open_answer","value":task.review.id.to_string()}),
    )
    .await?;
    event(&mut state, &manager, answer).await?;
    let body = event(
        &mut state,
        &manager,
        json!({"type":"confirm_answer","entry[values][outcome]":"9999"}),
    )
    .await?;
    proof(
        body.html.contains("Review answer recorded.")
            && body.html.contains("Unobserved outcome: Unknown"),
        "confirmed answer preserves bounded score and explicit unknown",
    )?;
    let mut mine = mount(&learning, &staff, "my", &query).await?;
    let body = refresh(&mut mine, &staff).await?;
    proof(
        body.html.contains("my-evaluation-tasks"),
        "My learning embeds its independent evaluations FlexTable",
    )?;
    let own = saved(
        training
            .evaluation_reviews(&staff, COMPANY, EvaluationKind::Evaluation)
            .await,
    )?
    .into_iter()
    .find(|t| t.session_id == session.id)
    .ok_or(Error::Internal)?;
    event(
        &mut mine,
        &staff,
        json!({"type":"open_evaluation","value":own.review.id.to_string()}),
    )
    .await?;
    event(&mut mine,&staff,json!({"type":"save_evaluation","evaluation[values][outcome]":"4","evaluation[values][uncertain]":"","evaluation[reason]":"Applied the learning"})).await?;
    let body = event(&mut mine, &staff, json!({"type":"confirm_evaluation"})).await?;
    proof(
        body.html.contains("Evaluation recorded."),
        "own evaluation commits through the self-employment proof",
    )?;
    // Leave another completed session unanswered for the real browser interaction.
    let browser_session = saved(
        training
            .create_session(
                &manager,
                COMPANY,
                &NewSession {
                    event_id: delivery.id,
                    name: "Browser review practice".into(),
                    capacity: 10,
                    time_zone: "UTC".into(),
                    starts_local: "2026-09-02T09:00".into(),
                    ends_local: "2026-09-02T10:00".into(),
                },
            )
            .await,
    )?;
    saved(
        training
            .record(
                &manager,
                COMPANY,
                &NewAttendance {
                    session_id: browser_session.id,
                    employee_id: staff_id,
                    status: AttendanceStatus::Confirmed,
                    import_key: "pt9_browser".into(),
                    reason: "Browser proof attendance".into(),
                },
            )
            .await,
    )?;
    saved(
        training
            .prepare_evaluation_reviews(
                &manager,
                COMPANY,
                browser_session.id,
                jiff::Timestamp::now(),
            )
            .await,
    )?;
    for (action, id) in [
        ("run_reminders", "evaluation-reminders-confirm"),
        ("freeze_summaries", "effectiveness-freeze-confirm"),
    ] {
        let body = event(&mut state, &manager, json!({"type":action})).await?;
        proof(
            body.html.contains(id),
            "maintenance waits for its own confirmation",
        )?;
        event(&mut state, &manager, json!({"type":"confirm_maintenance"})).await?;
    }
    let mut report = mount(&insights, &viewer, "insights", &query).await?;
    let body = refresh(&mut report, &viewer).await?;
    proof(
        body.html.contains("insights-period")
            && !body.html.contains("insights-from")
            && body.html.contains("Record-view access required"),
        "aggregate-only insights offers frozen periods and withholds drill",
    )?;
    let body = event(
        &mut report,
        &viewer,
        json!({"type":"filter","filters[perPage]":"10000"}),
    )
    .await?;
    proof(
        body.query.get("perPage").is_some_and(|v| v == "25"),
        "untrusted page size cannot exceed 300",
    )?;
    let mut drill_query = query.clone();
    drill_query.extend([
        ("from".into(), "2026-09-01".into()),
        ("until".into(), "2026-09-30".into()),
        ("course_id".into(), course.id.to_string()),
    ]);
    let mut drill = mount(&insights, &manager, "insights", &drill_query).await?;
    let body = refresh(&mut drill, &manager).await?;
    proof(
        body.html.contains("insight-drill") && body.html.contains("Completed review practice"),
        "authorized insights drills through bounded attendance in the display timezone",
    )?;
    grants
        .set_direct(&admin, MANAGER, RECORDS_VIEW, Some(false))
        .await?;
    let body = refresh(&mut drill, &manager).await?;
    proof(
        !body.html.contains("Completed review practice"),
        "revoked record access removes drill rows on refresh",
    )?;
    grants
        .set_direct(&admin, MANAGER, RECORDS_VIEW, Some(true))
        .await?;
    println!(
        "PT9 browser records ready: session {}, course {}",
        browser_session.id, course.id
    );
    Ok(())
}

// Keep this page proof independent of the date-sensitive Skills walkthrough.
// Only the Platform fixture is installed; every People row goes through its owner.
async fn setup(services: &Services, admin: &Scope, manager: &Scope) -> Result<()> {
    use bilibi_affiliation::{AffiliationInput, AffiliationService, EffectiveDates};
    use bilibi_party_ids::OrganisationId;
    use bilibi_people_reference_data::{NewEntry, ReferenceData};
    use bilibi_people_workforce::NewEmployment;
    use bilibi_person::{PersonInput, PersonKind, PersonService};
    use bilibi_user::UserService;
    let grants = services.get::<Grants>("base/authz.grants")?;
    grants.reconcile().await?;
    let db = services.get::<bilibi_database::Db>("base/database.pool")?;
    let client = db.get().await?;
    let role = crate::tenant_owner_role(&**client, admin).await?;
    grants.assign_role(admin, MANAGER, role, true).await?;
    for cap in CAPABILITIES {
        grants.set_direct(admin, MANAGER, cap, Some(true)).await?;
    }
    services
        .get::<ReferenceData>(bilibi_people_reference_data::SERVICE)?
        .create_entry(
            admin,
            COMPANY,
            &NewEntry::new("employment_type", "pt9_type", "Review employment"),
        )
        .await
        .map_err(|_| Error::Internal)?;
    let people = services.get::<Workforce>(bilibi_people_workforce::SERVICE)?;
    let manager_employee = people
        .create_employment(
            admin,
            COMPANY,
            &NewEmployment::new(1, 1, "PT9-001", "pt9_type", "active"),
        )
        .await
        .map_err(|_| Error::Internal)?;
    let person = services
        .get::<PersonService>(bilibi_person::SERVICE)?
        .create(
            admin,
            &PersonInput {
                name: "Sam Staff".into(),
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
            admin,
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
    let users = services.get::<UserService>("core/user.accounts")?;
    users
        .link_person(admin, STAFF, person.id)
        .await
        .map_err(|_| Error::Internal)?;
    users
        .link_affiliation(admin, STAFF, affiliation.id)
        .await
        .map_err(|_| Error::Internal)?;
    people
        .create_employment(
            admin,
            COMPANY,
            &NewEmployment {
                supervisor_employment_id: Some(manager_employee.id),
                ..NewEmployment::new(
                    person.id.get(),
                    affiliation.id.get(),
                    "PT9-002",
                    "pt9_type",
                    "active",
                )
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    let training = services.get::<Training>(SERVICE)?;
    saved(
        training
            .put_evaluation_settings(
                manager,
                COMPANY,
                &EvaluationSettings {
                    evaluation_days: Some(0),
                    checkpoints: Some(vec![1, 7]),
                    reminder_days: Some(0),
                    minimum_cohort: Some(2),
                    report_months: Some(3),
                    report_grace_days: Some(2),
                },
            )
            .await,
    )?;
    let criteria = vec![
        EvaluationCriterion {
            code: "outcome".into(),
            label: "Observed outcome".into(),
            minimum: 0,
            maximum: 5,
        },
        EvaluationCriterion {
            code: "uncertain".into(),
            label: "Unobserved outcome".into(),
            minimum: 0,
            maximum: 5,
        },
    ];
    saved(
        training
            .publish_evaluation_policy(
                manager,
                COMPANY,
                &NewEvaluationPolicy {
                    effective_from: "2026-01-01".into(),
                    effective_to: "2026-12-31".into(),
                    criteria: criteria.clone(),
                    effectiveness_criteria: criteria,
                    reason: "Review page proof criteria".into(),
                },
            )
            .await,
    )?;
    Ok(())
}
