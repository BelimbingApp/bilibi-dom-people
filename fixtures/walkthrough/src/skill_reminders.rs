//! PK7 executable proof: real facade, recipient notifications and Queue executor.
use super::{ADMIN, COMPANY, MANAGER, STAFF, TENANT, VIEWER};
use bilibi_authz::grants::Grants;
use bilibi_database::{pool, Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_skills::{
    ActionInput, ActionPlan, ActionSource, AssessmentInput, NewActionType, PolicyChanges,
    ReviewDecision, Skills, SkillsError, ASSESSMENTS_MANAGE, REMINDERS_SEND,
};
use bilibi_people_workforce::{Workforce, EMPLOYMENT_VIEW};
use bilibi_queue::{Config, Executor};
use bilibi_tenancy::Authentication;
use bilibi_user::UserService;
use jiff::civil::Date;
use std::{sync::Arc, time::Duration};

fn must<T>(result: bilibi_people_skills::Result<T>) -> Result<T> {
    result.map_err(|error| {
        eprintln!("PK7 unexpected: {error:?}");
        Error::Internal
    })
}
fn check(label: &str, value: bool) -> Result<()> {
    if !value {
        eprintln!("PK7 FAILED: {label}");
        return Err(Error::Internal);
    }
    println!("PK7 proved: {label}");
    Ok(())
}
async fn count(client: &tokio_postgres::Client, sql: &str) -> Result<i64> {
    Ok(client.query_one(sql, &[]).await?.get(0))
}

pub async fn run(url: &str) -> Result<()> {
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = pool(url)?;
    let mut services = Services::default();
    services.provide("bootstrap.database", Arc::new(db.clone()))?;
    services.provide("bootstrap.composition", Arc::new(bilibi_web::composition()))?;
    services.install(&bilibi_module_registry::discover()?)?;
    let services = Arc::new(services);
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
    let workforce = services.get::<Workforce>(bilibi_people_workforce::SERVICE)?;
    let skills = services.get::<Skills>(bilibi_people_skills::SERVICE)?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    let users = services.get::<UserService>("core/user.accounts")?;
    let employee: i64 = client.query_one("SELECT id FROM people_workforce_employments WHERE tenant_id=1 AND company_id=1 AND employee_number='E-002'",&[]).await?.get(0);
    let employment = workforce
        .employment(&admin, COMPANY, employee)
        .await
        .map_err(|_| Error::Internal)?
        .require_current()
        .map_err(|_| Error::Internal)?;
    crate::give_staff_desk(
        &users,
        &admin,
        STAFF,
        bilibi_party_ids::PersonId::new(employment.person_id).map_err(|_| Error::Invalid)?,
        bilibi_party_ids::OrganisationId::new(employment.platform_company_id)
            .map_err(|_| Error::Invalid)?,
    )
    .await
    .map_err(|_| Error::Internal)?;
    let staff = auth
        .authenticate(&**client, TENANT, STAFF, "walkthrough".into())
        .await?;
    grants
        .set_direct(&admin, MANAGER, REMINDERS_SEND, Some(true))
        .await?;
    grants
        .set_direct(&admin, MANAGER, ASSESSMENTS_MANAGE, Some(true))
        .await?;
    // This sender must not need the employment-page permission.
    grants
        .set_direct(&admin, MANAGER, EMPLOYMENT_VIEW, Some(false))
        .await?;
    must(
        skills
            .put_policy(
                &admin,
                COMPANY,
                &PolicyChanges {
                    reminder_window_days: Some(30),
                    backup_minimum: Some(100),
                    ..Default::default()
                },
            )
            .await,
    )?;
    let today = jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date();
    let tomorrow = today
        .checked_add(jiff::Span::new().days(1))
        .map_err(|_| Error::Invalid)?;
    let profile =
        must(skills.requirements(&admin, COMPANY, None, today).await)?.ok_or(Error::Internal)?;
    let requirement = profile.items.first().ok_or(Error::Internal)?;
    let assessment = must(
        skills
            .submit_assessment(
                &manager,
                COMPANY,
                &AssessmentInput {
                    employee_id: employee,
                    skill_id: requirement.skill_id,
                    assessed_level: 0,
                    evidence: "PK7 observed skill evidence".into(),
                    assessed_on: Some(today),
                    valid_until: Some(today),
                    request_key: "pk7-score".into(),
                    ..Default::default()
                },
            )
            .await,
    )?;
    must(
        skills
            .review_assessment(&admin, COMPANY, assessment.id, ReviewDecision::Verify, None)
            .await,
    )?;
    must(
        skills
            .finalize_assessment(&admin, COMPANY, assessment.id)
            .await,
    )?;
    let action_type = must(
        skills
            .create_action_type(
                &admin,
                COMPANY,
                &NewActionType {
                    code: "pk7-intervention".into(),
                    name: "Reminder intervention".into(),
                    requires_provider: false,
                },
            )
            .await,
    )?;
    let manager_employee = workforce
        .linked_employee(&**client, &manager, COMPANY)
        .await
        .map_err(|_| Error::Internal)?
        .ok_or(Error::Internal)?;
    let action = must(
        skills
            .propose_action(
                &manager,
                COMPANY,
                &ActionInput {
                    source: ActionSource::Manual {
                        employee_id: employee,
                        skill_id: requirement.skill_id,
                        starting_level: 0,
                        target_level: 1,
                        criticality: "critical".into(),
                        mandatory: true,
                        reason: "PK7 reminder proof".into(),
                    },
                    plan: ActionPlan {
                        action_type_id: action_type.id,
                        owner_employee_id: manager_employee,
                        coordinator_employee_id: manager_employee,
                        objective: "Practice the requirement".into(),
                        intervention: "Observed practice".into(),
                        expected_evidence: "Work record".into(),
                        start_on: Some(today),
                        due_on: Some(today),
                        ..Default::default()
                    },
                    request_key: "pk7-action".into(),
                },
            )
            .await,
    )?;
    must(skills.approve_action(&admin, COMPANY, action.id).await)?;
    let due = must(
        skills
            .due_reminders(&manager, COMPANY, Some(tomorrow))
            .await,
    )?;
    for rule in [
        "overdue_reassessment",
        "expiring_certificate",
        "overdue_action",
        "coverage_gap",
    ] {
        check(
            &format!("due rule {rule}"),
            due.iter().any(|r| r.rule == rule),
        )?;
    }
    let validity = due
        .iter()
        .find(|r| r.rule == "expiring_certificate" && r.employee_id == Some(employee))
        .ok_or(Error::Internal)?;
    check(
        "validity reaches employee and immediate supervisor",
        validity.recipients.contains(&MANAGER) && validity.recipients.contains(&STAFF),
    )?;
    check(
        "read preview writes no ledger",
        count(&client, "SELECT count(*) FROM people_skill_reminders").await? == 0,
    )?;
    check(
        "ungranted sender refused",
        matches!(
            skills.issue_reminders(&viewer, COMPANY, None).await,
            Err(SkillsError::Unauthorized)
        ),
    )?;
    check(
        "foreign company refused",
        skills.due_reminders(&manager, 100, None).await.is_err(),
    )?;

    // Deliberate notification failure: business rows still come from the facade.
    client.batch_execute("CREATE FUNCTION pg_temp.pk7_refuse_notice() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'PK7 injected notification failure'; END $$; CREATE TRIGGER pk7_notice_failure BEFORE INSERT ON notifications FOR EACH ROW EXECUTE FUNCTION pg_temp.pk7_refuse_notice();").await?;
    let failed = must(
        skills
            .issue_reminders(&manager, COMPANY, Some(tomorrow))
            .await,
    )?;
    check(
        "failed notifications retain failed ledger claims",
        failed.failed > 0
            && failed.sent == 0
            && count(
                &client,
                "SELECT count(*) FROM notifications WHERE type='people.skills.reminder'",
            )
            .await?
                == 0,
    )?;
    client
        .batch_execute("DROP TRIGGER pk7_notice_failure ON notifications")
        .await?;
    let recovered = must(
        skills
            .retry_reminders(&manager, COMPANY, Some(tomorrow))
            .await,
    )?;
    check(
        "failed rows recover once",
        recovered.sent == failed.failed && recovered.failed == 0,
    )?;
    check(
        "second retry ignores sent rows",
        must(
            skills
                .retry_reminders(&manager, COMPANY, Some(tomorrow))
                .await,
        )?
        .sent
            == 0,
    )?;
    let repeated = must(
        skills
            .issue_reminders(&manager, COMPANY, Some(tomorrow))
            .await,
    )?;
    check(
        "same-period replay skips every addressed item",
        repeated.sent == 0 && repeated.failed == 0 && repeated.skipped == recovered.sent,
    )?;
    check(
        "exactly one recipient notification per sent row",
        count(
            &client,
            "SELECT count(*) FROM notifications WHERE type='people.skills.reminder'",
        )
        .await?
            == count(
                &client,
                "SELECT count(*) FROM people_skill_reminders WHERE state='sent'",
            )
            .await?,
    )?;
    let inbox = must(skills.reminder_inbox(&staff, COMPANY, 50).await)?;
    check(
        "inbox is recipient-scoped",
        !inbox.is_empty()
            && inbox
                .iter()
                .all(|r| r.recipient_user_id == STAFF && r.state == "sent"),
    )?;
    let sent_id = inbox[0].id;
    check(
        "sent ledger rows are immutable",
        client
            .execute(
                "UPDATE people_skill_reminders SET state='failed' WHERE id=$1",
                &[&sent_id],
            )
            .await
            .is_err(),
    )?;
    check(
        "ledger rows cannot be deleted",
        client
            .execute(
                "DELETE FROM people_skill_reminders WHERE id=$1",
                &[&sent_id],
            )
            .await
            .is_err(),
    )?;
    let next_week = tomorrow
        .checked_add(jiff::Span::new().days(7))
        .map_err(|_| Error::Invalid)?;
    let (a, b) = tokio::join!(
        skills.issue_reminders(&manager, COMPANY, Some(next_week)),
        skills.issue_reminders(&manager, COMPANY, Some(next_week))
    );
    let (a, b) = (must(a)?, must(b)?);
    check("concurrent issuers create no duplicate period keys",a.sent+b.sent>0&&count(&client,"SELECT count(*) FROM (SELECT company_id,rule,employee_id,skill_id,action_id,period_key,recipient_user_id FROM people_skill_reminders GROUP BY 1,2,3,4,5,6,7 HAVING count(*)>1) d").await?==0)?;

    // Drop a sender after its claim commits and while notification SQL waits.
    let stalled_day = tomorrow
        .checked_add(jiff::Span::new().days(14))
        .map_err(|_| Error::Invalid)?;
    client.batch_execute("CREATE FUNCTION pg_temp.pk7_delay_notice() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_sleep(3); RETURN NEW; END $$; CREATE TRIGGER pk7_notice_delay BEFORE INSERT ON notifications FOR EACH ROW EXECUTE FUNCTION pg_temp.pk7_delay_notice();").await?;
    let task = tokio::spawn({
        let skills = skills.clone();
        let manager = manager.clone();
        async move {
            skills
                .issue_reminders(&manager, COMPANY, Some(stalled_day))
                .await
        }
    });
    let mut pending = false;
    for _ in 0..100 {
        if count(
            &client,
            "SELECT count(*) FROM people_skill_reminders WHERE state='pending'",
        )
        .await?
            > 0
        {
            pending = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    check(
        "committed pending claim survives interrupted sender",
        pending,
    )?;
    task.abort();
    let _ = task.await;
    client
        .batch_execute("DROP TRIGGER pk7_notice_delay ON notifications")
        .await?;
    check(
        "fresh pending rows are not retried",
        must(
            skills
                .retry_reminders(&manager, COMPANY, Some(stalled_day))
                .await,
        )?
        .sent
            == 0,
    )?;
    // Age only the interrupted claim, simulating the documented grace period.
    client.execute("UPDATE people_skill_reminders SET inserted_at=(now() AT TIME ZONE 'UTC')-interval '16 minutes' WHERE state='pending'",&[]).await?;
    check(
        "old pending row excluded from another period",
        must(
            skills
                .retry_reminders(&manager, COMPANY, Some(tomorrow))
                .await,
        )?
        .sent
            == 0,
    )?;
    let recipient: i64 = client
        .query_one(
            "SELECT recipient_user_id FROM people_skill_reminders WHERE state='pending'",
            &[],
        )
        .await?
        .get(0);
    let recipient_account = users.company_account(&admin, COMPANY, recipient).await?;
    users
        .unlink(&admin, recipient)
        .await
        .map_err(|_| Error::Internal)?;
    let absent = must(
        skills
            .retry_reminders(&admin, COMPANY, Some(stalled_day))
            .await,
    )?;
    check("missing recipient is a visible failed row", absent.failed == 1 && count(&client,"SELECT count(*) FROM people_skill_reminders WHERE state='failed' AND failure='recipient not found'").await? == 1)?;
    crate::give_staff_desk(
        &users,
        &admin,
        recipient,
        recipient_account.person,
        recipient_account.organisation,
    )
    .await
    .map_err(|_| Error::Internal)?;
    check(
        "stalled pending claim recovers once after recipient returns",
        must(
            skills
                .retry_reminders(&manager, COMPANY, Some(stalled_day))
                .await,
        )?
        .sent
            == 1,
    )?;
    // Complete the period, then remove every linked manager from its audience.
    must(
        skills
            .issue_reminders(&manager, COMPANY, Some(stalled_day))
            .await,
    )?;
    grants
        .set_direct(&admin, MANAGER, ASSESSMENTS_MANAGE, Some(false))
        .await?;
    let gaps = must(
        skills
            .due_reminders(&manager, COMPANY, Some(tomorrow))
            .await,
    )?;
    check(
        "coverage audience honors direct denial",
        gaps.iter()
            .filter(|r| r.rule == "coverage_gap")
            .all(|r| !r.recipients.contains(&MANAGER)),
    )?;
    users
        .unlink(&admin, STAFF)
        .await
        .map_err(|_| Error::Internal)?;
    let unlinked = must(
        skills
            .due_reminders(&manager, COMPANY, Some(tomorrow))
            .await,
    )?;
    check(
        "unlinked employee account leaves recipient set",
        unlinked.iter().all(|r| !r.recipients.contains(&STAFF)),
    )?;
    let current = must(
        skills
            .issue_reminders(&manager, COMPANY, Some(tomorrow))
            .await,
    )?;
    check("unaddressed due items are counted", current.unaddressed > 0)?;

    // ISO week-year boundary: January 1, 2027 belongs to ISO year 2026.
    must(
        skills
            .issue_reminders(
                &manager,
                COMPANY,
                Some("2027-01-01".parse::<Date>().map_err(|_| Error::Invalid)?),
            )
            .await,
    )?;
    check(
        "ISO week uses its week-year",
        count(
            &client,
            "SELECT count(*) FROM people_skill_reminders WHERE period_key='2026-W53'",
        )
        .await?
            > 0,
    )?;

    let job = must(skills.enqueue_reminders(&manager, COMPANY).await)?;
    grants
        .set_direct(&admin, MANAGER, REMINDERS_SEND, Some(false))
        .await?;
    check(
        "grant revoked before direct retry refuses next write",
        matches!(
            skills.retry_reminders(&manager, COMPANY, None).await,
            Err(SkillsError::Unauthorized)
        ),
    )?;
    let executor = Executor::from_services(
        &services,
        Config {
            poll_interval: Duration::from_millis(20),
            ..Default::default()
        },
    )?;
    let (shutdown, rx) = tokio::sync::watch::channel(false);
    let runner = tokio::spawn(executor.run(rx));
    let mut cancelled = false;
    for _ in 0..300 {
        let row = client
            .query_one(
                "SELECT state FROM base_queue_jobs WHERE id=$1",
                &[&job.id()],
            )
            .await?;
        if row.get::<_, String>(0) == "cancelled" {
            cancelled = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    shutdown.send(true).map_err(|_| Error::Internal)?;
    runner.await.map_err(|_| Error::Internal)??;
    check(
        "delegated worker cancels after operator grant revoked",
        cancelled,
    )?;
    grants
        .set_direct(&admin, MANAGER, REMINDERS_SEND, Some(true))
        .await?;
    let job = must(skills.enqueue_reminders(&manager, COMPANY).await)?;
    let executor = Executor::from_services(
        &services,
        Config {
            poll_interval: Duration::from_millis(20),
            ..Default::default()
        },
    )?;
    let (shutdown, rx) = tokio::sync::watch::channel(false);
    let runner = tokio::spawn(executor.run(rx));
    let mut completed = false;
    for _ in 0..300 {
        let row = client
            .query_one(
                "SELECT state FROM base_queue_jobs WHERE id=$1",
                &[&job.id()],
            )
            .await?;
        if row.get::<_, String>(0) == "completed" {
            completed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    shutdown.send(true).map_err(|_| Error::Internal)?;
    runner.await.map_err(|_| Error::Internal)??;
    check("delegated worker completes as operator", completed)?;
    check("recipient notifications and sent ledger share audit trace",count(&client,"SELECT count(*) FROM base_audit_mutations r JOIN base_audit_mutations n ON n.trace_id=r.trace_id AND n.auditable_type='notification' WHERE r.auditable_type='people_skill_reminder' AND r.event='updated'").await?>0)?;
    println!("PK7 proof complete");
    Ok(())
}

walkthrough_command!(
    "skill-reminders",
    |url, _argument| async move { run(url).await }
);
