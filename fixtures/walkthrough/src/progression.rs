//! PG1 running-system proof. All business rows are created through their owners.
use bilibi_authz::grants::Grants;
use bilibi_database::{pool, Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_performance as pm;
use bilibi_people_progression::{self as pg, *};
use bilibi_people_skills::{self as sk, Skills};
use bilibi_people_workforce::Workforce;
use bilibi_tenancy::Authentication;
use bilibi_user::UserService;
use jiff::{civil::date, Timestamp};
use std::{fmt::Debug, sync::Arc};

fn saved<T, E: std::fmt::Display>(value: std::result::Result<T, E>) -> Result<T> {
    value.map_err(|e| {
        eprintln!("PG1 unexpected refusal: {e}");
        Error::Internal
    })
}
fn prove(value: bool, step: &str) -> Result<()> {
    println!("PG1 {}: {step}", if value { "proved" } else { "FAILED" });
    if value {
        Ok(())
    } else {
        Err(Error::Internal)
    }
}
fn refused<T: Debug>(
    value: pg::Result<T>,
    expected: fn(&ProgressionError) -> bool,
    step: &str,
) -> Result<()> {
    match value {
        Err(error) if expected(&error) => {
            println!("PG1 refused {step}: {error}");
            Ok(())
        }
        other => {
            eprintln!("PG1 FAILED {step}: {other:?}");
            Err(Error::Internal)
        }
    }
}
async fn db_refused(client: &tokio_postgres::Client, sql: &str, id: i64, step: &str) -> Result<()> {
    match client.execute(sql, &[&id]).await {
        Err(e) if e.as_db_error().is_some_and(|e| e.code().code() == "23514") => {
            println!("PG1 PostgreSQL refused {step}");
            Ok(())
        }
        other => {
            eprintln!("PG1 FAILED database guard {step}: {other:?}");
            Err(Error::Internal)
        }
    }
}

pub async fn run(url: &str) -> Result<()> {
    let db = pool(url)?;
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
    let grants = services.get::<Grants>("base/authz.grants")?;
    let admin = auth
        .authenticate(&**client, 1, 4, "walkthrough".into())
        .await?;
    let manager = auth
        .authenticate(&**client, 1, 1, "walkthrough".into())
        .await?;
    let staff = auth
        .authenticate(&**client, 1, 3, "walkthrough".into())
        .await?;
    let viewer = auth
        .authenticate(&**client, 1, 2, "walkthrough".into())
        .await?;
    let pg = services.get::<Progression>(pg::SERVICE)?;
    let skills = services.get::<Skills>(sk::SERVICE)?;
    let people = services.get::<Workforce>(bilibi_people_workforce::SERVICE)?;
    let performance = services.get::<pm::Performance>(pm::SERVICE)?;
    for cap in pg::CAPABILITIES {
        grants.set_direct(&admin, 1, cap, Some(true)).await?;
        grants.set_direct(&admin, 3, cap, Some(true)).await?;
    }
    for cap in [sk::ASSESSMENTS_SUBMIT, sk::ASSESSMENTS_MANAGE] {
        grants.set_direct(&admin, 1, cap, Some(true)).await?;
    }
    for cap in [sk::ASSESSMENTS_REVIEW, sk::ASSESSMENTS_APPROVE] {
        grants.set_direct(&admin, 4, cap, Some(true)).await?;
    }
    grants
        .set_direct(&admin, 3, sk::SELF_VIEW, Some(true))
        .await?;
    let employee = saved(people.self_employee(&staff, 1).await)?;
    let profile = saved(skills.list_profiles(&manager, 1).await)?
        .into_iter()
        .find(|p| p.code == "pm1_profile")
        .ok_or(Error::Internal)?;
    let detail = saved(skills.get_profile(&manager, 1, profile.id).await)?;
    let skill = detail.items[0].skill_id;
    let perf = PerformanceRule {
        periods: vec![Period {
            start: "2020-01-01".into(),
            end: "2020-01-31".into(),
        }],
        accepted_outcomes: vec!["Corrected outcome".into()],
        missing_evidence: MissingEvidence::Unknown,
    };
    let input = DraftInput {
        code: "pg1_combined".into(),
        version: 1,
        name: "PG1 combined policy".into(),
        effective_from: "2020-01-01".into(),
        profile: Some(ProfilePin {
            id: profile.id,
            version: profile.version,
        }),
        performance: Some(perf.clone()),
    };
    refused(
        pg.explain(&staff, 1).await,
        |e| matches!(e, ProgressionError::NoPublishedPolicy),
        "no effective policy",
    )?;
    refused(
        pg.draft(&viewer, 1, &input).await,
        |e| matches!(e, ProgressionError::Unauthorized),
        "ungranted draft",
    )?;
    refused(
        pg.draft(
            &manager,
            1,
            &DraftInput {
                profile: None,
                performance: None,
                ..input.clone()
            },
        )
        .await,
        |e| matches!(e, ProgressionError::InvalidRules),
        "empty criteria",
    )?;
    refused(
        pg.draft(
            &manager,
            1,
            &DraftInput {
                profile: Some(ProfilePin {
                    id: profile.id,
                    version: 99,
                }),
                ..input.clone()
            },
        )
        .await,
        |e| matches!(e, ProgressionError::ProfileUnavailable),
        "wrong pinned profile version",
    )?;
    refused(
        pg.draft(
            &manager,
            1,
            &DraftInput {
                effective_from: "invalid".into(),
                ..input.clone()
            },
        )
        .await,
        |e| matches!(e, ProgressionError::InvalidPolicy),
        "invalid policy date",
    )?;
    refused(
        pg.draft(
            &manager,
            1,
            &DraftInput {
                performance: Some(PerformanceRule {
                    periods: vec![Period {
                        start: "2020-02-01".into(),
                        end: "2020-01-01".into(),
                    }],
                    ..perf.clone()
                }),
                ..input.clone()
            },
        )
        .await,
        |e| matches!(e, ProgressionError::InvalidRules),
        "reversed performance period",
    )?;
    let draft = saved(pg.draft(&manager, 1, &input).await)?;
    prove(
        draft.rules.competence.len() == detail.items.len()
            && draft.rules.competence[0].required_level == detail.items[0].required_level,
        "draft copies pinned requirements",
    )?;
    db_refused(
        &client,
        "UPDATE people_progression_policy_versions SET name='changed' WHERE id=$1",
        draft.id,
        "draft content edit",
    )
    .await?;
    db_refused(
        &client,
        "DELETE FROM people_progression_policy_versions WHERE id=$1",
        draft.id,
        "draft deletion",
    )
    .await?;
    let policy = saved(pg.publish(&manager, 1, draft.id).await)?;
    prove(
        policy.actor_user_id == 1
            && policy.published_by_user_id == Some(1)
            && policy.published_at.is_some(),
        "publication records actual actor and UTC time",
    )?;
    refused(
        pg.publish(&manager, 1, draft.id).await,
        |e| matches!(e, ProgressionError::AlreadyPublished),
        "repeat publication",
    )?;
    refused(
        pg.draft(&manager, 1, &input).await,
        |e| matches!(e, ProgressionError::InvariantRefused),
        "duplicate company code/version",
    )?;
    refused(
        pg.publish(&admin, 2, draft.id).await,
        |e| matches!(e, ProgressionError::NotFound),
        "foreign-company identity",
    )?;
    let before = saved(pg.explain(&staff, 1).await)?;
    prove(
        before.employee_id == employee.employee_id
            && before.explanations[0].status == Status::Unknown,
        "missing competence is unknown",
    )?;
    prove(
        matches!(
            &before.explanations[0].rules[1],
            Criterion::Performance {
                status: Status::Met,
                review_version: Some(2),
                supersedes_id: Some(_),
                ..
            }
        ),
        "released correction counts; later draft correction is excluded",
    )?;
    let today = Timestamp::now().to_zoned(jiff::tz::TimeZone::UTC).date();
    let assessment = sk::AssessmentInput {
        employee_id: employee.employee_id,
        skill_id: skill,
        assessed_level: 1,
        evidence: "PG1 observed evidence".into(),
        assessed_on: Some(today),
        request_key: "pg1-current".into(),
        ..Default::default()
    };
    let expired = saved(
        skills
            .submit_assessment(
                &manager,
                1,
                &sk::AssessmentInput {
                    assessed_on: Some(date(2020, 1, 1)),
                    valid_until: Some(date(2020, 1, 2)),
                    request_key: "pg1-expired".into(),
                    ..assessment.clone()
                },
            )
            .await,
    )?;
    saved(
        skills
            .review_assessment(&admin, 1, expired.id, sk::ReviewDecision::Verify, None)
            .await,
    )?;
    saved(skills.finalize_assessment(&admin, 1, expired.id).await)?;
    prove(
        saved(pg.explain(&staff, 1).await)?.explanations[0].status == Status::Unknown,
        "expired competence is unknown",
    )?;

    let a = saved(skills.submit_assessment(&manager, 1, &assessment).await)?;
    saved(
        skills
            .review_assessment(&admin, 1, a.id, sk::ReviewDecision::Verify, None)
            .await,
    )?;
    saved(skills.finalize_assessment(&admin, 1, a.id).await)?;
    prove(
        saved(pg.explain(&staff, 1).await)?.explanations[0].status == Status::Met,
        "matching current profile and scale meet competence",
    )?;
    let low = saved(
        skills
            .submit_assessment(
                &manager,
                1,
                &sk::AssessmentInput {
                    assessed_level: 0,
                    request_key: "pg1-low".into(),
                    ..assessment.clone()
                },
            )
            .await,
    )?;
    saved(
        skills
            .review_assessment(&admin, 1, low.id, sk::ReviewDecision::Verify, None)
            .await,
    )?;
    saved(skills.finalize_assessment(&admin, 1, low.id).await)?;
    prove(
        saved(pg.explain(&staff, 1).await)?.explanations[0].status == Status::NotMet,
        "below required level is not met",
    )?;
    // A second profile version retires the old one; the pinned policy remains readable.
    let next = saved(skills.new_profile_version(&admin, 1, profile.id).await)?;
    saved(
        skills
            .publish_profile(&admin, 1, next.profile.profile.id, today)
            .await,
    )?;
    let wrong = saved(
        skills
            .submit_assessment(
                &manager,
                1,
                &sk::AssessmentInput {
                    request_key: "pg1-other-profile".into(),
                    ..assessment
                },
            )
            .await,
    )?;
    saved(
        skills
            .review_assessment(&admin, 1, wrong.id, sk::ReviewDecision::Verify, None)
            .await,
    )?;
    saved(skills.finalize_assessment(&admin, 1, wrong.id).await)?;
    prove(
        saved(pg.explain(&staff, 1).await)?.explanations[0].status == Status::Unknown,
        "differently profiled evidence is unknown after profile retirement",
    )?;
    refused(
        pg.draft(
            &manager,
            1,
            &DraftInput {
                version: 9,
                ..input.clone()
            },
        )
        .await,
        |e| matches!(e, ProgressionError::ProfileUnavailable),
        "retired profile cannot start a draft",
    )?;

    let records = saved(
        performance
            .my_records(&staff, 1, pm::PageOptions::default())
            .await,
    )?;
    let source = records
        .reviews
        .rows
        .iter()
        .find(|r| r.review.version == 2)
        .ok_or(Error::Internal)?;
    let target = saved(
        performance
            .propose_target(
                &manager,
                1,
                &pm::TargetInput {
                    definition_id: source.targets[0].definition_id,
                    employee_id: employee.employee_id,
                    target: "PG1 longer-period target".into(),
                    period_start: "2020-01-01".into(),
                    period_end: "2020-12-31".into(),
                    effective_from: "2020-01-01".into(),
                    confidential: false,
                },
            )
            .await,
    )?;
    saved(
        performance
            .review_target(&admin, 1, target.id, "Independent PG1 review")
            .await,
    )?;
    saved(performance.publish_target(&admin, 1, target.id).await)?;
    let later = saved(
        performance
            .draft_review(
                &manager,
                1,
                &pm::ReviewInput {
                    employee_id: employee.employee_id,
                    description_id: source.review.description_id,
                    period_start: "2020-01-01".into(),
                    period_end: "2020-12-31".into(),
                    cutoff_at: Timestamp::now().to_string(),
                    outcome: "Outside policy period".into(),
                    rationale: "PG1 paged proof".into(),
                    observation_ids: source.observations.iter().map(|o| o.id).collect(),
                    target_ids: vec![target.id],
                },
            )
            .await,
    )?;
    let mut last = saved(performance.release_review(&admin, 1, later.id).await)?;
    let pinned = saved(performance.review(&manager, 1, last.id).await)?;
    for version in 2..=101 {
        let correction = saved(
            performance
                .correct_review(
                    &manager,
                    1,
                    last.id,
                    &pm::ReviewCorrection {
                        description_id: last.description_id,
                        cutoff_at: None,
                        outcome: "Corrected outcome".into(),
                        rationale: format!("PG1 correction {version}"),
                        change_reason: "PG1 paged evidence proof".into(),
                        observation_ids: pinned.observations.iter().map(|r| r.id).collect(),
                        target_ids: pinned.targets.iter().map(|r| r.id).collect(),
                    },
                )
                .await,
        )?;
        last = saved(performance.release_review(&admin, 1, correction.id).await)?;
    }
    let paged = saved(pg.explain(&staff, 1).await)?;
    prove(
        matches!(
            &paged.explanations[0].rules[1],
            Criterion::Performance {
                review_version: Some(2),
                ..
            }
        ),
        "all released review pages are scanned",
    )?;

    let performance_only = DraftInput {
        code: "pg1_performance".into(),
        profile: None,
        ..input.clone()
    };
    let first = saved(pg.draft(&manager, 1, &performance_only).await)?;
    saved(pg.publish(&manager, 1, first.id).await)?;
    let second = saved(
        pg.draft(
            &manager,
            1,
            &DraftInput {
                version: 2,
                ..performance_only.clone()
            },
        )
        .await,
    )?;
    saved(pg.publish(&manager, 1, second.id).await)?;
    let older = saved(
        pg.draft(
            &manager,
            1,
            &DraftInput {
                version: 3,
                effective_from: "2019-12-31".into(),
                ..performance_only.clone()
            },
        )
        .await,
    )?;
    refused(
        pg.publish(&manager, 1, older.id).await,
        |e| matches!(e, ProgressionError::EffectiveOrder),
        "backdated publication",
    )?;
    let lower = saved(
        pg.draft(
            &manager,
            1,
            &DraftInput {
                version: 1,
                code: "pg1_order".into(),
                ..performance_only.clone()
            },
        )
        .await,
    )?;
    let higher = saved(
        pg.draft(
            &manager,
            1,
            &DraftInput {
                version: 2,
                code: "pg1_order".into(),
                ..performance_only.clone()
            },
        )
        .await,
    )?;
    saved(pg.publish(&manager, 1, higher.id).await)?;
    refused(
        pg.publish(&manager, 1, lower.id).await,
        |e| matches!(e, ProgressionError::VersionOrder),
        "lower version publication",
    )?;
    let future = saved(
        pg.draft(
            &manager,
            1,
            &DraftInput {
                version: 4,
                effective_from: "9999-01-01".into(),
                ..performance_only.clone()
            },
        )
        .await,
    )?;
    saved(pg.publish(&manager, 1, future.id).await)?;
    let standing = saved(pg.explain(&staff, 1).await)?;
    prove(
        standing
            .explanations
            .iter()
            .find(|e| e.policy.code == "pg1_performance")
            .is_some_and(|e| e.policy.id == second.id),
        "same-day higher version governs; future publication leaves it current",
    )?;
    let missing = PerformanceRule {
        periods: vec![Period {
            start: "2018-01-01".into(),
            end: "2018-12-31".into(),
        }],
        missing_evidence: MissingEvidence::NotMet,
        ..perf.clone()
    };
    let absent = saved(
        pg.draft(
            &manager,
            1,
            &DraftInput {
                code: "pg1_missing".into(),
                performance: Some(missing),
                ..performance_only.clone()
            },
        )
        .await,
    )?;
    saved(pg.publish(&manager, 1, absent.id).await)?;
    prove(
        saved(pg.explain(&staff, 1).await)?
            .explanations
            .iter()
            .find(|e| e.policy.id == absent.id)
            .is_some_and(|e| e.status == Status::NotMet),
        "explicit missing-evidence not_met rule",
    )?;
    let rejecting = saved(
        pg.draft(
            &manager,
            1,
            &DraftInput {
                code: "pg1_outcome".into(),
                performance: Some(PerformanceRule {
                    accepted_outcomes: vec!["Different outcome".into()],
                    ..perf
                }),
                ..performance_only.clone()
            },
        )
        .await,
    )?;
    saved(pg.publish(&manager, 1, rejecting.id).await)?;
    prove(
        saved(pg.explain(&staff, 1).await)?
            .explanations
            .iter()
            .find(|e| e.policy.id == rejecting.id)
            .is_some_and(|e| e.status == Status::NotMet),
        "released outcome outside accepted values",
    )?;
    for version in 1..=12 {
        saved(
            pg.draft(
                &manager,
                1,
                &DraftInput {
                    code: "pg1_history".into(),
                    version,
                    ..performance_only.clone()
                },
            )
            .await,
        )?;
    }
    let history = saved(
        pg.policies(
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
        history.total >= 20 && history.page == (history.total + 9) / 10 && history.rows.len() <= 10,
        "bounded history clamps oversized pages",
    )?;
    refused(
        pg.policies(
            &manager,
            1,
            PageOptions {
                page: 0,
                page_size: 10,
            },
        )
        .await,
        |e| matches!(e, ProgressionError::InvalidOptions),
        "invalid history options",
    )?;
    refused(
        pg.policies(&viewer, 1, PageOptions::default()).await,
        |e| matches!(e, ProgressionError::Unauthorized),
        "history permission",
    )?;
    let impersonated = auth
        .authenticate_session(&**client, 1, 1, Some(4), "walkthrough".into())
        .await?;
    refused(
        pg.draft(&impersonated, 1, &performance_only).await,
        |e| matches!(e, ProgressionError::ImpersonationRefused),
        "impersonated draft",
    )?;
    refused(
        pg.publish(&impersonated, 1, older.id).await,
        |e| matches!(e, ProgressionError::ImpersonationRefused),
        "impersonated publication",
    )?;
    grants
        .set_direct(&admin, 1, POLICY_MANAGE, Some(false))
        .await?;
    refused(
        pg.draft(&manager, 1, &performance_only).await,
        |e| matches!(e, ProgressionError::Unauthorized),
        "revoked grant on existing scope",
    )?;
    grants
        .set_direct(&admin, 1, POLICY_MANAGE, Some(true))
        .await?;
    grants
        .set_direct(&admin, 3, sk::SELF_VIEW, Some(false))
        .await?;
    prove(
        pg.explain(&staff, 1).await.is_err(),
        "missing Skills evidence permission refuses explanation",
    )?;
    grants
        .set_direct(&admin, 3, sk::SELF_VIEW, Some(true))
        .await?;
    grants
        .set_direct(&admin, 3, pm::SELF_VIEW, Some(false))
        .await?;
    prove(
        pg.explain(&staff, 1).await.is_err(),
        "missing Performance evidence permission refuses explanation",
    )?;
    grants
        .set_direct(&admin, 3, pm::SELF_VIEW, Some(true))
        .await?;
    let users = services.get::<UserService>("core/user.accounts")?;
    saved(users.unlink(&admin, 3).await)?;
    refused(
        pg.explain(&staff, 1).await,
        |e| matches!(e, ProgressionError::EmployeeUnavailable),
        "old scope after unlink",
    )?;
    // Restore the same account through its known employment affiliation.
    let employment =
        saved(saved(people.employment(&admin, 1, employee.employee_id).await)?.require_current())?;
    saved(
        users
            .link_affiliation(
                &admin,
                3,
                bilibi_party_ids::AffiliationId::new(employment.affiliation_id)
                    .map_err(|_| Error::Invalid)?,
            )
            .await,
    )?;
    db_refused(
        &client,
        "UPDATE people_progression_policy_versions SET name='changed' WHERE id=$1",
        policy.id,
        "published content edit",
    )
    .await?;
    db_refused(&client,"UPDATE people_progression_policy_versions SET status='draft',published_by_user_id=NULL,published_at=NULL WHERE id=$1",policy.id,"published workflow reversal").await?;
    db_refused(
        &client,
        "DELETE FROM people_progression_policy_versions WHERE id=$1",
        policy.id,
        "published deletion",
    )
    .await?;
    let companies = services.get::<bilibi_company::CompanyService>(bilibi_company::SERVICE)?;
    let company = match companies
        .create_company(
            &admin,
            &bilibi_company::NewCompany {
                name: "PG1 archive proof".into(),
                code: Some("pg1-archive".into()),
                ..Default::default()
            },
        )
        .await?
    {
        bilibi_company::CreateOutcome::Created(company) => company,
        other => {
            eprintln!("PG1 company refused: {other:?}");
            return Err(Error::Internal);
        }
    };
    saved(pg.draft(&admin, company.id, &performance_only).await)?;
    saved(
        companies
            .archive_company(&admin, company.id, Some("PG1 immutable company proof"))
            .await,
    )?;
    refused(
        pg.draft(&admin, company.id, &performance_only).await,
        // Workforce::live_company deliberately makes already archived
        // companies indistinguishable from missing companies.
        |e| matches!(e, ProgressionError::NotFound),
        "archived company refuses policy write",
    )?;

    // Revoke permission in another session while a write waits for the company.
    let mut blocker = db.get().await?;
    let transaction = blocker.transaction().await?;
    let blocker_pid: i32 = transaction
        .query_one("SELECT pg_backend_pid()", &[])
        .await?
        .get(0);
    transaction
        .query_one(
            "SELECT id FROM companies WHERE tenant_id=1 AND id=1 FOR UPDATE",
            &[],
        )
        .await?;
    let pending_pg = pg.clone();
    let pending_scope = manager.clone();
    let pending_input = DraftInput {
        code: "pg1_waiting_grant".into(),
        ..performance_only.clone()
    };
    let waiting =
        tokio::spawn(async move { pending_pg.draft(&pending_scope, 1, &pending_input).await });
    let mut reached_lock = false;
    for _ in 0..100 {
        reached_lock=client.query_one("SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE $1=ANY(pg_blocking_pids(pid)))",&[&blocker_pid]).await?.get(0);
        if reached_lock {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    prove(reached_lock, "second session waits for company lock")?;
    grants
        .set_direct(&admin, 1, POLICY_MANAGE, Some(false))
        .await?;
    transaction.rollback().await?;
    refused(
        waiting.await.map_err(|_| Error::Internal)?,
        |e| matches!(e, ProgressionError::Unauthorized),
        "grant revoked while write waited for company lock",
    )?;
    grants
        .set_direct(&admin, 1, POLICY_MANAGE, Some(true))
        .await?;

    let prior_count = saved(pg.explain(&staff, 1).await)?.explanations.len();
    let mut last_policy_id = 0;
    for index in 0..101 {
        let row = saved(
            pg.draft(
                &manager,
                1,
                &DraftInput {
                    code: format!("zz_pg1_batch_{index:03}"),
                    version: 1,
                    name: "PG1 governing-policy batch proof".into(),
                    effective_from: today.to_string(),
                    profile: Some(ProfilePin {
                        id: next.profile.profile.id,
                        version: next.profile.profile.version,
                    }),
                    performance: None,
                },
            )
            .await,
        )?;
        last_policy_id = saved(pg.publish(&manager, 1, row.id).await)?.id;
    }
    let all_codes = saved(pg.explain(&staff, 1).await)?;
    prove(
        all_codes.explanations.len() == prior_count + 101
            && all_codes
                .explanations
                .iter()
                .any(|e| e.policy.id == last_policy_id && e.status == Status::Met),
        "every governing policy code is explained across bounded batches",
    )?;

    let count:i64=client.query_one("SELECT count(*) FROM base_audit_mutations WHERE tenant_id=1 AND auditable_type='people_progression_policy_versions'",&[]).await?.get(0);
    let expected:i64=client.query_one("SELECT count(*)+count(published_at) FROM people_progression_policy_versions WHERE tenant_id=1",&[]).await?.get(0);
    prove(count == expected, "every progression write is audited")?;
    println!("PG1 proof complete: {count} audited policy changes");
    Ok(())
}

walkthrough_command!(
    "progression",
    |url, _argument| async move { run(url).await }
);
