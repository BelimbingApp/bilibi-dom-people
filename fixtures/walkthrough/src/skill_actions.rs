//! PK3: facade and database proofs, using the disposable People walkthrough.
//! Bilimbi's reassessment and action flows, including PK3 automatic closure.
use bilibi_affiliation::AffiliationId;
use bilibi_authz::grants::Grants;
use bilibi_database::{Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_skills::{
    ActionGroup, ActionInput, ActionPlan, ActionSource, Assessment, AssessmentInput, NewActionType,
    PolicyChanges, ReviewDecision, Skills, SkillsError, ACTIONS_APPROVE, ACTIONS_MANAGE,
    ACTIONS_UPDATE, ACTIONS_VIEW, ASSESSMENTS_APPROVE, ASSESSMENTS_MANAGE, ASSESSMENTS_REVIEW,
    ASSESSMENTS_SUBMIT, REASSESSMENTS_EXECUTE, REASSESSMENTS_SUBMIT,
};
use bilibi_people_workforce::Workforce;
use bilibi_tenancy::{Authentication, Scope};
use bilibi_user::UserService;
use jiff::civil::Date;
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
        eprintln!("PK3 unexpected refusal: {error:?}");
        Error::Internal
    })
}
fn check(label: &str, condition: bool) -> Result<()> {
    if !condition {
        eprintln!("PK3 failed: {label}");
        return Err(Error::Internal);
    }
    println!("PK3 proved: {label}");
    Ok(())
}
fn refuses<T: Debug>(
    label: &str,
    result: bilibi_people_skills::Result<T>,
    expected: fn(&SkillsError) -> bool,
) -> Result<()> {
    match result {
        Err(ref error) if expected(error) => {
            println!("PK3 refused: {label}: {error}");
            Ok(())
        }
        other => {
            eprintln!("PK3 unexpected outcome for {label}: {other:?}");
            Err(Error::Internal)
        }
    }
}
async fn finalize(skills: &Skills, scope: &Scope, a: &Assessment) -> Result<Assessment> {
    saved(
        skills
            .review_assessment(scope, super::COMPANY, a.id, ReviewDecision::Verify, None)
            .await,
    )?;
    saved(
        skills
            .finalize_assessment(scope, super::COMPANY, a.id)
            .await,
    )
}
fn today() -> Date {
    jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date()
}

pub(crate) async fn run(services: &Services, client: &Client, c: Context<'_>) -> Result<()> {
    let company = super::COMPANY;
    let skills = services.get::<Skills>(bilibi_people_skills::SERVICE)?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    let workforce = services.get::<Workforce>(bilibi_people_workforce::SERVICE)?;
    let users = services.get::<UserService>("core/user.accounts")?;
    let auth = services.get::<Authentication>("base/tenancy.authentication")?;
    println!("\n== PK3: reassessments and development actions");
    for capability in [
        ACTIONS_MANAGE,
        ACTIONS_APPROVE,
        ACTIONS_VIEW,
        REASSESSMENTS_SUBMIT,
        ASSESSMENTS_SUBMIT,
    ] {
        grants
            .set_direct(c.admin, super::MANAGER, capability, Some(true))
            .await?;
    }
    for capability in [
        ACTIONS_UPDATE,
        ACTIONS_APPROVE,
        ACTIONS_VIEW,
        REASSESSMENTS_SUBMIT,
        REASSESSMENTS_EXECUTE,
        ASSESSMENTS_SUBMIT,
    ] {
        grants
            .set_direct(c.admin, super::STAFF, capability, Some(true))
            .await?;
    }
    // Remove PK2's company-wide assessment reach for the performer and the
    // viewer. Execute deliberately bypasses the supervisor reach check.
    grants
        .set_direct(c.admin, super::STAFF, ASSESSMENTS_MANAGE, Some(false))
        .await?;
    grants
        .set_direct(c.admin, super::MANAGER, ASSESSMENTS_MANAGE, Some(false))
        .await?;
    for capability in [REASSESSMENTS_EXECUTE, ASSESSMENTS_SUBMIT, ACTIONS_VIEW] {
        grants
            .set_direct(c.admin, super::VIEWER, capability, Some(true))
            .await?;
    }
    let employment = workforce
        .employment(c.admin, company, c.staff_employee)
        .await
        .map_err(|_| Error::Internal)?
        .require_current()
        .map_err(|_| Error::Internal)?;
    users
        .link_affiliation(
            c.admin,
            super::STAFF,
            AffiliationId::new(employment.affiliation_id).map_err(|_| Error::Invalid)?,
        )
        .await
        .map_err(|_| Error::Internal)?;
    let staff = auth
        .authenticate(client, super::TENANT, super::STAFF, "walkthrough".into())
        .await?;
    let profile = saved(skills.requirements(c.admin, company, None, today()).await)?
        .ok_or(Error::Internal)?;
    let item = profile
        .items
        .iter()
        .find(|i| i.required_level > 0)
        .ok_or(Error::Internal)?;
    let base = AssessmentInput {
        employee_id: c.staff_employee,
        skill_id: item.skill_id,
        assessed_level: 0,
        evidence: "PK3 observed work record.".into(),
        request_key: "pk3-initial".into(),
        ..AssessmentInput::default()
    };
    let initial = saved(skills.submit_assessment(c.manager, company, &base).await)?;
    let initial = finalize(&skills, c.admin, &initial).await?;
    let kind = saved(
        skills
            .create_action_type(
                c.admin,
                company,
                &NewActionType {
                    code: "pk3-coaching".into(),
                    name: "Observed practice".into(),
                    requires_provider: true,
                },
            )
            .await,
    )?;
    let types = saved(skills.list_action_types(c.viewer, company).await)?;
    check(
        "action types read as company data",
        types.iter().any(|t| t.id == kind.id),
    )?;
    refuses(
        "duplicate type code",
        skills
            .create_action_type(
                c.admin,
                company,
                &NewActionType {
                    code: kind.code.clone(),
                    name: kind.name.clone(),
                    requires_provider: false,
                },
            )
            .await,
        |e| matches!(e, SkillsError::Invalid(_)),
    )?;
    let plan = ActionPlan {
        action_type_id: kind.id,
        owner_employee_id: c.staff_employee,
        coordinator_employee_id: c.manager_employee,
        provider_name: Some("Named provider".into()),
        objective: "Demonstrate the required skill.".into(),
        intervention: "Practice under observation.".into(),
        expected_evidence: "A retained work record.".into(),
        start_on: Some(today()),
        due_on: Some(
            today()
                .checked_add(jiff::Span::new().days(10))
                .map_err(|_| Error::Internal)?,
        ),
        ..ActionPlan::default()
    };
    let input = ActionInput {
        source: ActionSource::Assessment(initial.id),
        plan: plan.clone(),
        request_key: "pk3-gap".into(),
    };
    refuses(
        "provider required",
        skills
            .propose_action(
                c.manager,
                company,
                &ActionInput {
                    plan: ActionPlan {
                        provider_name: None,
                        ..plan.clone()
                    },
                    ..input.clone()
                },
            )
            .await,
        |e| matches!(e, SkillsError::ProviderRequired),
    )?;
    saved(
        skills
            .set_action_type_active(c.admin, company, kind.id, false)
            .await,
    )?;
    refuses(
        "inactive action type",
        skills.propose_action(c.manager, company, &input).await,
        |e| matches!(e, SkillsError::TypeUnavailable),
    )?;
    saved(
        skills
            .set_action_type_active(c.admin, company, kind.id, true)
            .await,
    )?;
    refuses(
        "named employee outside company",
        skills
            .propose_action(
                c.manager,
                company,
                &ActionInput {
                    plan: ActionPlan {
                        owner_employee_id: i64::MAX,
                        ..plan.clone()
                    },
                    ..input.clone()
                },
            )
            .await,
        |e| matches!(e, SkillsError::EmployeeUnavailable),
    )?;
    let (one, two) = tokio::join!(
        skills.propose_action(c.manager, company, &input),
        skills.propose_action(c.manager, company, &input)
    );
    let action = saved(one)?;
    check(
        "concurrent proposal retries create one action and one event",
        saved(two)?.id == action.id
            && saved(skills.action_events(c.manager, company, action.id).await)?.len() == 1,
    )?;
    check(
        "gap and policy snapshot retained",
        action.source_assessment_id == Some(initial.id)
            && action.gap_at_start == initial.required_level
            && action.priority_score == action.gap_at_start * action.priority_multiplier
            && action.mandatory == initial.mandatory,
    )?;
    refuses(
        "same key changed objective",
        skills
            .propose_action(
                c.manager,
                company,
                &ActionInput {
                    plan: ActionPlan {
                        objective: "Different".into(),
                        ..plan.clone()
                    },
                    ..input.clone()
                },
            )
            .await,
        |e| matches!(e, SkillsError::KeyConflict),
    )?;
    refuses(
        "one action per source assessment",
        skills
            .propose_action(
                c.manager,
                company,
                &ActionInput {
                    request_key: "pk3-second-gap".into(),
                    ..input.clone()
                },
            )
            .await,
        |e| matches!(e, SkillsError::AlreadyProposed),
    )?;
    refuses(
        "proposer cannot approve",
        skills.approve_action(c.manager, company, action.id).await,
        |e| matches!(e, SkillsError::SelfApproval),
    )?;
    refuses(
        "subject cannot approve",
        skills.approve_action(&staff, company, action.id).await,
        |e| matches!(e, SkillsError::SelfApproval),
    )?;
    refuses(
        "unapproved action cannot start",
        skills.start_action(&staff, company, action.id).await,
        |e| matches!(e, SkillsError::InvalidTransition),
    )?;
    let revised = saved(
        skills
            .revise_action(
                c.manager,
                company,
                action.id,
                &ActionPlan {
                    intervention: "Revised practice.".into(),
                    ..plan.clone()
                },
            )
            .await,
    )?;
    check(
        "revision changes the plan, preserves gap snapshot",
        revised.intervention == "Revised practice."
            && revised.priority_score == action.priority_score,
    )?;
    let approved = saved(skills.approve_action(c.admin, company, action.id).await)?;
    check(
        "approval due to start today is not_started",
        approved.status == "not_started",
    )?;
    refuses(
        "approved plan cannot be revised",
        skills
            .revise_action(c.manager, company, action.id, &plan)
            .await,
        |e| matches!(e, SkillsError::NotProposed),
    )?;
    refuses(
        "hold needs reason",
        skills.hold_action(&staff, company, action.id, " ").await,
        |e| matches!(e, SkillsError::ReasonRequired),
    )?;
    saved(skills.start_action(&staff, company, action.id).await)?;
    saved(
        skills
            .hold_action(&staff, company, action.id, "Waiting for material.")
            .await,
    )?;
    saved(skills.start_action(&staff, company, action.id).await)?;
    saved(
        skills
            .comment_action(
                &staff,
                company,
                action.id,
                "Observed progress.",
                Some("Observation reference"),
            )
            .await,
    )?;
    refuses(
        "completion needs evidence",
        skills
            .complete_action(&staff, company, action.id, " ", today())
            .await,
        |e| matches!(e, SkillsError::EvidenceRequired),
    )?;
    refuses(
        "reassessment date cannot be past",
        skills
            .complete_action(
                &staff,
                company,
                action.id,
                "Record",
                today()
                    .checked_sub(jiff::Span::new().days(1))
                    .map_err(|_| Error::Internal)?,
            )
            .await,
        |e| matches!(e, SkillsError::InvalidDate),
    )?;
    saved(
        skills
            .complete_action(
                &staff,
                company,
                action.id,
                "Intervention observed and recorded.",
                today(),
            )
            .await,
    )?;
    let old = saved(
        skills
            .submit_assessment(
                c.manager,
                company,
                &AssessmentInput {
                    assessed_on: Some(
                        today()
                            .checked_sub(jiff::Span::new().days(1))
                            .map_err(|_| Error::Internal)?,
                    ),
                    request_key: "pk3-backdated".into(),
                    ..base.clone()
                },
            )
            .await,
    )?;
    finalize(&skills, c.admin, &old).await?;
    check(
        "backdated finalization cannot close intervention",
        saved(
            skills
                .owned_actions(&staff, company, ActionGroup::Open)
                .await,
        )?
        .iter()
        .any(|a| a.action.id == action.id && a.action.status == "pending_reassessment"),
    )?;
    refuses(
        "explicit link requires later reassessment",
        skills
            .link_action_reassessment(&staff, company, action.id, old.id)
            .await,
        |e| matches!(e, SkillsError::NotAReassessment),
    )?;

    // A request is due after the configured window, not at score due date.
    saved(
        skills
            .put_policy(
                c.admin,
                company,
                &PolicyChanges {
                    reassessment_due_days: Some(7),
                    ..PolicyChanges::default()
                },
            )
            .await,
    )?;
    refuses(
        "requester cannot request self",
        skills
            .request_reassessment(&staff, company, c.staff_employee, item.skill_id, "Again")
            .await,
        |e| matches!(e, SkillsError::SelfRequest),
    )?;
    refuses(
        "request requires a current score",
        skills
            .request_reassessment(
                c.admin,
                company,
                c.manager_employee,
                item.skill_id,
                "First score needed.",
            )
            .await,
        |e| matches!(e, SkillsError::NoScore),
    )?;
    let (one, two) = tokio::join!(
        skills.request_reassessment(
            c.manager,
            company,
            c.staff_employee,
            item.skill_id,
            "Check improvement."
        ),
        skills.request_reassessment(
            c.manager,
            company,
            c.staff_employee,
            item.skill_id,
            "Check improvement."
        )
    );
    let request = match (one, two) {
        (Ok(a), Err(SkillsError::AlreadyOpen)) | (Err(SkillsError::AlreadyOpen), Ok(a)) => a,
        other => {
            eprintln!("PK3 concurrent requests: {other:?}");
            return Err(Error::Internal);
        }
    };
    check(
        "one open request, policy due window",
        request.due_on
            == Some(
                today()
                    .checked_add(jiff::Span::new().days(7))
                    .map_err(|_| Error::Internal)?,
            ),
    )?;
    check(
        "requester sees own requests",
        saved(skills.pending_reassessments(c.manager, company).await)?
            .iter()
            .any(|r| r.request.id == request.id),
    )?;
    refuses(
        "performer cannot assess self",
        skills
            .perform_reassessment(&staff, company, request.id, &base)
            .await,
        |e| matches!(e, SkillsError::SelfAssessment),
    )?;
    grants
        .set_direct(c.admin, super::VIEWER, ASSESSMENTS_SUBMIT, Some(false))
        .await?;
    refuses(
        "perform requires submit too",
        skills
            .perform_reassessment(c.viewer, company, request.id, &base)
            .await,
        |e| matches!(e, SkillsError::Unauthorized),
    )?;
    grants
        .set_direct(c.admin, super::VIEWER, ASSESSMENTS_SUBMIT, Some(true))
        .await?;
    let performed = saved(
        skills
            .perform_reassessment(
                c.viewer,
                company,
                request.id,
                &AssessmentInput {
                    employee_id: i64::MAX,
                    skill_id: i64::MAX,
                    assessed_level: item.required_level,
                    evidence: "Independent post-intervention observation.".into(),
                    request_key: "ignored-input-key".into(),
                    supersedes_assessment_id: Some(i64::MAX),
                    ..AssessmentInput::default()
                },
            )
            .await,
    )?;
    check(
        "performer has company-wide execution; stored request fixes subject and skill",
        performed.request.status == "performed"
            && performed.assessment.status == "pending_review"
            && performed.assessment.employee_id == c.staff_employee
            && performed.assessment.skill_id == item.skill_id
            && performed.assessment.supersedes_assessment_id.is_none(),
    )?;
    check(
        "performance of request leaves action pending",
        saved(
            skills
                .owned_actions(&staff, company, ActionGroup::Open)
                .await,
        )?
        .iter()
        .any(|a| a.action.id == action.id),
    )?;
    finalize(&skills, c.admin, &performed.assessment).await?;
    let closed = saved(
        skills
            .owned_actions(&staff, company, ActionGroup::Closed)
            .await,
    )?;
    check(
        "finalization automatically closes competent action with signed improvement",
        closed.iter().any(|a| {
            a.action.id == action.id
                && a.action.closure == "closed_competent"
                && a.action.post_assessment_id == Some(performed.assessment.id)
                && a.action.improvement == Some(item.required_level)
        }),
    )?;
    refuses(
        "performed request immutable",
        skills
            .cancel_reassessment(c.manager, company, request.id)
            .await,
        |e| matches!(e, SkillsError::NotPending),
    )?;
    refuses(
        "terminal action cannot start",
        skills.start_action(&staff, company, action.id).await,
        |e| matches!(e, SkillsError::InvalidTransition),
    )?;
    saved(
        skills
            .comment_action(&staff, company, action.id, "Result acknowledged.", None)
            .await,
    )?;
    let events = saved(skills.action_events(&staff, company, action.id).await)?;
    check(
        "automatic closure and terminal comment appear in event trail",
        events
            .iter()
            .any(|e| e.event_type == "reassessment_linked" && e.actor_user_id == super::ADMIN)
            && events.last().is_some_and(|e| e.event_type == "commented"),
    )?;
    refuses(
        "competent current assessment is not actionable",
        skills
            .propose_action(
                c.manager,
                company,
                &ActionInput {
                    source: ActionSource::Assessment(performed.assessment.id),
                    request_key: "pk3-competent".into(),
                    plan: plan.clone(),
                },
            )
            .await,
        |e| matches!(e, SkillsError::NotActionable),
    )?;
    refuses(
        "old assessment no longer current",
        skills
            .propose_action(
                c.manager,
                company,
                &ActionInput {
                    request_key: "pk3-stale".into(),
                    ..input.clone()
                },
            )
            .await,
        |e| matches!(e, SkillsError::AssessmentNotCurrent),
    )?;

    let manual = ActionInput {
        source: ActionSource::Manual {
            employee_id: c.staff_employee,
            skill_id: item.skill_id,
            starting_level: 2,
            target_level: 3,
            criticality: "development".into(),
            mandatory: false,
            reason: "Planned development beyond the current requirement.".into(),
        },
        plan: ActionPlan {
            start_on: Some(
                today()
                    .checked_add(jiff::Span::new().days(1))
                    .map_err(|_| Error::Internal)?,
            ),
            ..plan.clone()
        },
        request_key: "pk3-manual".into(),
    };
    let manual_action = saved(skills.propose_action(c.manager, company, &manual).await)?;
    let scheduled = saved(
        skills
            .approve_action(c.admin, company, manual_action.id)
            .await,
    )?;
    check(
        "future plan is scheduled, manual reason retained",
        scheduled.status == "scheduled" && scheduled.manual_reason.is_some(),
    )?;
    grants
        .set_direct(c.admin, super::MANAGER, ACTIONS_MANAGE, Some(false))
        .await?;
    refuses(
        "supervisor may view but cannot progress action they do not own",
        skills
            .start_action(c.manager, company, manual_action.id)
            .await,
        |e| matches!(e, SkillsError::Unauthorized | SkillsError::NotFound),
    )?;
    check(
        "supervisor can read in-reach action",
        saved(
            skills
                .list_actions(c.manager, company, ActionGroup::Open)
                .await,
        )?
        .iter()
        .any(|r| r.action.id == manual_action.id),
    )?;
    check(
        "unlinked viewer sees no out-of-reach actions",
        saved(
            skills
                .list_actions(c.viewer, company, ActionGroup::Open)
                .await,
        )?
        .is_empty(),
    )?;
    refuses(
        "viewer cannot read out-of-reach events",
        skills
            .action_events(c.viewer, company, manual_action.id)
            .await,
        |e| matches!(e, SkillsError::NotFound),
    )?;
    grants
        .set_direct(c.admin, super::MANAGER, ACTIONS_MANAGE, Some(true))
        .await?;
    saved(
        skills
            .complete_action(
                &staff,
                company,
                manual_action.id,
                "Practice completed.",
                today(),
            )
            .await,
    )?;
    let lower = saved(
        skills
            .submit_assessment(
                c.manager,
                company,
                &AssessmentInput {
                    assessed_level: 1,
                    request_key: "pk3-lower".into(),
                    ..base.clone()
                },
            )
            .await,
    )?;
    finalize(&skills, c.admin, &lower).await?;
    check(
        "lower reassessment closes as further action required, negative improvement",
        saved(
            skills
                .owned_actions(&staff, company, ActionGroup::Closed)
                .await,
        )?
        .iter()
        .any(|a| {
            a.action.id == manual_action.id
                && a.action.closure == "further_action_required"
                && a.action.improvement == Some(-1)
        }),
    )?;
    let cancel = saved(
        skills
            .propose_action(
                c.manager,
                company,
                &ActionInput {
                    request_key: "pk3-cancel".into(),
                    ..manual.clone()
                },
            )
            .await,
    )?;
    saved(
        skills
            .cancel_action(&staff, company, cancel.id, "No longer needed.")
            .await,
    )?;
    let request = saved(
        skills
            .request_reassessment(
                c.manager,
                company,
                c.staff_employee,
                item.skill_id,
                "Another request.",
            )
            .await,
    )?;
    saved(
        skills
            .cancel_reassessment(c.viewer, company, request.id)
            .await,
    )?;
    refuses(
        "cancelled request cannot perform",
        skills
            .perform_reassessment(c.viewer, company, request.id, &base)
            .await,
        |e| matches!(e, SkillsError::NotPending),
    )?;

    // Same scope, revoked grant: no mutation and no comment event.
    grants
        .set_direct(c.admin, super::STAFF, ACTIONS_UPDATE, Some(false))
        .await?;
    refuses(
        "revoked action grant",
        skills
            .comment_action(&staff, company, action.id, "After revoke", None)
            .await,
        |e| matches!(e, SkillsError::Unauthorized),
    )?;
    grants
        .set_direct(c.admin, super::STAFF, ACTIONS_UPDATE, Some(true))
        .await?;
    users
        .unlink(c.admin, super::STAFF)
        .await
        .map_err(|_| Error::Internal)?;
    refuses(
        "old owner scope after unlink",
        skills
            .comment_action(&staff, company, action.id, "After unlink", None)
            .await,
        |e| matches!(e, SkillsError::NotFound | SkillsError::Unavailable),
    )?;
    users
        .link_affiliation(
            c.admin,
            super::STAFF,
            AffiliationId::new(employment.affiliation_id).map_err(|_| Error::Invalid)?,
        )
        .await
        .map_err(|_| Error::Internal)?;
    for (label, sql, id) in [
        (
            "event update",
            "UPDATE people_skill_action_events SET comment='rewritten' WHERE action_id=$1",
            action.id,
        ),
        (
            "event deletion",
            "DELETE FROM people_skill_action_events WHERE action_id=$1",
            action.id,
        ),
        (
            "terminal action update",
            "UPDATE people_skill_actions SET objective='rewritten' WHERE id=$1",
            action.id,
        ),
        (
            "terminal request update",
            "UPDATE people_skill_reassessment_requests SET reason='rewritten' WHERE id=$1",
            request.id,
        ),
    ] {
        match client.execute(sql, &[&id]).await {
            Err(e) if e.as_db_error().is_some() => println!("PK3 database refused: {label}"),
            other => {
                eprintln!("PK3 unexpected SQL for {label}: {other:?}");
                return Err(Error::Internal);
            }
        }
    }
    let audit=client.query_one("SELECT count(*)::bigint FROM base_audit_mutations WHERE tenant_id=$1 AND auditable_type IN ('people_skill_action','people_skill_action_event','people_skill_reassessment_request','people_skill_action_type')", &[&super::TENANT]).await?;
    let kinds=client.query_one("SELECT count(DISTINCT auditable_type)::bigint FROM base_audit_mutations WHERE tenant_id=$1 AND auditable_type IN ('people_skill_action','people_skill_action_event','people_skill_reassessment_request','people_skill_action_type')", &[&super::TENANT]).await?;
    check(
        "action, request, type and event mutations audited",
        audit.get::<_, i64>(0) > 20 && kinds.get::<_, i64>(0) == 4,
    )?;
    // Keep separate assessment permissions used by PK2's later read proofs.
    for capability in [ASSESSMENTS_REVIEW, ASSESSMENTS_APPROVE] {
        grants
            .set_direct(c.admin, super::STAFF, capability, Some(true))
            .await?;
    }
    println!("PK3 walkthrough complete: requests, actions, independent approval, automatic closure, reach, revocation, unlink and database guards proved.");
    Ok(())
}
