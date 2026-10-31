//! PK5 browser records and executable page proof on the disposable walkthrough DB.
//! Run after the default People walkthrough; never use with production data.
use bilibi_authz::grants::Grants;
use bilibi_database::{Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_skills::*;
use bilibi_people_workforce::Workforce;
use bilibi_tenancy::{Authentication, Scope};
use bilibi_ui::page::{LiveInput, LiveRequest, LiveState, PageRequest, PageService};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc};

fn must<T>(result: bilibi_people_skills::Result<T>) -> Result<T> {
    result.map_err(|e| {
        eprintln!("PK5 unexpected: {e:?}");
        Error::Internal
    })
}
fn check(ok: bool, message: &str) -> Result<()> {
    if !ok {
        eprintln!("PK5 FAILED: {message}");
        return Err(Error::Internal);
    }
    println!("PK5 proved: {message}");
    Ok(())
}
async fn mount(
    services: &Services,
    scope: &Scope,
    operation: &'static str,
) -> Result<Box<dyn LiveState<Scope, Error>>> {
    let pages = services.get::<PageService<Scope, Error>>("people/skills.assessment-pages")?;
    let path = HashMap::new();
    let query = HashMap::from([("company_id".into(), "1".into())]);
    let mut page = pages
        .0
        .mount(PageRequest {
            operation,
            scope: Some(scope),
            path: &path,
            query: &query,
            body: &[],
            framed: false,
        })
        .await?
        .ok_or(Error::Internal)?;
    page.update(LiveRequest {
        scope,
        credential: "pk5",
        input: LiveInput::Refresh,
    })
    .await?;
    Ok(page)
}
async fn event(
    page: &mut Box<dyn LiveState<Scope, Error>>,
    scope: &Scope,
    payload: Value,
) -> Result<String> {
    Ok(page
        .update(LiveRequest {
            scope,
            credential: "pk5",
            input: LiveInput::Event(&payload),
        })
        .await?
        .html)
}
#[tokio::main]
async fn main() -> Result<()> {
    let url = std::env::var("DATABASE_URL").map_err(|_| Error::Invalid)?;
    let mode = std::env::args().nth(1).unwrap_or_else(|| "prove".into());
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = bilibi_database::pool(&url)?;
    let mut services = Services::default();
    services.provide("bootstrap.database", Arc::new(db.clone()))?;
    services.provide("bootstrap.composition", Arc::new(bilibi_web::composition()))?;
    services.install(&bilibi_module_registry::discover()?)?;
    services
        .get::<bilibi_base::step_up::StepUp>(bilibi_base::step_up::SERVICE)?
        .set_enforced(false);
    let client = db.get().await?;
    let auth = services.get::<Authentication>("base/tenancy.authentication")?;
    let admin = auth.authenticate(&**client, 1, 4, "pk5".into()).await?;
    let manager = auth.authenticate(&**client, 1, 1, "pk5".into()).await?;
    let viewer = auth.authenticate(&**client, 1, 2, "pk5".into()).await?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    let users = services.get::<bilibi_user::UserService>("core/user.accounts")?;
    let workforce = services.get::<Workforce>(bilibi_people_workforce::SERVICE)?;
    let skills = services.get::<Skills>(bilibi_people_skills::SERVICE)?;
    let employee: i64 = client.query_one("SELECT id FROM people_workforce_employments WHERE tenant_id=1 AND company_id=1 AND employee_number='E-002'", &[]).await?.get(0);
    let employment = workforce
        .employment(&admin, 1, employee)
        .await
        .map_err(|_| Error::Internal)?
        .require_current()
        .map_err(|_| Error::Internal)?;
    if mode == "revoke-review" || mode == "restore-review" {
        grants
            .set_direct(
                &admin,
                1,
                ASSESSMENTS_REVIEW,
                Some(mode == "restore-review"),
            )
            .await?;
        return Ok(());
    }
    if mode == "unlink-staff" {
        users.unlink(&admin, 3).await.map_err(|_| Error::Internal)?;
        return Ok(());
    }
    bilibi_people_walkthrough::give_staff_desk(
        &users,
        &admin,
        3,
        bilibi_party_ids::PersonId::new(employment.person_id).map_err(|_| Error::Invalid)?,
        bilibi_party_ids::OrganisationId::new(1).map_err(|_| Error::Invalid)?,
    )
    .await
    .map_err(|_| Error::Internal)?;
    if mode == "relink-staff" {
        return Ok(());
    }
    for cap in CAPABILITIES {
        grants.set_direct(&admin, 1, cap, Some(true)).await?;
    }
    for cap in CAPABILITIES {
        grants
            .set_direct(&admin, 2, cap, Some(cap == ASSESSMENTS_VIEW))
            .await?;
    }
    grants
        .set_direct(&admin, 2, bilibi_company::TENANT_WIDE, Some(true))
        .await?;
    grants.set_direct(&admin, 3, SELF_VIEW, Some(true)).await?;
    let staff = auth.authenticate(&**client, 1, 3, "pk5".into()).await?;
    let skill = must(skills.list_skills(&admin, 1).await)?
        .into_iter()
        .find(|s| s.code == "welding")
        .ok_or(Error::Internal)?;
    let input = AssessmentInput {
        employee_id: employee,
        skill_id: skill.id,
        assessed_level: 0,
        evidence: "PK5 observed work, with retained inspection evidence.".into(),
        request_key: "pk5-page-review".into(),
        ..AssessmentInput::default()
    };
    if mode == "browser-records" {
        let coordinator: i64 = client.query_one("SELECT id FROM people_workforce_employments WHERE tenant_id=1 AND company_id=1 AND employee_number='E-001'", &[]).await?.get(0);
        let kind = must(
            skills
                .create_action_type(
                    &admin,
                    1,
                    &NewActionType {
                        code: "pk5-observation".into(),
                        name: "Observed practice".into(),
                        requires_provider: false,
                    },
                )
                .await,
        )?;
        let action = must(
            skills
                .propose_action(
                    &manager,
                    1,
                    &ActionInput {
                        source: ActionSource::Manual {
                            employee_id: employee,
                            skill_id: skill.id,
                            starting_level: 0,
                            target_level: 1,
                            criticality: "essential".into(),
                            mandatory: false,
                            reason: "Observed practice needs follow-up.".into(),
                        },
                        plan: ActionPlan {
                            action_type_id: kind.id,
                            owner_employee_id: employee,
                            coordinator_employee_id: coordinator,
                            objective: "Retain the practice and observation record.".into(),
                            intervention: "Practice with feedback.".into(),
                            expected_evidence: "Recorded observation.".into(),
                            due_on: Some(
                                jiff::Timestamp::now()
                                    .to_zoned(jiff::tz::TimeZone::UTC)
                                    .date(),
                            ),
                            ..ActionPlan::default()
                        },
                        request_key: "pk5-browser-action".into(),
                    },
                )
                .await,
        )?;
        must(skills.approve_action(&admin, 1, action.id).await)?;
        must(
            skills
                .request_reassessment(
                    &manager,
                    1,
                    employee,
                    skill.id,
                    "Observe the next practice session.",
                )
                .await,
        )?;
        println!("PK5 My skills action and reassessment browser records ready.");
        return Ok(());
    }
    let a = must(skills.submit_assessment(&admin, 1, &input).await)?;
    let mut page = mount(&services, &manager, "assessments").await?;
    let html = event(
        &mut page,
        &manager,
        json!({"type":"review_assessment", "id":a.id.to_string()}),
    )
    .await?;
    check(
        html.contains("id=\"skills-confirm\""),
        "Verify waits for a confirmation",
    )?;
    event(&mut page, &manager, json!({"type":"dismiss_confirm"})).await?;
    check(
        must(
            skills
                .list_assessments(&admin, 1, &AssessmentFilters::default())
                .await,
        )?
        .iter()
        .any(|v| v.assessment.id == a.id && v.assessment.status == "pending_review"),
        "Cancel writes nothing",
    )?;
    event(
        &mut page,
        &manager,
        json!({"type":"review_assessment", "id":a.id.to_string()}),
    )
    .await?;
    grants
        .set_direct(&admin, 1, ASSESSMENTS_REVIEW, Some(false))
        .await?;
    let html = event(&mut page, &manager, json!({"type":"confirm"})).await?;
    check(
        html.contains("You cannot do that for this company."),
        "revoked permission refuses an already-open confirmation",
    )?;
    grants
        .set_direct(&admin, 1, ASSESSMENTS_REVIEW, Some(true))
        .await?;
    event(
        &mut page,
        &manager,
        json!({"type":"review_assessment", "id":a.id.to_string()}),
    )
    .await?;
    let html = event(
        &mut page,
        &manager,
        json!({"type":"confirm", "id":"999999"}),
    )
    .await?;
    check(
        html.contains("Assessment verified."),
        "confirmation ignores a replacement target",
    )?;
    event(
        &mut page,
        &manager,
        json!({"type":"finalize_assessment", "id":a.id.to_string()}),
    )
    .await?;
    check(
        event(&mut page, &manager, json!({"type":"confirm"}))
            .await?
            .contains("Assessment finalized; the current score is updated."),
        "finalization updates the current score",
    )?;
    let mut own = mount(&services, &staff, "my").await?;
    check(
        event(
            &mut own,
            &staff,
            json!({"type":"scores_grid", "op":"zoom_preset", "preset":"compact"}),
        )
        .await?
        .contains(&skill.name),
        "My skills shows finalized scores in compact rows",
    )?;
    users.unlink(&admin, 3).await.map_err(|_| Error::Internal)?;
    let html = event(
        &mut own,
        &staff,
        json!({"type":"scores_grid", "op":"zoom_preset", "preset":"normal"}),
    )
    .await?;
    check(
        html.contains("your account needs a link to a current employee.")
            && !html.contains("id=\"my-skills-section\""),
        "unlinking the account removes scores on the next table event",
    )?;
    bilibi_people_walkthrough::give_staff_desk(
        &users,
        &admin,
        3,
        bilibi_party_ids::PersonId::new(employment.person_id).map_err(|_| Error::Invalid)?,
        bilibi_party_ids::OrganisationId::new(1).map_err(|_| Error::Invalid)?,
    )
    .await
    .map_err(|_| Error::Internal)?;
    let mut restricted = mount(&services, &viewer, "assessments").await?;
    let html = event(
        &mut restricted,
        &viewer,
        json!({"type":"create_assessment"}),
    )
    .await?;
    check(
        html.contains("You do not assess employees for this company.")
            && !html.contains("id=\"assessment-form\""),
        "restricted reader sees the withheld-control sentence and cannot submit",
    )?;
    let html = event(&mut page, &manager, json!({"type":"create_assessment", "assessment[employee_id]":employee.to_string(), "assessment[skill_id]":skill.id.to_string(), "assessment[assessed_level]":"1", "assessment[evidence]":"Observation evidence. ".repeat(80)})).await?;
    check(
        html.contains("Assessment submitted for review."),
        "evidence longer than one KiB reaches owner validation",
    )?;
    let submitted = must(
        skills
            .list_assessments(&admin, 1, &AssessmentFilters::default())
            .await,
    )?
    .into_iter()
    .find(|r| {
        r.assessment.assessor_user_id == 1
            && r.assessment.evidence.starts_with("Observation evidence.")
    })
    .ok_or(Error::Internal)?;
    let mut admin_page = mount(&services, &admin, "assessments").await?;
    event(
        &mut admin_page,
        &admin,
        json!({"type":"start_return", "id":submitted.assessment.id.to_string()}),
    )
    .await?;
    event(
        &mut admin_page,
        &admin,
        json!({"type":"return_assessment", "note":"Include the observation date."}),
    )
    .await?;
    check(
        event(&mut admin_page, &admin, json!({"type":"confirm"}))
            .await?
            .contains("Assessment returned to the assessor."),
        "Return retains its reason through confirmation",
    )?;
    event(
        &mut page,
        &manager,
        json!({"type":"start_correction", "id":submitted.assessment.id.to_string()}),
    )
    .await?;
    let html = event(&mut page, &manager, json!({"type":"create_assessment", "assessment[employee_id]":employee.to_string(), "assessment[skill_id]":skill.id.to_string(), "assessment[assessed_level]":"1", "assessment[evidence]":"Corrected dated observation."})).await?;
    check(
        html.contains("Assessment submitted for review."),
        "returned work is corrected by a new submission",
    )?;
    check(
        must(
            skills
                .list_assessments(&admin, 1, &AssessmentFilters::default())
                .await,
        )?
        .iter()
        .any(|r| r.assessment.supersedes_assessment_id == Some(submitted.assessment.id)),
        "correction retains the returned assessment as its predecessor",
    )?;
    let html = event(
        &mut page,
        &manager,
        json!({"type":"register_grid", "op":"remove", "spec":"status"}),
    )
    .await?;
    check(
        !html.contains("data-column=\"status\""),
        "register column settings remain usable",
    )?;
    event(
        &mut page,
        &manager,
        json!({"type":"register_grid", "op":"suggest", "add":"Status"}),
    )
    .await?;
    let html = event(
        &mut page,
        &manager,
        json!({"type":"register_grid", "op":"add_typed", "add":"Status"}),
    )
    .await?;
    check(html.contains("Status"), "removed columns can be added back")?;
    // Leave fresh independent work for the browser pass.
    must(
        skills
            .submit_assessment(
                &admin,
                1,
                &AssessmentInput {
                    request_key: "pk5-browser-review".into(),
                    evidence: "PK5 browser review evidence.".into(),
                    ..input.clone()
                },
            )
            .await,
    )?;
    println!("PK5 page proof complete; browser records ready.");
    Ok(())
}
