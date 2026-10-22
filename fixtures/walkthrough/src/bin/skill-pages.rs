//! PK6 executable live-page proof; run after the default disposable walkthrough.
use bilibi_database::{Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_skills::*;
use bilibi_tenancy::{Authentication, Scope};
use bilibi_ui::page::{LiveInput, LiveRequest, LiveState, PageRequest, PageService};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc};
fn check(value: bool, label: &str) -> Result<()> {
    if !value {
        eprintln!("PK6 failed: {label}");
        return Err(Error::Internal);
    }
    println!("PK6 proved: {label}");
    Ok(())
}
fn saved<T>(value: bilibi_people_skills::Result<T>) -> Result<T> {
    value.map_err(|e| {
        eprintln!("PK6 unexpected refusal: {e:?}");
        Error::Internal
    })
}
async fn event(
    page: &mut Box<dyn LiveState<Scope, Error>>,
    scope: &Scope,
    value: Value,
) -> Result<String> {
    Ok(page
        .update(LiveRequest {
            scope,
            credential: "walkthrough",
            input: LiveInput::Event(&value),
        })
        .await?
        .html)
}
async fn mount(
    services: &Services,
    scope: &Scope,
    operation: &'static str,
) -> Result<Box<dyn LiveState<Scope, Error>>> {
    let pages = services.get::<PageService<Scope, Error>>("people/skills.action-policy-pages")?;
    let query = HashMap::from([("company_id".into(), "1".into())]);
    let path = HashMap::new();
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
        credential: "walkthrough",
        input: LiveInput::Refresh,
    })
    .await?;
    Ok(page)
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
    let admin = auth
        .authenticate(&**client, 1, 4, "pk6-proof".into())
        .await?;
    let manager = auth
        .authenticate(&**client, 1, 1, "pk6-proof".into())
        .await?;
    let staff = auth
        .authenticate(&**client, 1, 3, "pk6-proof".into())
        .await?;
    let grants = services.get::<bilibi_authz::grants::Grants>("base/authz.grants")?;
    let skills = services.get::<Skills>(bilibi_people_skills::SERVICE)?;
    if mode == "revoke-policy" || mode == "revoke-actions" {
        grants
            .set_direct(
                &admin,
                1,
                if mode == "revoke-policy" {
                    POLICY_MANAGE
                } else {
                    ACTIONS_MANAGE
                },
                Some(false),
            )
            .await?;
        return Ok(());
    }
    for cap in [
        ACTIONS_VIEW,
        ACTIONS_UPDATE,
        ACTIONS_MANAGE,
        ACTIONS_APPROVE,
        CATALOG_MANAGE,
        POLICY_MANAGE,
        REMINDERS_SEND,
        REASSESSMENTS_SUBMIT,
        REASSESSMENTS_EXECUTE,
    ] {
        grants.set_direct(&admin, 1, cap, Some(true)).await?;
    }
    for cap in [ACTIONS_VIEW, ACTIONS_UPDATE] {
        grants.set_direct(&admin, 3, cap, Some(true)).await?;
    }
    for cap in [ACTIONS_MANAGE, ACTIONS_APPROVE, POLICY_MANAGE] {
        grants.set_direct(&admin, 3, cap, Some(false)).await?;
    }
    if mode == "restore" {
        return Ok(());
    }
    let employee:i64=client.query_one("SELECT id FROM people_workforce_employments WHERE tenant_id=1 AND company_id=1 AND employee_number='E-002'",&[]).await?.get(0);
    let manager_employee:i64=client.query_one("SELECT id FROM people_workforce_employments WHERE tenant_id=1 AND company_id=1 AND employee_number='E-001'",&[]).await?.get(0);
    let skill = saved(skills.list_skills(&admin, 1).await)?
        .into_iter()
        .find(|s| s.active)
        .ok_or(Error::Internal)?;
    let kind = saved(
        skills
            .create_action_type(
                &admin,
                1,
                &NewActionType {
                    code: "pk6-practice".into(),
                    name: "Guided practice".into(),
                    requires_provider: false,
                },
            )
            .await,
    )?;
    let plan = ActionPlan {
        action_type_id: kind.id,
        owner_employee_id: employee,
        coordinator_employee_id: manager_employee,
        objective: "PK6 browser intervention".into(),
        intervention: "Observed practice with feedback".into(),
        expected_evidence: "Recorded observation".into(),
        due_on: Some(
            jiff::Timestamp::now()
                .to_zoned(jiff::tz::TimeZone::UTC)
                .date(),
        ),
        ..ActionPlan::default()
    };
    let input = ActionInput {
        source: ActionSource::Manual {
            employee_id: employee,
            skill_id: skill.id,
            starting_level: 0,
            target_level: 1,
            criticality: "essential".into(),
            mandatory: false,
            reason: "PK6 workflow proof".into(),
        },
        plan: plan.clone(),
        request_key: "pk6-page-proof".into(),
    };
    let action = saved(skills.propose_action(&manager, 1, &input).await)?;
    let mut page = mount(&services, &manager, "actions").await?;
    let html = event(
        &mut page,
        &manager,
        json!({
            "type": "create_action", "action[employee_id]": employee.to_string(),
            "action[skill_id]": skill.id.to_string(), "action[action_type_id]": kind.id.to_string()
        }),
    )
    .await?;
    check(
        html.contains("Check the entered values.")
            && html.contains("Starting level (manual) is invalid.")
            && !html.contains("id=\"skills-confirm\""),
        "incomplete manual gaps show field errors before confirmation",
    )?;
    let html = event(
        &mut page,
        &manager,
        json!({"type":"approve_action","id":action.id.to_string()}),
    )
    .await?;
    check(
        html.contains("skills-confirm"),
        "approval opens a confirmation",
    )?;
    let html = event(&mut page, &manager, json!({"type":"confirm","id":"999999"})).await?;
    check(
        html.contains("You cannot decide on work you did or that concerns you."),
        "self approval refuses without ending the live page",
    )?;
    let mut page = mount(&services, &admin, "actions").await?;
    event(
        &mut page,
        &admin,
        json!({"type":"approve_action","id":action.id.to_string()}),
    )
    .await?;
    event(&mut page, &admin, json!({"type":"confirm","id":"999999"})).await?;
    let mut page = mount(&services, &staff, "actions").await?;
    let html = event(
        &mut page,
        &staff,
        json!({"type":"show_history","id":action.id.to_string()}),
    )
    .await?;
    check(
        html.contains("action-history-"),
        "owned history uses a FlexTable",
    )?;
    event(
        &mut page,
        &staff,
        json!({"type":"start_action","id":action.id.to_string()}),
    )
    .await?;
    event(&mut page, &staff, json!({"type":"confirm","id":"999999"})).await?;
    check(
        saved(skills.owned_actions(&staff, 1, ActionGroup::Open).await)?
            .iter()
            .any(|r| r.action.id == action.id && r.action.status == "in_progress"),
        "confirmation ignores a replacement target",
    )?;
    event(
        &mut page,
        &staff,
        json!({"type":"hold_action","id":action.id.to_string(),"reason":"Waiting for review"}),
    )
    .await?;
    let html = event(&mut page, &staff, json!({"type":"dismiss_confirm"})).await?;
    check(
        !html.contains("id=\"skills-confirm\""),
        "Cancel removes the confirmation",
    )?;
    let mut page = mount(&services, &manager, "policy").await?;
    let html = event(
        &mut page,
        &manager,
        json!({"type":"save_policy","policy[reassessment_due_days]":"bad"}),
    )
    .await?;
    check(
        html.contains("Enter whole numbers within the shown limits."),
        "invalid policy stays on the live page",
    )?;
    let before = saved(skills.policy(&admin, 1).await)?;
    grants
        .set_direct(&admin, 1, POLICY_MANAGE, Some(false))
        .await?;
    let html = event(
        &mut page,
        &manager,
        json!({"type":"save_policy","policy[reassessment_due_days]":"17"}),
    )
    .await?;
    check(
        html.contains("You no longer have permission"),
        "revoked page grant refuses the next event",
    )?;
    check(
        saved(skills.policy(&admin, 1).await)? == before,
        "revoked event wrote nothing",
    )?;
    grants
        .set_direct(&admin, 1, POLICY_MANAGE, Some(true))
        .await?;
    check(
        matches!(
            skills.reminder_ledger(&staff, 1, 300).await,
            Err(SkillsError::Unauthorized)
        ),
        "delivery ledger requires reminder-send permission",
    )?;
    let mut page = mount(&services, &manager, "policy").await?;
    for _ in 0..2 {
        let html = event(
            &mut page,
            &manager,
            json!({"type":"toggle_action_type", "id":kind.id.to_string(), "active":"false"}),
        )
        .await?;
        check(
            !html.contains("id=\"skills-confirm\""),
            "type activation applies immediately",
        )?;
    }
    check(
        saved(skills.list_action_types(&admin, 1).await)?
            .iter()
            .any(|t| t.id == kind.id && !t.active),
        "repeated deactivation preserves the requested inactive state",
    )?;
    event(
        &mut page,
        &manager,
        json!({"type":"toggle_action_type", "id":kind.id.to_string(), "active":"true"}),
    )
    .await?;
    check(
        event(&mut page, &manager, json!({"type":"run_reminders"}))
            .await?
            .contains("skills-confirm"),
        "reminder queue waits for confirmation",
    )?;
    event(&mut page, &manager, json!({"type":"dismiss_confirm"})).await?;
    saved(
        skills
            .propose_action(
                &manager,
                1,
                &ActionInput {
                    request_key: "pk6-browser-proposed".into(),
                    plan: ActionPlan {
                        objective: "PK6 proposed browser action".into(),
                        ..plan
                    },
                    source: input.source,
                },
            )
            .await,
    )?;
    println!("PK6 page proof complete; browser fixture ready.");
    Ok(())
}
