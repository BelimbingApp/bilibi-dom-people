//! PM1 running-system proof. Creates its own records through owner facades.
use bilibi_affiliation::{AffiliationInput, AffiliationService, EffectiveDates, OrganisationId};
use bilibi_authz::grants::Grants;
use bilibi_database::{pool, Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_organisation::{NewAssignment, NewPosition, NewVersion, Organisation};
use bilibi_people_performance::{self as pm, *};
use bilibi_people_reference_data::{NewEntry, ReferenceData};
use bilibi_people_skills::{self as sk, Skills};
use bilibi_people_workforce::{NewEmployment, Workforce};
use bilibi_person::{PersonInput, PersonKind, PersonService};
use bilibi_tenancy::Authentication;
use bilibi_user::UserService;
use jiff::{civil::date, Timestamp};
use rust_decimal::Decimal;
use std::{fmt::Debug, sync::Arc};

fn saved<T, E: std::fmt::Display>(value: std::result::Result<T, E>) -> Result<T> {
    value.map_err(|e| {
        eprintln!("PM1 unexpected refusal: {e}");
        Error::Internal
    })
}
fn prove(value: bool, step: &str) -> Result<()> {
    println!("PM1 {}: {step}", if value { "proved" } else { "FAILED" });
    if value {
        Ok(())
    } else {
        Err(Error::Internal)
    }
}
fn refused<T: Debug>(
    value: pm::Result<T>,
    expected: fn(&PerformanceError) -> bool,
    step: &str,
) -> Result<()> {
    match value {
        Err(error) if expected(&error) => {
            println!("PM1 refused {step}: {error}");
            Ok(())
        }
        other => {
            eprintln!("PM1 FAILED {step}: {other:?}");
            Err(Error::Internal)
        }
    }
}
async fn db_refused(client: &tokio_postgres::Client, sql: &str, id: i64, step: &str) -> Result<()> {
    match client.execute(sql, &[&id]).await {
        Err(error)
            if error
                .as_db_error()
                .is_some_and(|e| e.code().code() == "23514") =>
        {
            println!("PM1 PostgreSQL refused {step}");
            Ok(())
        }
        other => {
            eprintln!("PM1 FAILED PostgreSQL guard {step}: {other:?}");
            Err(Error::Internal)
        }
    }
}

pub async fn run(url: &str) -> Result<()> {
    run_with_pages(url, false).await
}
pub(super) async fn run_with_pages(url: &str, pages: bool) -> Result<()> {
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
    if std::env::var("BILIBI_ENV").as_deref() == Ok("development") {
        services
            .get::<bilibi_base::step_up::StepUp>(bilibi_base::step_up::SERVICE)?
            .set_enforced(false);
    }
    let client = db.get().await?;
    let auth = services.get::<Authentication>("base/tenancy.authentication")?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    grants.reconcile().await?;
    let admin = auth
        .authenticate(&**client, 1, 4, "walkthrough".into())
        .await?;
    let manager = auth
        .authenticate(&**client, 1, 1, "walkthrough".into())
        .await?;
    let viewer = auth
        .authenticate(&**client, 1, 2, "walkthrough".into())
        .await?;
    let people = services.get::<Workforce>(bilibi_people_workforce::SERVICE)?;
    let performance = services.get::<Performance>(pm::SERVICE)?;
    let org = services.get::<Organisation>(bilibi_people_organisation::SERVICE)?;
    let skills = services.get::<Skills>(sk::SERVICE)?;
    for capability in pm::CAPABILITIES {
        grants.set_direct(&admin, 1, capability, Some(true)).await?;
        grants.set_direct(&admin, 3, capability, Some(true)).await?;
    }
    let references = services.get::<ReferenceData>(bilibi_people_reference_data::SERVICE)?;
    saved(
        references
            .create_entry(
                &admin,
                1,
                &NewEntry::new("employment_type", "pm1_type", "PM1 employment type"),
            )
            .await,
    )?;
    let manager_employee = saved(
        people
            .create_employment(
                &admin,
                1,
                &NewEmployment::new(1, 1, "PM1-001", "pm1_type", "active"),
            )
            .await,
    )?;
    let persons = services.get::<PersonService>(bilibi_person::SERVICE)?;
    let person = saved(
        persons
            .create(
                &admin,
                &PersonInput {
                    name: "PM1 Employee".into(),
                    kind: PersonKind::Human,
                    email: None,
                    phone: None,
                },
            )
            .await,
    )?;
    let affiliations = services.get::<AffiliationService>(bilibi_affiliation::SERVICE)?;
    let affiliation = affiliations
        .create(
            &admin,
            &AffiliationInput {
                person_id: person.id,
                organisation_id: OrganisationId::new(1).map_err(|_| Error::Invalid)?,
                title: None,
                work_email: None,
                work_phone: None,
                dates: EffectiveDates::new(None, None).map_err(|_| Error::Invalid)?,
            },
        )
        .await?;
    let users = services.get::<UserService>("core/user.accounts")?;
    saved(users.link_affiliation(&admin, 3, affiliation.id).await)?;
    let employee = saved(
        people
            .create_employment(
                &admin,
                1,
                &NewEmployment {
                    supervisor_employment_id: Some(manager_employee.id),
                    ..NewEmployment::new(
                        person.id.get(),
                        affiliation.id.get(),
                        "PM1-002",
                        "pm1_type",
                        "active",
                    )
                },
            )
            .await,
    )?;
    let staff = auth
        .authenticate(&**client, 1, 3, "walkthrough".into())
        .await?;
    let position = saved(
        org.create_position(&admin, 1, &NewPosition::new("PM1-POSITION"))
            .await,
    )?;
    saved(
        org.record_version(
            &admin,
            1,
            position.id,
            &NewVersion {
                version: 1,
                title: "PM1 position".into(),
                effective_from: date(2020, 1, 1),
                effective_to: None,
            },
        )
        .await,
    )?;
    saved(
        org.assign(
            &admin,
            1,
            position.id,
            &NewAssignment {
                employee_id: employee.id,
                kind: "substantive".into(),
                effective_from: date(2020, 1, 1),
                effective_to: None,
            },
        )
        .await,
    )?;
    let category = saved(
        skills
            .create_category(
                &admin,
                1,
                &sk::NewCategory {
                    code: "pm1_category".into(),
                    name: "PM1 category".into(),
                    description: None,
                },
            )
            .await,
    )?;
    let skill = saved(
        skills
            .create_skill(
                &admin,
                1,
                &sk::NewSkill {
                    category_id: category.id,
                    code: "pm1_skill".into(),
                    name: "PM1 skill".into(),
                    definition: "Fixture skill definition".into(),
                    ..Default::default()
                },
            )
            .await,
    )?;
    let scale = saved(
        skills
            .create_scale(
                &admin,
                1,
                &sk::NewScale {
                    code: "pm1_scale".into(),
                    name: "PM1 scale".into(),
                },
            )
            .await,
    )?;
    for level in 0..=1 {
        saved(
            skills
                .put_scale_level(
                    &admin,
                    1,
                    scale.id,
                    &sk::Level {
                        level,
                        name: format!("Level {level}"),
                        anchor: format!("Observable level {level}"),
                        authority: "Fixture authority".into(),
                    },
                )
                .await,
        )?;
    }
    saved(skills.publish_scale(&admin, 1, scale.id).await)?;
    let profile = saved(
        skills
            .create_profile(
                &admin,
                1,
                &sk::NewProfile {
                    code: "pm1_profile".into(),
                    name: "PM1 profile".into(),
                    scale_id: scale.id,
                },
            )
            .await,
    )?;
    saved(
        skills
            .put_item(
                &admin,
                1,
                profile.id,
                &sk::ItemInput {
                    skill_id: skill.id,
                    required_level: 1,
                    criticality: "essential".into(),
                    weight_percent: Decimal::from(100),
                    mandatory: true,
                    evidence_standard: None,
                },
            )
            .await,
    )?;
    saved(
        skills
            .add_selector(&admin, 1, profile.id, sk::Target::Company)
            .await,
    )?;
    saved(
        skills
            .publish_profile(&admin, 1, profile.id, date(2020, 1, 1))
            .await,
    )?;

    let description_input = DescriptionInput {
        code: "pm1_description".into(),
        version: 1,
        position_id: position.id,
        position_version: 1,
        effective_from: "2020-01-01".into(),
        effective_to: None,
        purpose: "PM1 purpose".into(),
        responsibilities: "PM1 responsibilities".into(),
        duties: "PM1 duties".into(),
        authority: "PM1 authority".into(),
        qualifications: "PM1 qualifications".into(),
        competency_links: vec![ProfileLink {
            id: profile.id,
            version: 1,
        }],
    };
    refused(
        performance
            .draft_description(&viewer, 1, &description_input)
            .await,
        |e| matches!(e, PerformanceError::Unauthorized),
        "ungranted description writer",
    )?;
    let description = saved(
        performance
            .draft_description(&manager, 1, &description_input)
            .await,
    )?;
    saved(
        performance
            .publish_description(&manager, 1, description.id)
            .await,
    )?;
    let overlap = saved(
        performance
            .draft_description(
                &manager,
                1,
                &DescriptionInput {
                    code: "pm1_overlap".into(),
                    ..description_input.clone()
                },
            )
            .await,
    )?;
    refused(
        performance
            .publish_description(&manager, 1, overlap.id)
            .await,
        |e| matches!(e, PerformanceError::OverlappingDescription),
        "overlapping description",
    )?;
    refused(
        performance
            .draft_description(
                &manager,
                1,
                &DescriptionInput {
                    position_version: 99,
                    ..description_input.clone()
                },
            )
            .await,
        |e| matches!(e, PerformanceError::PositionVersionUnavailable),
        "wrong pinned position version",
    )?;
    refused(
        performance
            .draft_description(
                &manager,
                1,
                &DescriptionInput {
                    competency_links: vec![ProfileLink {
                        id: profile.id,
                        version: 99,
                    }],
                    ..description_input
                },
            )
            .await,
        |e| matches!(e, PerformanceError::ProfileUnavailable),
        "wrong pinned profile version",
    )?;
    let definition_input = DefinitionInput {
        code: "pm1_kpi".into(),
        version: 1,
        name: "PM1 measure".into(),
        purpose: "PM1 purpose".into(),
        unit: "Fixture unit".into(),
        measure: "Fixture measurement".into(),
        source_reference: "fixture:source".into(),
        calculation_version: "1".into(),
        direction: "higher".into(),
        rubric: Some("   ".into()),
        precision: 2,
        interpretation: "Fixture interpretation".into(),
    };
    let definition = saved(performance.define_kpi(&manager, 1, &definition_input).await)?;
    prove(
        definition.rubric.is_none(),
        "blank optional rubric stays absent",
    )?;
    refused(
        performance
            .define_kpi(
                &manager,
                1,
                &DefinitionInput {
                    direction: "rubric".into(),
                    ..definition_input.clone()
                },
            )
            .await,
        |e| matches!(e, PerformanceError::RubricRequired),
        "rubric without text",
    )?;
    let target_input = TargetInput {
        definition_id: definition.id,
        employee_id: employee.id,
        target: "Fixture target".into(),
        period_start: "2020-01-01".into(),
        period_end: "2020-01-31".into(),
        effective_from: "2020-01-01".into(),
        confidential: false,
    };
    refused(
        performance
            .propose_target(
                &manager,
                1,
                &TargetInput {
                    employee_id: manager_employee.id,
                    ..target_input.clone()
                },
            )
            .await,
        |e| matches!(e, PerformanceError::OutOfReach),
        "self target",
    )?;
    let target = saved(performance.propose_target(&manager, 1, &target_input).await)?;
    refused(
        performance
            .review_target(&manager, 1, target.id, "Self")
            .await,
        |e| matches!(e, PerformanceError::SelfApproval),
        "author target review",
    )?;
    refused(
        performance
            .review_target(&staff, 1, target.id, "Self")
            .await,
        |e| matches!(e, PerformanceError::SelfApproval),
        "employee target review",
    )?;
    refused(
        performance.publish_target(&admin, 1, target.id).await,
        |e| matches!(e, PerformanceError::NotPublishable),
        "unreviewed communication",
    )?;
    saved(
        performance
            .review_target(&admin, 1, target.id, "Independent rationale")
            .await,
    )?;
    saved(performance.publish_target(&admin, 1, target.id).await)?;
    let confidential = saved(
        performance
            .propose_target(
                &manager,
                1,
                &TargetInput {
                    confidential: true,
                    ..target_input.clone()
                },
            )
            .await,
    )?;
    saved(
        performance
            .review_target(&admin, 1, confidential.id, "Confidential rationale")
            .await,
    )?;
    refused(
        performance.publish_target(&admin, 1, confidential.id).await,
        |e| matches!(e, PerformanceError::NotPublishable),
        "confidential communication",
    )?;
    let amended = saved(
        performance
            .amend_target(
                &manager,
                1,
                target.id,
                &TargetAmendment {
                    target: "Amended target".into(),
                    effective_from: "2020-01-15".into(),
                    change_reason: "New evidence".into(),
                },
            )
            .await,
    )?;
    prove(
        amended.version == 2 && amended.status == "proposed" && amended.review_note.is_none(),
        "target amendment starts a new approval cycle",
    )?;
    refused(
        performance
            .amend_target(
                &manager,
                1,
                target.id,
                &TargetAmendment {
                    target: "Duplicate".into(),
                    effective_from: "2020-01-16".into(),
                    change_reason: "Duplicate".into(),
                },
            )
            .await,
        |e| matches!(e, PerformanceError::AlreadyCorrected),
        "second target successor",
    )?;
    let observation = saved(
        performance
            .record_observation(
                &manager,
                1,
                &ObservationInput {
                    employee_id: employee.id,
                    window_start: "2020-01-01".into(),
                    window_end: "2020-01-31".into(),
                    evidence: "Original evidence".into(),
                    source_reference: "fixture:observation".into(),
                    source_version: "1".into(),
                },
            )
            .await,
    )?;
    let correction = saved(
        performance
            .correct_observation(
                &manager,
                1,
                observation.id,
                &ObservationCorrection {
                    evidence: "Corrected evidence".into(),
                    source_version: "2".into(),
                    change_reason: "Corrected source".into(),
                },
            )
            .await,
    )?;
    prove(
        correction.supersedes_id == Some(observation.id),
        "observation correction retains original",
    )?;
    let review_input = ReviewInput {
        employee_id: employee.id,
        description_id: description.id,
        period_start: "2020-01-01".into(),
        period_end: "2020-01-31".into(),
        cutoff_at: Timestamp::now().to_string(),
        outcome: "Fixture outcome".into(),
        rationale: "Original rationale".into(),
        observation_ids: vec![observation.id, observation.id],
        target_ids: vec![target.id],
    };
    refused(
        performance
            .draft_review(
                &manager,
                1,
                &ReviewInput {
                    observation_ids: vec![],
                    ..review_input.clone()
                },
            )
            .await,
        |e| matches!(e, PerformanceError::EvidenceRequired),
        "review without pins",
    )?;
    let review = saved(performance.draft_review(&manager, 1, &review_input).await)?;
    prove(
        saved(performance.planning_records(&staff, 1).await)?
            .drafts
            .is_empty(),
        "subject cannot see own release queue",
    )?;
    prove(
        saved(performance.planning_records(&staff, 1).await)?
            .targets
            .is_empty(),
        "subject cannot see own confidential/planning targets",
    )?;
    refused(
        performance.review(&staff, 1, review.id).await,
        |e| matches!(e, PerformanceError::NotFound),
        "subject inspecting draft",
    )?;
    prove(
        saved(
            performance
                .my_records(&staff, 1, PageOptions::default())
                .await,
        )?
        .reviews
        .total
            == 0,
        "draft absent from My performance",
    )?;
    refused(
        performance.release_review(&manager, 1, review.id).await,
        |e| matches!(e, PerformanceError::SelfApproval),
        "author review release",
    )?;
    refused(
        performance.release_review(&staff, 1, review.id).await,
        |e| matches!(e, PerformanceError::SelfApproval),
        "subject review release",
    )?;
    let future = saved(
        performance
            .draft_review(
                &manager,
                1,
                &ReviewInput {
                    cutoff_at: "2100-01-01T00:00:00Z".into(),
                    ..review_input.clone()
                },
            )
            .await,
    )?;
    refused(
        performance.release_review(&admin, 1, future.id).await,
        |e| matches!(e, PerformanceError::FutureCutoff),
        "future UTC cutoff",
    )?;
    let released = saved(performance.release_review(&admin, 1, review.id).await)?;
    prove(released.status == "released", "independent release")?;
    saved(
        performance
            .respond(&staff, 1, review.id, "Employee response")
            .await,
    )?;
    refused(
        performance
            .respond(&staff, 1, review.id, "Second response")
            .await,
        |e| matches!(e, PerformanceError::InvariantRefused),
        "second employee response",
    )?;
    let corrected_review = saved(
        performance
            .correct_review(
                &manager,
                1,
                review.id,
                &ReviewCorrection {
                    description_id: description.id,
                    cutoff_at: Some(Timestamp::now().to_string()),
                    outcome: "Corrected outcome".into(),
                    rationale: "Corrected rationale".into(),
                    change_reason: "Late evidence".into(),
                    observation_ids: vec![correction.id],
                    target_ids: vec![target.id],
                },
            )
            .await,
    )?;
    saved(
        performance
            .release_review(&admin, 1, corrected_review.id)
            .await,
    )?;
    let retained_cutoff = saved(
        performance
            .correct_review(
                &manager,
                1,
                corrected_review.id,
                &ReviewCorrection {
                    description_id: description.id,
                    cutoff_at: None,
                    outcome: "Further correction".into(),
                    rationale: "Same cutoff".into(),
                    change_reason: "Clarified wording".into(),
                    observation_ids: vec![correction.id],
                    target_ids: vec![target.id],
                },
            )
            .await,
    )?;
    prove(
        retained_cutoff.cutoff_at == corrected_review.cutoff_at && retained_cutoff.version == 3,
        "correction without a cutoff retains the preceding cutoff",
    )?;
    let authored = saved(
        performance
            .reviews(
                &manager,
                1,
                PageOptions {
                    page: i64::MAX,
                    page_size: 10,
                },
            )
            .await,
    )?;
    prove(
        authored.total == 4 && authored.page == 1,
        "authored review history and page clamping",
    )?;
    refused(
        performance
            .reviews(
                &manager,
                1,
                PageOptions {
                    page: 0,
                    page_size: 10,
                },
            )
            .await,
        |e| matches!(e, PerformanceError::InvalidOptions),
        "invalid review pagination",
    )?;
    refused(
        performance.review(&admin, 2, review.id).await,
        |e| matches!(e, PerformanceError::NotFound),
        "cross-company review identity",
    )?;
    refused(
        performance.review(&viewer, 1, review.id).await,
        |e| matches!(e, PerformanceError::Unauthorized),
        "ungranted review reader",
    )?;
    let original = saved(performance.review(&manager, 1, review.id).await)?;
    prove(
        original.review.rationale == "Original rationale"
            && original.observations.len() == 1
            && original.observations[0].evidence == "Original evidence"
            && original.responses.len() == 1,
        "original pinned evidence, rationale and response remain unchanged",
    )?;
    let mine = saved(
        performance
            .my_records(
                &staff,
                1,
                PageOptions {
                    page: i64::MAX,
                    page_size: 10,
                },
            )
            .await,
    )?;
    prove(
        mine.reviews.total == 2
            && mine.reviews.page == 1
            && mine.targets.iter().all(|t| !t.confidential),
        "released history, safe pagination and communicated targets",
    )?;
    let choices = saved(performance.planning_choices(&manager, 1).await)?;
    prove(
        choices.employees.iter().any(|e| e.id == employee.id)
            && choices.descriptions.iter().all(|d| d.status == "published"),
        "published-only selectors and direct-report choices",
    )?;
    let impersonated = auth
        .authenticate_session(&**client, 1, 1, Some(4), "walkthrough".into())
        .await?;
    refused(
        performance
            .define_kpi(&impersonated, 1, &definition_input)
            .await,
        |e| matches!(e, PerformanceError::ImpersonationRefused),
        "impersonated decision",
    )?;
    grants
        .set_direct(&admin, 1, KPIS_SUBMIT, Some(false))
        .await?;
    refused(
        performance.define_kpi(&manager, 1, &definition_input).await,
        |e| matches!(e, PerformanceError::Unauthorized),
        "revoked grant using existing scope",
    )?;
    grants
        .set_direct(&admin, 1, KPIS_SUBMIT, Some(true))
        .await?;
    saved(users.unlink(&admin, 3).await)?;
    refused(
        performance
            .respond(&staff, 1, corrected_review.id, "Old link")
            .await,
        |e| matches!(e, PerformanceError::Unavailable),
        "old scope after unlink",
    )?;
    saved(users.link_affiliation(&admin, 3, affiliation.id).await)?;
    // Database history guards, independently of the facade.
    db_refused(
        &client,
        "UPDATE people_performance_appraisals SET rationale = 'Changed' WHERE id = $1",
        review.id,
        "released rationale edit",
    )
    .await?;
    db_refused(
        &client,
        "DELETE FROM people_performance_evidence WHERE id = $1",
        observation.id,
        "evidence delete",
    )
    .await?;
    db_refused(
        &client,
        "UPDATE people_performance_kpi_targets SET target = 'Changed' WHERE id = $1",
        target.id,
        "communicated target edit",
    )
    .await?;
    db_refused(
        &client,
        "UPDATE people_performance_responses SET response = 'Changed' WHERE review_id = $1",
        review.id,
        "response edit",
    )
    .await?;
    db_refused(&client, "INSERT INTO people_performance_review_evidence (tenant_id,company_id,actor_user_id,review_id,observation_id) SELECT tenant_id,company_id,actor_user_id,id,$1 FROM people_performance_appraisals WHERE id = (SELECT review_id FROM people_performance_review_evidence WHERE observation_id=$1 LIMIT 1)", correction.id, "pin after release").await?;
    let count: i64 = client.query_one("SELECT count(*) FROM base_audit_mutations WHERE tenant_id=1 AND auditable_type LIKE 'people_performance_%'", &[]).await?.get(0);
    prove(count >= 25, "every performance mutation and pin is audited")?;
    println!("PM1 proof complete: {count} audited changes");
    if pages {
        super::performance_pages::verify(&services).await?;
    }
    Ok(())
}
