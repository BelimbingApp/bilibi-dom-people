//! PK2: live facade and PostgreSQL proofs on the disposable walkthrough DB.
//! All records start through their owner's facade; SQL is used only to inspect
//! outcomes and deliberately attempt writes the database must refuse.
use bilibi_affiliation::{
    AffiliationId, AffiliationInput, AffiliationService, EffectiveDates, OrganisationId,
};
use bilibi_authz::grants::Grants;
use bilibi_database::{Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_skills::{
    Assessment, AssessmentFilters, AssessmentInput, PolicyChanges, ReviewDecision, Skills,
    SkillsError, ASSESSMENTS_APPROVE, ASSESSMENTS_MANAGE, ASSESSMENTS_REVIEW, ASSESSMENTS_SUBMIT,
    ASSESSMENTS_VIEW, SELF_VIEW,
};
use bilibi_people_workforce::{EmploymentChange, NewEmployment, Workforce};
use bilibi_person::{PersonInput, PersonKind, PersonService};
use bilibi_tenancy::{Authentication, Scope};
use bilibi_user::UserService;
use jiff::civil::date;
use std::fmt::Debug;
use tokio_postgres::Client;

pub(crate) struct Context<'a> {
    pub admin: &'a Scope,
    pub manager: &'a Scope,
    pub viewer: &'a Scope,
    pub manager_employee: i64,
    pub staff_employee: i64,
}

fn saved<T>(result: bilibi_people_skills::Result<T>) -> Result<T> {
    result.map_err(|error| {
        eprintln!("PK2 unexpected refusal: {error:?}");
        Error::Internal
    })
}
fn check(label: &str, condition: bool) -> Result<()> {
    if !condition {
        eprintln!("PK2 failed: {label}");
        return Err(Error::Internal);
    }
    println!("PK2 proved: {label}");
    Ok(())
}
fn refuses<T: Debug>(
    label: &str,
    result: bilibi_people_skills::Result<T>,
    expected: fn(&SkillsError) -> bool,
) -> Result<()> {
    match result {
        Err(ref error) if expected(error) => {
            println!("PK2 refused: {label}: {error}");
            Ok(())
        }
        other => {
            eprintln!("PK2 unexpected outcome for {label}: {other:?}");
            Err(Error::Internal)
        }
    }
}
async fn sql_refuses(client: &Client, label: &str, sql: &str, id: i64) -> Result<()> {
    match client.execute(sql, &[&id]).await {
        Err(e) if e.as_db_error().is_some() => {
            println!(
                "PK2 database refused: {label}: {}",
                e.as_db_error().map(|e| e.message()).unwrap_or_default()
            );
            Ok(())
        }
        other => {
            eprintln!("PK2 database should refuse {label}: {other:?}");
            Err(Error::Internal)
        }
    }
}
async fn finalized(skills: &Skills, reviewer: &Scope, input: Assessment) -> Result<Assessment> {
    saved(
        skills
            .review_assessment(
                reviewer,
                super::COMPANY,
                input.id,
                ReviewDecision::Verify,
                None,
            )
            .await,
    )?;
    saved(
        skills
            .finalize_assessment(reviewer, super::COMPANY, input.id)
            .await,
    )
}

pub(crate) async fn run(services: &Services, client: &Client, c: Context<'_>) -> Result<()> {
    let company = super::COMPANY;
    let skills = services.get::<Skills>(bilibi_people_skills::SERVICE)?;
    let workforce = services.get::<Workforce>(bilibi_people_workforce::SERVICE)?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    let users = services.get::<UserService>("core/user.accounts")?;
    let auth = services.get::<Authentication>("base/tenancy.authentication")?;
    println!("\n== PK2: assessments, decisions, supersession and standing");
    for capability in [
        ASSESSMENTS_SUBMIT,
        ASSESSMENTS_VIEW,
        ASSESSMENTS_REVIEW,
        ASSESSMENTS_APPROVE,
        SELF_VIEW,
    ] {
        grants
            .set_direct(c.admin, super::MANAGER, capability, Some(true))
            .await?;
    }
    for capability in [
        ASSESSMENTS_SUBMIT,
        ASSESSMENTS_VIEW,
        ASSESSMENTS_REVIEW,
        ASSESSMENTS_APPROVE,
        ASSESSMENTS_MANAGE,
        SELF_VIEW,
    ] {
        grants
            .set_direct(c.admin, super::STAFF, capability, Some(true))
            .await?;
    }
    // An unlinked viewer needs the company axis to reach this company at
    // all; this does not grant company-wide reach over Skills employees.
    grants
        .set_direct(
            c.admin,
            super::VIEWER,
            bilibi_company::TENANT_WIDE,
            Some(true),
        )
        .await?;
    grants
        .set_direct(c.admin, super::VIEWER, ASSESSMENTS_SUBMIT, Some(true))
        .await?;
    grants
        .set_direct(c.admin, super::VIEWER, ASSESSMENTS_VIEW, Some(true))
        .await?;
    let staff_employment = workforce
        .employment(c.admin, company, c.staff_employee)
        .await
        .map_err(|_| Error::Internal)?
        .require_current()
        .map_err(|_| Error::Internal)?;
    users
        .link_affiliation(
            c.admin,
            super::STAFF,
            AffiliationId::new(staff_employment.affiliation_id).map_err(|_| Error::Invalid)?,
        )
        .await
        .map_err(|_| Error::Internal)?;
    let staff = auth
        .authenticate(client, super::TENANT, super::STAFF, "walkthrough".into())
        .await?;
    let catalog = saved(skills.list_skills(c.admin, company).await)?;
    let skill = catalog
        .iter()
        .find(|s| s.code == "welding")
        .ok_or(Error::Internal)?;
    let other_skill = catalog
        .iter()
        .find(|s| s.code == "forklift")
        .ok_or(Error::Internal)?;
    let profile = saved(
        skills
            .requirements(c.admin, company, None, date(2026, 10, 1))
            .await,
    )?
    .ok_or(Error::Internal)?;
    let requirement = profile
        .items
        .iter()
        .find(|i| i.skill_id == skill.id)
        .ok_or(Error::Internal)?;
    let policy = saved(skills.policy(c.admin, company).await)?;
    let base = AssessmentInput {
        employee_id: c.staff_employee,
        skill_id: skill.id,
        assessed_level: 0,
        evidence: "Observed the work and retained the inspection record.".into(),
        assessed_on: Some(date(2026, 10, 1)),
        request_key: "pk2-initial".into(),
        ..AssessmentInput::default()
    };
    refuses(
        "viewer without the catalog's assessment submit reach",
        skills.submit_assessment(c.viewer, company, &base).await,
        |e| matches!(e, SkillsError::OutOfReach),
    )?;
    refuses(
        "manager cannot assess self",
        skills
            .submit_assessment(
                c.manager,
                company,
                &AssessmentInput {
                    employee_id: c.manager_employee,
                    request_key: "pk2-self".into(),
                    ..base.clone()
                },
            )
            .await,
        |e| matches!(e, SkillsError::SelfAssessment),
    )?;
    refuses(
        "empty evidence",
        skills
            .submit_assessment(
                c.manager,
                company,
                &AssessmentInput {
                    evidence: " ".into(),
                    ..base.clone()
                },
            )
            .await,
        |e| matches!(e, SkillsError::InvalidAssessment),
    )?;
    refuses(
        "future assessment",
        skills
            .submit_assessment(
                c.manager,
                company,
                &AssessmentInput {
                    assessed_on: Some(date(9999, 1, 1)),
                    ..base.clone()
                },
            )
            .await,
        |e| matches!(e, SkillsError::FutureAssessment),
    )?;
    refuses(
        "validity precedes assessment",
        skills
            .submit_assessment(
                c.manager,
                company,
                &AssessmentInput {
                    valid_until: Some(date(2026, 9, 30)),
                    ..base.clone()
                },
            )
            .await,
        |e| matches!(e, SkillsError::InvalidValidity),
    )?;
    refuses(
        "a level absent from the pinned scale",
        skills
            .submit_assessment(
                c.manager,
                company,
                &AssessmentInput {
                    assessed_level: 20,
                    ..base.clone()
                },
            )
            .await,
        |e| matches!(e, SkillsError::AssessmentLevelNotOnScale),
    )?;
    refuses(
        "no requirement before the profile starts",
        skills
            .submit_assessment(
                c.manager,
                company,
                &AssessmentInput {
                    assessed_on: Some(date(2025, 1, 1)),
                    ..base.clone()
                },
            )
            .await,
        |e| matches!(e, SkillsError::NoRequirement),
    )?;
    refuses(
        "missing skill",
        skills
            .submit_assessment(
                c.manager,
                company,
                &AssessmentInput {
                    skill_id: i64::MAX,
                    ..base.clone()
                },
            )
            .await,
        |e| matches!(e, SkillsError::AssessmentSkillUnavailable),
    )?;
    refuses(
        "foreign company subject",
        skills
            .submit_assessment(c.manager, super::OTHER_COMPANY, &base)
            .await,
        |e| matches!(e, SkillsError::NotFound),
    )?;
    let a = saved(skills.submit_assessment(c.manager, company, &base).await)?;
    check(
        "the requirement, scale, weight, mandatory flag, gap, band and priority are snapshotted",
        a.profile_id == profile.profile.id
            && a.scale_id == profile.profile.scale_id
            && a.required_level == requirement.required_level
            && a.weight_percent == requirement.weight_percent
            && a.mandatory == requirement.mandatory
            && a.gap == requirement.required_level
            && a.priority_multiplier == policy.multiplier(&requirement.criticality) as i32
            && a.priority_score == a.gap * a.priority_multiplier
            && a.result_band == "critical_gap"
            && a.next_due_on == Some(date(2028, 4, 1)),
    )?;
    let replay = saved(
        skills
            .submit_assessment(
                c.manager,
                company,
                &AssessmentInput {
                    method: Some("Optional retry metadata".into()),
                    ..base.clone()
                },
            )
            .await,
    )?;
    check(
        "same request key and five facts replay without another decision",
        replay.id == a.id
            && saved(skills.assessment_decisions(c.admin, company, a.id).await)?.len() == 1,
    )?;
    refuses(
        "same key with changed level",
        skills
            .submit_assessment(
                c.manager,
                company,
                &AssessmentInput {
                    assessed_level: 1,
                    ..base.clone()
                },
            )
            .await,
        |e| matches!(e, SkillsError::KeyConflict),
    )?;
    saved(
        skills
            .put_policy(
                c.admin,
                company,
                &PolicyChanges {
                    multiplier_critical: Some(9),
                    backup_minimum: Some(2),
                    ..PolicyChanges::default()
                },
            )
            .await,
    )?;
    let list = saved(
        skills
            .list_assessments(c.admin, company, &AssessmentFilters::default())
            .await,
    )?;
    check(
        "policy changes preserve historical priority",
        list.iter().any(|v| {
            v.assessment.id == a.id && v.assessment.priority_multiplier == a.priority_multiplier
        }),
    )?;
    check(
        "pending work does not create a score",
        saved(skills.standing(&staff, company).await)?
            .scores
            .is_empty(),
    )?;
    refuses(
        "assessor cannot verify",
        skills
            .review_assessment(c.manager, company, a.id, ReviewDecision::Verify, None)
            .await,
        |e| matches!(e, SkillsError::SelfApproval),
    )?;
    refuses(
        "subject cannot verify even with manage",
        skills
            .review_assessment(&staff, company, a.id, ReviewDecision::Verify, None)
            .await,
        |e| matches!(e, SkillsError::SelfApproval),
    )?;
    refuses(
        "return without a note",
        skills
            .review_assessment(c.admin, company, a.id, ReviewDecision::Return, Some(" "))
            .await,
        |e| matches!(e, SkillsError::NoteRequired),
    )?;
    refuses(
        "pending work cannot be finalized",
        skills.finalize_assessment(c.admin, company, a.id).await,
        |e| matches!(e, SkillsError::NotVerified),
    )?;
    check(
        "review queue omits the assessor and subject",
        saved(skills.assessment_queue(c.manager, company).await)?
            .review
            .is_empty()
            && saved(skills.assessment_queue(&staff, company).await)?
                .review
                .is_empty()
            && saved(skills.assessment_queue(c.admin, company).await)?
                .review
                .iter()
                .any(|v| v.assessment.id == a.id),
    )?;
    sql_refuses(client,"assessor review enforced in SQL","UPDATE people_skill_assessments SET status='verified',reviewed_by_user_id=assessor_user_id,reviewed_at=now() WHERE id=$1",a.id).await?;
    sql_refuses(client,"assessment facts immutable","UPDATE people_skill_assessments SET evidence='changed',status='verified',reviewed_by_user_id=4,reviewed_at=now() WHERE id=$1",a.id).await?;
    saved(
        skills
            .review_assessment(
                c.admin,
                company,
                a.id,
                ReviewDecision::Verify,
                Some("Evidence checked."),
            )
            .await,
    )?;
    refuses(
        "assessor cannot finalize",
        skills.finalize_assessment(c.manager, company, a.id).await,
        |e| matches!(e, SkillsError::SelfApproval),
    )?;
    refuses(
        "subject cannot finalize",
        skills.finalize_assessment(&staff, company, a.id).await,
        |e| matches!(e, SkillsError::SelfApproval),
    )?;
    sql_refuses(client,"assessor finalization enforced in SQL","UPDATE people_skill_assessments SET status='finalized',finalized_by_user_id=assessor_user_id,finalized_at=now() WHERE id=$1",a.id).await?;
    saved(skills.finalize_assessment(c.admin, company, a.id).await)?;
    check(
        "finalization creates a matching score and three ledger decisions",
        saved(skills.standing(&staff, company).await)?
            .scores
            .iter()
            .any(|s| {
                s.score.assessment_id == a.id
                    && s.assessment_profile_version == profile.profile.version
                    && s.assessment_scale_id == a.scale_id
            })
            && saved(skills.assessment_decisions(c.admin, company, a.id).await)?.len() == 3,
    )?;
    check(
        "gaps include finalized work in the manager's chain",
        saved(skills.gaps(c.manager, company).await)?
            .iter()
            .any(|s| s.score.assessment_id == a.id),
    )?;
    refuses(
        "coverage needs company-wide manage",
        skills
            .coverage(c.manager, company, date(2026, 10, 22))
            .await,
        |e| matches!(e, SkillsError::OutOfReach),
    )?;
    let b_input = AssessmentInput {
        assessed_on: Some(date(2026, 9, 30)),
        assessed_level: 1,
        supersedes_assessment_id: Some(a.id),
        request_key: "pk2-correction-returned".into(),
        ..base.clone()
    };
    let b = saved(skills.submit_assessment(c.manager, company, &b_input).await)?;
    check(
        "two-level gap has the major band",
        b.result_band == "major_gap",
    )?;
    saved(
        skills
            .review_assessment(
                c.admin,
                company,
                b.id,
                ReviewDecision::Return,
                Some("Attach the missing observation."),
            )
            .await,
    )?;
    check(
        "returned correction leaves the finalized score intact",
        saved(skills.standing(&staff, company).await)?.scores[0]
            .score
            .assessment_id
            == a.id,
    )?;
    refuses(
        "returned work can only be corrected by its original assessor",
        skills
            .submit_assessment(
                c.admin,
                company,
                &AssessmentInput {
                    supersedes_assessment_id: Some(b.id),
                    request_key: "pk2-wrong-assessor".into(),
                    ..base.clone()
                },
            )
            .await,
        |e| matches!(e, SkillsError::NotOriginalAssessor),
    )?;
    let c_input = AssessmentInput {
        assessed_on: Some(date(2026, 9, 29)),
        assessed_level: 3,
        supersedes_assessment_id: Some(b.id),
        request_key: "pk2-chain-correction".into(),
        ..base.clone()
    };
    let correction = saved(skills.submit_assessment(c.manager, company, &c_input).await)?;
    refuses(
        "one successor per assessment",
        skills
            .submit_assessment(
                c.manager,
                company,
                &AssessmentInput {
                    request_key: "pk2-second-successor".into(),
                    ..c_input.clone()
                },
            )
            .await,
        |e| matches!(e, SkillsError::AlreadySuperseded),
    )?;
    refuses(
        "pending work cannot be corrected",
        skills
            .submit_assessment(
                c.manager,
                company,
                &AssessmentInput {
                    supersedes_assessment_id: Some(correction.id),
                    request_key: "pk2-pending-successor".into(),
                    ..base.clone()
                },
            )
            .await,
        |e| matches!(e, SkillsError::NotSupersedable),
    )?;
    let corrected = finalized(&skills, c.admin, correction).await?;
    let own = saved(skills.standing(&staff, company).await)?;
    check(
        "finalized correction excludes every ancestor through a returned node, even when backdated",
        own.scores[0].score.assessment_id == corrected.id
            && own.scores[0].result_band == "meets"
            && own.scores[0].score.gap == 0,
    )?;
    let older = saved(
        skills
            .submit_assessment(
                c.manager,
                company,
                &AssessmentInput {
                    assessed_on: Some(date(2026, 8, 1)),
                    request_key: "pk2-older-independent".into(),
                    ..base.clone()
                },
            )
            .await,
    )?;
    finalized(&skills, c.admin, older).await?;
    check(
        "finalizing older independent work does not replace the latest score",
        saved(skills.standing(&staff, company).await)?.scores[0]
            .score
            .assessment_id
            == corrected.id,
    )?;
    let same_day = saved(
        skills
            .submit_assessment(
                c.manager,
                company,
                &AssessmentInput {
                    assessed_on: Some(date(2026, 9, 29)),
                    assessed_level: 3,
                    request_key: "pk2-tied-day".into(),
                    ..base.clone()
                },
            )
            .await,
    )?;
    let same_day = finalized(&skills, c.admin, same_day).await?;
    check(
        "ties on assessment date use the newer id",
        saved(skills.standing(&staff, company).await)?.scores[0]
            .score
            .assessment_id
            == same_day.id,
    )?;
    check(
        "met requirement has the meets band",
        same_day.result_band == "meets",
    )?;
    let expired = saved(
        skills
            .submit_assessment(
                c.manager,
                company,
                &AssessmentInput {
                    skill_id: other_skill.id,
                    assessed_level: 0,
                    assessed_on: Some(date(2026, 1, 31)),
                    valid_until: Some(date(2026, 2, 28)),
                    request_key: "pk2-expired".into(),
                    ..base.clone()
                },
            )
            .await,
    )?;
    let expired = finalized(&skills, c.admin, expired).await?;
    check(
        "explicit validity is the next due date and one-level gap is minor",
        expired.next_due_on == Some(date(2026, 2, 28)) && expired.result_band == "minor_gap",
    )?;
    check(
        "standing distinguishes expired scores",
        saved(skills.standing(&staff, company).await)?
            .scores
            .iter()
            .any(|s| s.score.assessment_id == expired.id && s.state == "expired"),
    )?;
    let month_end = saved(
        skills
            .submit_assessment(
                c.manager,
                company,
                &AssessmentInput {
                    assessed_on: Some(date(2026, 1, 31)),
                    request_key: "pk2-month-end".into(),
                    ..base.clone()
                },
            )
            .await,
    )?;
    check(
        "skill interval takes precedence over policy",
        month_end.next_due_on == Some(date(2027, 7, 31)),
    )?;
    saved(
        skills
            .put_policy(
                c.admin,
                company,
                &PolicyChanges {
                    default_reassessment_months: Some(1),
                    ..PolicyChanges::default()
                },
            )
            .await,
    )?;
    let clamped = saved(
        skills
            .submit_assessment(
                c.manager,
                company,
                &AssessmentInput {
                    skill_id: other_skill.id,
                    assessed_on: Some(date(2026, 1, 31)),
                    valid_until: None,
                    request_key: "pk2-clamped-month".into(),
                    ..base.clone()
                },
            )
            .await,
    )?;
    check(
        "default interval clamps January 31 to February 28",
        clamped.next_due_on == Some(date(2026, 2, 28)),
    )?;
    refuses(
        "manage does not allow self assessment",
        skills
            .submit_assessment(
                &staff,
                company,
                &AssessmentInput {
                    request_key: "pk2-manage-self".into(),
                    ..base.clone()
                },
            )
            .await,
        |e| matches!(e, SkillsError::SelfAssessment),
    )?;
    // A second employee two levels down proves transitive reporting reach.
    let persons = services.get::<PersonService>(bilibi_person::SERVICE)?;
    let affiliations = services.get::<AffiliationService>(bilibi_affiliation::SERVICE)?;
    let person = persons
        .create(
            c.admin,
            &PersonInput {
                name: "Assessment Report".into(),
                kind: PersonKind::Human,
                email: None,
                phone: None,
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    let affiliation = affiliations
        .create(
            c.admin,
            &AffiliationInput {
                person_id: person.id,
                organisation_id: OrganisationId::new(company).map_err(|_| Error::Invalid)?,
                title: None,
                work_email: None,
                work_phone: None,
                dates: EffectiveDates::new(None, None).map_err(|_| Error::Invalid)?,
            },
        )
        .await?;
    let report = workforce
        .create_employment(
            c.admin,
            company,
            &NewEmployment {
                supervisor_employment_id: Some(c.staff_employee),
                ..NewEmployment::new(
                    person.id.get(),
                    affiliation.id.get(),
                    "PK2-REPORT",
                    "full_time",
                    "active",
                )
            },
        )
        .await
        .map_err(|e| {
            eprintln!("{e:?}");
            Error::Internal
        })?;
    let indirect = saved(
        skills
            .submit_assessment(
                c.manager,
                company,
                &AssessmentInput {
                    employee_id: report.id,
                    assessed_level: 3,
                    request_key: "pk2-indirect".into(),
                    valid_until: Some(date(2026, 10, 22)),
                    ..base.clone()
                },
            )
            .await,
    )?;
    finalized(&skills, c.admin, indirect).await?;
    check(
        "assessable reaches indirect reports and never self",
        saved(skills.assessable(c.manager, company).await)?
            .iter()
            .any(|e| e.id == report.id)
            && !saved(skills.assessable(c.manager, company).await)?
                .iter()
                .any(|e| e.id == c.manager_employee),
    )?;
    let coverage = saved(skills.coverage(c.admin, company, date(2026, 10, 22)).await)?;
    check(
        "coverage counts competent working holders against backup minimum",
        coverage
            .iter()
            .any(|c| c.skill_id == skill.id && c.holders == 2 && c.covered),
    )?;
    check(
        "coverage validity is inclusive on its last day",
        saved(skills.coverage(c.admin, company, date(2026, 10, 23)).await)?
            .iter()
            .any(|c| c.skill_id == skill.id && c.holders == 1 && !c.covered),
    )?;
    let concurrent_input = AssessmentInput {
        employee_id: report.id,
        assessed_on: Some(date(2026, 10, 2)),
        request_key: "pk2-concurrent-retry".into(),
        assessed_level: 3,
        valid_until: Some(date(2026, 10, 22)),
        ..base.clone()
    };
    let (one, two) = tokio::join!(
        skills.submit_assessment(c.manager, company, &concurrent_input),
        skills.submit_assessment(c.manager, company, &concurrent_input)
    );
    let concurrent = saved(one)?;
    check(
        "concurrent retries return one assessment and one submitted decision",
        saved(two)?.id == concurrent.id
            && saved(
                skills
                    .assessment_decisions(c.admin, company, concurrent.id)
                    .await,
            )?
            .len()
                == 1,
    )?;
    // Relinking changes subject independence even for an old authenticated scope.
    users
        .unlink(c.admin, super::STAFF)
        .await
        .map_err(|_| Error::Internal)?;
    users
        .link_affiliation(c.admin, super::STAFF, affiliation.id)
        .await
        .map_err(|_| Error::Internal)?;
    refuses(
        "old scope after relink to the assessed employee",
        skills
            .review_assessment(&staff, company, concurrent.id, ReviewDecision::Verify, None)
            .await,
        |e| matches!(e, SkillsError::SelfApproval),
    )?;
    users
        .unlink(c.admin, super::STAFF)
        .await
        .map_err(|_| Error::Internal)?;
    users
        .link_affiliation(
            c.admin,
            super::STAFF,
            AffiliationId::new(staff_employment.affiliation_id).map_err(|_| Error::Invalid)?,
        )
        .await
        .map_err(|_| Error::Internal)?;
    saved(
        skills
            .review_assessment(
                c.admin,
                company,
                concurrent.id,
                ReviewDecision::Verify,
                None,
            )
            .await,
    )?;
    let (one, two) = tokio::join!(
        skills.finalize_assessment(c.admin, company, concurrent.id),
        skills.finalize_assessment(c.admin, company, concurrent.id)
    );
    check(
        "concurrent finalization commits exactly one decision",
        matches!(
            (&one, &two),
            (Ok(_), Err(SkillsError::NotVerified)) | (Err(SkillsError::NotVerified), Ok(_))
        ) && saved(
            skills
                .assessment_decisions(c.admin, company, concurrent.id)
                .await,
        )?
        .len()
            == 3,
    )?;
    let new_status = workforce
        .update_employment(
            c.admin,
            company,
            report.id,
            &EmploymentChange {
                status: Some("inactive".into()),
                ..EmploymentChange::at(report.version)
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    let coverage = saved(skills.coverage(c.admin, company, date(2026, 10, 22)).await)?;
    check(
        "coverage excludes nonworking holders",
        coverage
            .iter()
            .any(|c| c.skill_id == skill.id && c.holders == 1 && !c.covered),
    )?;
    workforce
        .update_employment(
            c.admin,
            company,
            report.id,
            &EmploymentChange {
                status: Some("active".into()),
                ..EmploymentChange::at(new_status.version)
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    let changed = workforce
        .update_employment(
            c.admin,
            company,
            c.staff_employee,
            &EmploymentChange {
                status: Some("inactive".into()),
                ..EmploymentChange::at(staff_employment.version)
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    refuses(
        "nonworking subject still cannot review own work",
        skills
            .review_assessment(&staff, company, month_end.id, ReviewDecision::Verify, None)
            .await,
        |e| matches!(e, SkillsError::SelfApproval),
    )?;
    finalized(&skills, c.admin, month_end).await?;
    workforce
        .update_employment(
            c.admin,
            company,
            c.staff_employee,
            &EmploymentChange {
                status: Some("active".into()),
                ..EmploymentChange::at(changed.version)
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    let exceeds = saved(
        skills
            .submit_assessment(
                c.manager,
                company,
                &AssessmentInput {
                    skill_id: other_skill.id,
                    assessed_level: 2,
                    // This is a standing read at the real UTC clock, unlike the
                    // explicit as-of coverage checks above.
                    valid_until: Some(
                        jiff::Timestamp::now()
                            .to_zoned(jiff::tz::TimeZone::UTC)
                            .date(),
                    ),
                    request_key: "pk2-exceeds".into(),
                    ..base.clone()
                },
            )
            .await,
    )?;
    let exceeds = finalized(&skills, c.admin, exceeds).await?;
    check(
        "level above its requirement has the exceeds band",
        exceeds.result_band == "exceeds" && exceeds.gap == 0,
    )?;
    check(
        "standing reports a score past its validity date as expired",
        saved(skills.standing(&staff, company).await)?
            .scores
            .iter()
            .any(|s| s.score.assessment_id == exceeds.id && s.state == "expired"),
    )?;
    // Revoked grants and unlinked accounts are checked against this same Scope.
    grants
        .set_direct(c.admin, super::MANAGER, ASSESSMENTS_SUBMIT, Some(false))
        .await?;
    refuses(
        "revoked grant with old scope",
        skills
            .submit_assessment(
                c.manager,
                company,
                &AssessmentInput {
                    request_key: "pk2-revoked".into(),
                    ..base.clone()
                },
            )
            .await,
        |e| matches!(e, SkillsError::Unauthorized),
    )?;
    grants
        .set_direct(c.admin, super::MANAGER, ASSESSMENTS_SUBMIT, Some(true))
        .await?;
    users
        .unlink(c.admin, super::STAFF)
        .await
        .map_err(|_| Error::Internal)?;
    refuses(
        "old scope after account unlink",
        skills.standing(&staff, company).await,
        |e| matches!(e, SkillsError::Unavailable),
    )?;
    users
        .link_affiliation(
            c.admin,
            super::STAFF,
            AffiliationId::new(staff_employment.affiliation_id).map_err(|_| Error::Invalid)?,
        )
        .await
        .map_err(|_| Error::Internal)?;
    // Reads are tenant/company bound; a filtered register cannot expose a sibling.
    check(
        "out-of-reach reader sees no assessments or gaps",
        saved(
            skills
                .list_assessments(c.viewer, company, &AssessmentFilters::default())
                .await,
        )?
        .is_empty()
            && saved(skills.gaps(c.viewer, company).await)?.is_empty(),
    )?;
    refuses(
        "decision trail hides out-of-reach records",
        skills.assessment_decisions(c.viewer, company, a.id).await,
        |e| matches!(e, SkillsError::NotFound),
    )?;
    refuses(
        "other-tenant company",
        skills
            .list_assessments(
                c.admin,
                super::OTHER_TENANT_COMPANY,
                &AssessmentFilters::default(),
            )
            .await,
        |e| matches!(e, SkillsError::Unauthorized | SkillsError::NotFound),
    )?;
    for (label, sql, id) in [
        (
            "finalized facts cannot change",
            "UPDATE people_skill_assessments SET notes='changed' WHERE id=$1",
            a.id,
        ),
        (
            "finalized assessment cannot be deleted",
            "DELETE FROM people_skill_assessments WHERE id=$1",
            a.id,
        ),
        (
            "decisions cannot be changed",
            "UPDATE people_skill_assessment_decisions SET note='changed' WHERE assessment_id=$1",
            a.id,
        ),
        (
            "decisions cannot be deleted",
            "DELETE FROM people_skill_assessment_decisions WHERE assessment_id=$1",
            a.id,
        ),
        (
            "score must exactly mirror its finalized assessment",
            "UPDATE people_skill_scores SET current_level=current_level+1 WHERE assessment_id=$1",
            same_day.id,
        ),
        (
            "scores cannot be deleted",
            "DELETE FROM people_skill_scores WHERE assessment_id=$1",
            same_day.id,
        ),
    ] {
        sql_refuses(client, label, sql, id).await?;
    }
    // Own the archive-proof company: other slices still need Company 002.
    let companies = services.get::<bilibi_company::CompanyService>(bilibi_company::SERVICE)?;
    let archive_company = match companies
        .create_company(
            c.admin,
            &bilibi_company::NewCompany {
                name: "PK2 archive proof".into(),
                ..bilibi_company::NewCompany::default()
            },
        )
        .await
        .map_err(|_| Error::Internal)?
    {
        bilibi_company::CreateOutcome::Created(company) => company.id,
        bilibi_company::CreateOutcome::Refused(errors) => {
            eprintln!("PK2 archive-proof company: {errors:?}");
            return Err(Error::Internal);
        }
    };
    // The same Skills write must succeed before archiving, so this cannot
    // pass merely because an invalid subject would have been refused anyway.
    let policy_write = PolicyChanges {
        backup_minimum: Some(2),
        ..PolicyChanges::default()
    };
    saved(
        skills
            .put_policy(c.admin, archive_company, &policy_write)
            .await,
    )?;
    companies
        .archive_company(c.admin, archive_company, Some("PK2 archived-company proof"))
        .await
        .map_err(|error| {
            eprintln!("PK2 archive: {error:?}");
            Error::Internal
        })?;
    refuses(
        "archived company refuses the next write",
        skills
            .put_policy(c.admin, archive_company, &policy_write)
            .await,
        |e| {
            matches!(
                e,
                SkillsError::CompanyArchived | SkillsError::NotFound | SkillsError::Unauthorized
            )
        },
    )?;
    let audit=client.query_one("SELECT count(*)::bigint FROM base_audit_mutations WHERE tenant_id=$1 AND auditable_type='people_skill_score'",&[&super::TENANT]).await?;
    check(
        "score projection changes are audited",
        audit.get::<_, i64>(0) > 0,
    )?;
    println!("PK2 walkthrough complete; assessment register, standing, gaps, coverage and database refusals proved.");
    Ok(())
}
