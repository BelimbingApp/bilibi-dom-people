//! PT3: executable governance proofs against the disposable walkthrough DB.
//! Mirrors Bilimbi Training Governance contracts; no production sample data.
use crate::{ADMIN, COMPANY, MANAGER, STAFF, TENANT, VIEWER};
use bilibi_authz::grants::Grants;
use bilibi_database::{Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_training::*;
use bilibi_people_workforce::Workforce;
use bilibi_tenancy::{Authentication, Scope};
use bilibi_user::UserService;
use std::fmt::Debug;

fn saved<T: Debug>(label: &str, result: bilibi_people_training::Result<T>) -> Result<T> {
    match result {
        Ok(value) => {
            println!("PT3 {label}: {value:?}");
            Ok(value)
        }
        Err(error) => {
            eprintln!("PT3 {label}: unexpected {error:?}");
            Err(Error::Internal)
        }
    }
}
fn refused<T: Debug>(
    label: &str,
    result: bilibi_people_training::Result<T>,
    expected: TrainingError,
) -> Result<()> {
    match result {
        Err(error) if std::mem::discriminant(&error) == std::mem::discriminant(&expected) => {
            println!("PT3 {label}: refused ({error})");
            Ok(())
        }
        other => {
            eprintln!("PT3 {label}: expected {expected:?}, got {other:?}");
            Err(Error::Internal)
        }
    }
}
fn proof(label: &str, holds: bool) -> Result<()> {
    if holds {
        println!("PT3 {label}: proved");
        Ok(())
    } else {
        eprintln!("PT3 {label}: FAILED");
        Err(Error::Internal)
    }
}
fn request(cost: &str) -> NewLearningRequest {
    NewLearningRequest {
        course_id: None,
        need: "A learning need".into(),
        objective: "Build the required capability".into(),
        expected_result: "Demonstrate the skill at work".into(),
        proposed_on: "2026-11-10".into(),
        estimated_cost: cost.into(),
        currency: "ABC".into(),
    }
}
fn budget(amount: &str) -> NewBudgetPolicy {
    NewBudgetPolicy {
        currency: "ABC".into(),
        effective_from: "2026-11-01".into(),
        effective_to: "2026-11-30".into(),
        amount: amount.into(),
        reason: "Allocate the learning period".into(),
    }
}
fn plan() -> NewLearningPlan {
    NewLearningPlan {
        period_start: "2026-11-01".into(),
        period_end: "2026-11-30".into(),
        objectives: "Develop the team".into(),
        reason: "Plan the next learning period".into(),
    }
}
fn item(request_id: Option<i64>) -> NewLearningPlanItem {
    NewLearningPlanItem {
        request_id,
        need: "Team learning need".into(),
        expected_result: "Apply the skill".into(),
        target_cohort: "Current team".into(),
        responsible_owner: "Team manager".into(),
        intended_timing: "During this period".into(),
        evaluation_approach: "Observe the result".into(),
    }
}
async fn pending(
    training: &Training,
    staff: &Scope,
    manager: &Scope,
    admin: &Scope,
    cost: &str,
) -> Result<LearningRequest> {
    let row = saved(
        "create request",
        training
            .create_learning_request(staff, COMPANY, &request(cost))
            .await,
    )?;
    for (scope, action) in [(staff, "submit"), (manager, "recommend"), (admin, "review")] {
        saved(
            action,
            training
                .decide_learning_request(
                    scope,
                    COMPANY,
                    row.id,
                    action,
                    "Proceed with this learning need",
                )
                .await,
        )?;
    }
    Ok(row)
}

pub async fn run(
    services: &Services,
    client: &tokio_postgres::Client,
    admin: &Scope,
    manager: &Scope,
    staff_employee: i64,
) -> Result<()> {
    let training = services.get::<Training>(SERVICE)?;
    let workforce = services.get::<Workforce>(bilibi_people_workforce::SERVICE)?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    let users = services.get::<UserService>("core/user.accounts")?;
    let authentication = services.get::<Authentication>("base/tenancy.authentication")?;
    // Previous slices deliberately unlink this account. Restore through its owner.
    let employment = workforce
        .employment(manager, COMPANY, staff_employee)
        .await
        .map_err(|_| Error::Internal)?
        .require_current()
        .map_err(|_| Error::Internal)?;
    users
        .link_person(
            admin,
            STAFF,
            bilibi_party_ids::PersonId::new(employment.person_id).map_err(|_| Error::Invalid)?,
        )
        .await
        .map_err(|_| Error::Internal)?;
    crate::give_staff_desk(
        &users,
        admin,
        STAFF,
        bilibi_party_ids::PersonId::new(employment.person_id).map_err(|_| Error::Invalid)?,
        bilibi_party_ids::OrganisationId::new(employment.platform_company_id)
            .map_err(|_| Error::Invalid)?,
    )
    .await
    .map_err(|_| Error::Internal)?;
    for user in [MANAGER, STAFF] {
        for capability in [
            REQUESTS_SUBMIT,
            REQUESTS_RECOMMEND,
            REQUESTS_REVIEW,
            REQUESTS_APPROVE,
            REQUESTS_VIEW,
            PLANS_SUBMIT,
            PLANS_APPROVE,
            PLANS_VIEW,
            BUDGETS_VIEW,
            BUDGETS_MANAGE,
        ] {
            grants
                .set_direct(admin, user, capability, Some(true))
                .await?;
        }
    }
    let staff = authentication
        .authenticate(client, TENANT, STAFF, "walkthrough".into())
        .await?;
    let viewer = authentication
        .authenticate(client, TENANT, VIEWER, "walkthrough".into())
        .await?;
    refused(
        "viewer budget access",
        training.learning_budgets(&viewer, COMPANY).await,
        TrainingError::Unauthorized,
    )?;
    refused(
        "malformed currencies",
        training
            .put_learning_currencies(admin, COMPANY, &["bad".into()])
            .await,
        TrainingError::InvalidCurrencies,
    )?;
    let currencies = saved(
        "currencies deduplicate",
        training
            .put_learning_currencies(admin, COMPANY, &["ABC".into(), "ABC".into()])
            .await,
    )?;
    proof("one enabled currency", currencies == ["ABC"])?;
    let allocation = saved(
        "initial allocation",
        training
            .create_budget_policy(admin, COMPANY, &budget("100.0000"))
            .await,
    )?;
    refused(
        "overlapping policy",
        training
            .create_budget_policy(admin, COMPANY, &budget("100"))
            .await,
        TrainingError::OverlappingPolicy,
    )?;
    refused(
        "more than four decimal places",
        training
            .create_learning_request(&staff, COMPANY, &request("0.00001"))
            .await,
        TrainingError::InvalidLearningValues,
    )?;
    refused(
        "money upper bound",
        training
            .create_learning_request(&staff, COMPANY, &request("100000000000000"))
            .await,
        TrainingError::InvalidLearningValues,
    )?;
    refused(
        "negative cost",
        training
            .create_learning_request(&staff, COMPANY, &request("-1"))
            .await,
        TrainingError::InvalidLearningValues,
    )?;
    let draft = saved(
        "staff draft",
        training
            .create_learning_request(&staff, COMPANY, &request("40.0000"))
            .await,
    )?;
    for (scope, audience) in [
        (manager, LearningAudience::Team),
        (admin, LearningAudience::Hr),
    ] {
        proof(
            "draft hidden from register",
            saved(
                "register",
                training.learning_requests(scope, COMPANY, audience).await,
            )?
            .iter()
            .all(|r| r.id != draft.id),
        )?;
        proof(
            "draft history hidden",
            saved(
                "histories",
                training
                    .learning_histories(
                        scope,
                        COMPANY,
                        LearningKind::Request,
                        &[draft.id],
                        audience,
                    )
                    .await,
            )?
            .is_empty(),
        )?;
    }
    refused(
        "blank decision reason",
        training
            .decide_learning_request(&staff, COMPANY, draft.id, "submit", "  ")
            .await,
        TrainingError::ReasonRequired,
    )?;
    refused(
        "skip stages",
        training
            .decide_learning_request(admin, COMPANY, draft.id, "approve", "Skip")
            .await,
        TrainingError::InvalidTransition,
    )?;
    saved(
        "submit",
        training
            .decide_learning_request(&staff, COMPANY, draft.id, "submit", "Ready")
            .await,
    )?;
    refused(
        "non-owner cancellation",
        training
            .decide_learning_request(manager, COMPANY, draft.id, "cancel", "Cancel")
            .await,
        TrainingError::NotOwner,
    )?;
    refused(
        "not in own team",
        training
            .decide_learning_request(&staff, COMPANY, draft.id, "recommend", "Recommend")
            .await,
        TrainingError::OutsideTeam,
    )?;
    saved(
        "manager recommendation",
        training
            .decide_learning_request(manager, COMPANY, draft.id, "recommend", "Relevant")
            .await,
    )?;
    refused(
        "author self review",
        training
            .decide_learning_request(&staff, COMPANY, draft.id, "review", "Review")
            .await,
        TrainingError::SelfApproval,
    )?;
    // A second account linked to the subject is also refused, regardless of author ID.
    users
        .link_person(
            admin,
            VIEWER,
            bilibi_party_ids::PersonId::new(employment.person_id).map_err(|_| Error::Invalid)?,
        )
        .await
        .map_err(|_| Error::Internal)?;
    crate::give_staff_desk(
        &users,
        admin,
        VIEWER,
        bilibi_party_ids::PersonId::new(employment.person_id).map_err(|_| Error::Invalid)?,
        bilibi_party_ids::OrganisationId::new(employment.platform_company_id)
            .map_err(|_| Error::Invalid)?,
    )
    .await
    .map_err(|_| Error::Internal)?;
    grants
        .set_direct(admin, VIEWER, REQUESTS_REVIEW, Some(true))
        .await?;
    let subject_account = authentication
        .authenticate(client, TENANT, VIEWER, "walkthrough".into())
        .await?;
    refused(
        "subject self review through another account",
        training
            .decide_learning_request(&subject_account, COMPANY, draft.id, "review", "Review")
            .await,
        TrainingError::SelfApproval,
    )?;
    users
        .unlink(admin, VIEWER)
        .await
        .map_err(|_| Error::Internal)?;
    grants
        .set_direct(admin, VIEWER, REQUESTS_REVIEW, None)
        .await?;
    saved(
        "HR review",
        training
            .decide_learning_request(admin, COMPANY, draft.id, "review", "Suitable")
            .await,
    )?;
    let approved = saved(
        "approval snapshots policy and cost",
        training
            .decide_learning_request(admin, COMPANY, draft.id, "approve", "Funded")
            .await,
    )?;
    proof(
        "approval snapshot",
        approved.budget_policy_id == Some(allocation.id)
            && approved.approved_cost.as_deref() == Some("40.0000"),
    )?;
    refused(
        "terminal request",
        training
            .decide_learning_request(&staff, COMPANY, draft.id, "cancel", "Cancel")
            .await,
        TrainingError::InvalidTransition,
    )?;
    let histories = saved(
        "submitted request histories",
        training
            .learning_histories(
                admin,
                COMPANY,
                LearningKind::Request,
                &[draft.id],
                LearningAudience::Hr,
            )
            .await,
    )?;
    proof(
        "all five request decisions",
        histories
            .get(&draft.id)
            .is_some_and(|d| d.len() == 5 && d[0].action == "create" && d[4].action == "approve"),
    )?;
    refused(
        "allocation below commitments",
        training
            .supersede_budget_policy(admin, COMPANY, allocation.id, &budget("39.9999"))
            .await,
        TrainingError::BudgetBelowCommitments,
    )?;
    let mut narrowed = budget("100");
    narrowed.effective_from = "2026-11-11".into();
    refused(
        "corrected period excludes commitment",
        training
            .supersede_budget_policy(admin, COMPANY, allocation.id, &narrowed)
            .await,
        TrainingError::CommitmentsOutsidePeriod,
    )?;
    let corrected = saved(
        "correct budget",
        training
            .supersede_budget_policy(admin, COMPANY, allocation.id, &budget("100"))
            .await,
    )?;
    refused(
        "superseded budget",
        training
            .supersede_budget_policy(admin, COMPANY, allocation.id, &budget("100"))
            .await,
        TrainingError::SupersededBudget,
    )?;
    let p1 = pending(&training, &staff, manager, admin, "40").await?;
    let p2 = pending(&training, &staff, manager, admin, "40").await?;
    let (first, second) = tokio::join!(
        training.decide_learning_request(admin, COMPANY, p1.id, "approve", "Funded"),
        training.decide_learning_request(admin, COMPANY, p2.id, "approve", "Funded")
    );
    let accepted = match (first, second) {
        (Ok(row), Err(TrainingError::BudgetExceeded))
        | (Err(TrainingError::BudgetExceeded), Ok(row)) => row,
        other => {
            eprintln!("PT3 approval race: {other:?}");
            return Err(Error::Internal);
        }
    };
    proof(
        "concurrent approvals cannot overspend",
        accepted.budget_policy_id == Some(corrected.id),
    )?;
    let totals = saved(
        "budget totals",
        training.learning_budgets(admin, COMPANY).await,
    )?;
    proof(
        "allocated committed remaining",
        totals.iter().any(|r| {
            r.policy.id == corrected.id
                && r.policy.amount == "100.0000"
                && r.committed == "80.0000"
                && r.remaining == "20.0000"
        }),
    )?;
    let cancel = saved(
        "cancellable draft",
        training
            .create_learning_request(&staff, COMPANY, &request("1"))
            .await,
    )?;
    saved(
        "cancel draft",
        training
            .decide_learning_request(&staff, COMPANY, cancel.id, "cancel", "No longer needed")
            .await,
    )?;
    let rejected = saved(
        "rejection draft",
        training
            .create_learning_request(&staff, COMPANY, &request("1"))
            .await,
    )?;
    saved(
        "submit for rejection",
        training
            .decide_learning_request(&staff, COMPANY, rejected.id, "submit", "Ready")
            .await,
    )?;
    saved(
        "reject at HOD",
        training
            .decide_learning_request(
                manager,
                COMPANY,
                rejected.id,
                "reject",
                "Different learning is needed",
            )
            .await,
    )?;
    // Owner/team restrictions and two competing amendments.
    refused(
        "empty plan",
        training
            .create_learning_plan(manager, COMPANY, &plan(), &[])
            .await,
        TrainingError::ItemsRequired,
    )?;
    refused(
        "unapproved linked request",
        training
            .create_learning_plan(manager, COMPANY, &plan(), &[item(Some(rejected.id))])
            .await,
        TrainingError::RequestUnapproved,
    )?;
    let team_plan = saved(
        "create team plan",
        training
            .create_learning_plan(
                manager,
                COMPANY,
                &plan(),
                &[item(Some(draft.id)), item(None)],
            )
            .await,
    )?;
    proof("two immutable items", team_plan.items.len() == 2)?;
    proof(
        "HR cannot read plan draft",
        saved(
            "HR plans",
            training
                .learning_plans(admin, COMPANY, LearningAudience::Hr)
                .await,
        )?
        .iter()
        .all(|r| r.id != team_plan.id),
    )?;
    saved(
        "submit plan",
        training
            .decide_learning_plan(manager, COMPANY, team_plan.id, "submit", "Ready")
            .await,
    )?;
    refused(
        "plan author self approval",
        training
            .decide_learning_plan(manager, COMPANY, team_plan.id, "approve", "Approved")
            .await,
        TrainingError::SelfApproval,
    )?;
    saved(
        "approve plan",
        training
            .decide_learning_plan(admin, COMPANY, team_plan.id, "approve", "Approved")
            .await,
    )?;
    let amendment = saved(
        "amend plan",
        training
            .amend_learning_plan(manager, COMPANY, team_plan.id, &plan(), &[item(None)])
            .await,
    )?;
    let rival = saved(
        "competing amendment",
        training
            .amend_learning_plan(manager, COMPANY, team_plan.id, &plan(), &[item(None)])
            .await,
    )?;
    proof(
        "consecutive amendment versions",
        amendment.version == 2 && rival.version == 3 && amendment.plan_key == rival.plan_key,
    )?;
    for id in [amendment.id, rival.id] {
        saved(
            "submit amendment",
            training
                .decide_learning_plan(manager, COMPANY, id, "submit", "Amend")
                .await,
        )?;
    }
    saved(
        "approve amendment",
        training
            .decide_learning_plan(admin, COMPANY, amendment.id, "approve", "Replace")
            .await,
    )?;
    refused(
        "rival amendment cannot supersede twice",
        training
            .decide_learning_plan(admin, COMPANY, rival.id, "approve", "Replace")
            .await,
        TrainingError::SupersededPlan,
    )?;
    let plans = saved(
        "plan history versions",
        training
            .learning_plans(manager, COMPANY, LearningAudience::Team)
            .await,
    )?;
    proof(
        "prior superseded, approved successor intact",
        plans
            .iter()
            .any(|p| p.id == team_plan.id && p.status == "superseded")
            && plans
                .iter()
                .any(|p| p.id == amendment.id && p.status == "approved"),
    )?;
    let history = saved(
        "supersession history",
        training
            .learning_histories(
                admin,
                COMPANY,
                LearningKind::Plan,
                &[team_plan.id],
                LearningAudience::Hr,
            )
            .await,
    )?;
    proof(
        "supersede decision retained",
        history[&team_plan.id]
            .last()
            .is_some_and(|d| d.action == "supersede"),
    )?;
    let revoked = saved(
        "draft before grant revocation",
        training
            .create_learning_request(&staff, COMPANY, &request("1"))
            .await,
    )?;
    grants
        .set_direct(admin, STAFF, REQUESTS_SUBMIT, Some(false))
        .await?;
    refused(
        "stale scope loses revoked grant",
        training
            .decide_learning_request(&staff, COMPANY, revoked.id, "submit", "Ready")
            .await,
        TrainingError::Unauthorized,
    )?;
    grants
        .set_direct(admin, STAFF, REQUESTS_SUBMIT, Some(true))
        .await?;
    users
        .unlink(admin, STAFF)
        .await
        .map_err(|_| Error::Internal)?;
    // Keep company reach so the refusal specifically proves the current employment link.
    grants
        .set_direct(admin, STAFF, "admin.company.tenant-wide.manage", Some(true))
        .await?;
    refused(
        "unlinked account cannot act for former employee",
        training
            .decide_learning_request(&staff, COMPANY, revoked.id, "submit", "Ready")
            .await,
        TrainingError::EmployeeUnavailable,
    )?;
    // Database refusals use rollback-only transactions, leaving all facade facts intact.
    for (label, sql, id) in [
        (
            "budget immutable",
            "UPDATE people_training_budget_policies SET amount=0 WHERE id=$1",
            allocation.id,
        ),
        (
            "decision append only",
            "DELETE FROM people_training_need_decisions WHERE request_id=$1",
            draft.id,
        ),
        (
            "item immutable",
            "UPDATE people_training_team_plan_items SET need='Rewrite' WHERE plan_id=$1",
            team_plan.id,
        ),
        (
            "request content immutable",
            "UPDATE people_training_needs SET need='Rewrite' WHERE id=$1",
            draft.id,
        ),
        (
            "terminal request immutable",
            "UPDATE people_training_needs SET status='draft' WHERE id=$1",
            draft.id,
        ),
        (
            "plan content immutable",
            "UPDATE people_training_team_plans SET objectives='Rewrite' WHERE id=$1",
            team_plan.id,
        ),
        (
            "budget overlap SQL",
            "INSERT INTO people_training_budget_policies(tenant_id,company_id,actor_user_id,currency,effective_from,effective_to,amount,reason) SELECT tenant_id,company_id,actor_user_id,currency,effective_from,effective_to,amount,'Overlap' FROM people_training_budget_policies WHERE id=$1",
            corrected.id,
        ),
    ] {
        client.batch_execute("BEGIN").await?;
        let result = client.execute(sql, &[&id]).await;
        client.batch_execute("ROLLBACK").await?;
        proof(
            label,
            result
                .as_ref()
                .err()
                .and_then(|e| e.code())
                .is_some_and(|c| c.code() == "23514"),
        )?;
    }
    proof("admin fixture identity", admin.user() == ADMIN)?;
    println!("PT3 governance proof complete.");
    Ok(())
}

walkthrough_step!(
    "training_governance",
    after = ["participation"],
    |ctx| async move {
        let client = ctx.db.get().await?;
        // PT3: requests, team plans, immutable budgets and audience-filtered decisions.
        crate::training_governance::run(
            &ctx.services,
            &client,
            &ctx.admin,
            &ctx.manager,
            ctx.staff_employment.id,
        )
        .await?;

        Ok(())
    }
);
