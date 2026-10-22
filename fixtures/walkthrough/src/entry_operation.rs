//! PL5 runtime proof through the real registry and reviewed plan service.
//! Uses disposable fixture credentials; never prints credentials or plan codes.
use bilibi_agent_api::{
    plans::{PlanAuthority, PlanIdentity, PlannedOperation, Plans},
    Registry,
};
use bilibi_authz::grants::Grants;
use bilibi_database::{Db, Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_leave::{Leave, NewLeaveType, POLICIES_MANAGE};
use bilibi_tenancy::{ProofFuture, Scope};
use bilibi_user::UserService;
use serde_json::{json, Value};
use tokio_postgres::Transaction;

const KEY: &str = "people.leave.entry.record";
struct Browser<'a> {
    users: &'a UserService,
    token: &'a str,
}
impl PlanAuthority for Browser<'_> {
    fn verify<'a>(&'a self, tx: &'a Transaction<'_>) -> ProofFuture<'a, PlanIdentity> {
        Box::pin(async move {
            tx.query_opt(
                "SELECT id FROM sessions WHERE id=$1 AND expires_at>clock_timestamp() FOR UPDATE",
                &[&self.token],
            )
            .await?
            .ok_or(Error::Unauthorized)?;
            Ok(PlanIdentity {
                scope: self.users.sessions.check(tx, self.token).await?,
                credential_binding: format!("browser:{}", self.token),
            })
        })
    }
}
fn op(input: Value) -> PlannedOperation {
    PlannedOperation {
        key: KEY.into(),
        input,
    }
}
fn code(plan: &Value) -> Result<&str> {
    plan["plan_code"].as_str().ok_or(Error::Invalid)
}
fn backend(error: bilibi_agent_api::Error) -> Error {
    eprintln!("unexpected operation failure: {error}");
    Error::Invalid
}
async fn preview(plans: &Plans, browser: &Browser<'_>, ops: &[PlannedOperation]) -> Result<Value> {
    let mut plan = plans.preview(browser, ops).await.map_err(backend)?;
    let secret = plan["plan_code"].take();
    println!(
        "entry review: valid={} facts={} code issued={}",
        plan["valid"],
        plan["results"],
        secret.is_string()
    );
    plan["plan_code"] = secret;
    Ok(plan)
}
async fn state(db: &Db, source: &str) -> Result<(i64, i64)> {
    let client = db.get().await?;
    let row = client.query_one("SELECT count(*), (SELECT count(*) FROM base_audit_mutations WHERE auditable_type='people_leave_ledger_entry' AND auditable_id IN (SELECT id::text FROM people_leave_ledger_entries WHERE tenant_id=1 AND source=$1)) FROM people_leave_ledger_entries WHERE tenant_id=1 AND source=$1", &[&source]).await?;
    Ok((row.get(0), row.get(1)))
}
fn same(step: &str, got: (i64, i64), expected: (i64, i64)) -> Result<()> {
    println!("{step}: entries={} captured={}", got.0, got.1);
    if got == expected {
        Ok(())
    } else {
        Err(Error::Invalid)
    }
}
pub(super) async fn run(
    services: &Services,
    db: &Db,
    admin: &Scope,
    company: i64,
    employee: i64,
    other_employee: i64,
) -> Result<()> {
    println!("\n== PL5: reviewed Leave entry recording");
    let users = services.get::<UserService>("core/user.accounts")?;
    let plans = services.get::<Plans>("base/agent_api.plans")?;
    let registry = services.get::<Registry>("base/agent_api.registry")?;
    let leave = services.get::<Leave>(bilibi_people_leave::SERVICE)?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    let login = users
        .login("manager@example.test", bilibi_walkthrough_fixture::PASSWORD)
        .await
        .map_err(|_| Error::Unauthorized)?;
    let browser = Browser {
        users: &users,
        token: &login.token,
    };
    let client = db.get().await?;
    let scope = users.sessions.check(&**client, &login.token).await?;
    let leave_type = leave
        .create_type(
            &scope,
            company,
            &NewLeaveType::new("entry_proof", "Entry proof", "hour"),
        )
        .await
        .map_err(|_| Error::Invalid)?;
    println!(
        "entry describe: {}",
        registry
            .describe(&**client, &scope, KEY)
            .await
            .map_err(backend)?
    );
    println!(
        "entry guide: {}",
        registry
            .guide(&**client, &scope, "people.leave.entry")
            .await
            .map_err(backend)?["title"]
    );
    let input = json!({"company_id":company,"employee_id":employee,"leave_type_id":leave_type.id,"entry_type":"opening","quantity":"5.25","occurred_on":"2026-07-01","source":"pl5-proof","entry_key":"opening","note":"  Opening proof  "});
    println!(
        "direct write: {:?}",
        registry.call(&**client, &scope, KEY, &input).await
    );
    let ops = [op(input.clone())];
    let plan = preview(&plans, &browser, &ops).await?;
    same(
        "preview has no entry writes",
        state(db, "pl5-proof").await?,
        (0, 0),
    )?;
    println!(
        "opening commit: {}",
        plans
            .commit(&scope, &browser, code(&plan)?, &ops)
            .await
            .map_err(backend)?
    );
    same("opening", state(db, "pl5-proof").await?, (1, 1))?;
    println!(
        "single-use plan replay: {:?}",
        plans.commit(&scope, &browser, code(&plan)?, &ops).await
    );
    let replay = preview(&plans, &browser, &ops).await?;
    println!(
        "identical entry retry: {}",
        plans
            .commit(&scope, &browser, code(&replay)?, &ops)
            .await
            .map_err(backend)?
    );
    same(
        "retry adds no entry or mutation",
        state(db, "pl5-proof").await?,
        (1, 1),
    )?;
    let mut adjustment = input.clone();
    adjustment["entry_type"] = json!("adjustment");
    adjustment["quantity"] = json!("-0.50");
    adjustment["entry_key"] = json!("adjustment");
    let adjust = [op(adjustment)];
    let plan = preview(&plans, &browser, &adjust).await?;
    println!(
        "adjustment commit: {}",
        plans
            .commit(&scope, &browser, code(&plan)?, &adjust)
            .await
            .map_err(backend)?
    );
    same("adjustment", state(db, "pl5-proof").await?, (2, 2))?;
    for (field, value) in [
        ("quantity", json!("7.00")), // existing key conflict
        ("quantity", json!("0")),
        ("quantity", json!("0.001")),
        ("quantity", json!("99999")),
        ("quantity", json!(5.25)),
        ("quantity", json!("1.00000000000000000000000000001")),
        ("occurred_on", json!("2026-02-30")),
        ("source", json!("policy")),
        ("source", json!("")),
        ("source", json!("é".repeat(17))),
        ("entry_key", json!("")),
        ("entry_type", json!("taken")),
        ("employee_id", json!(other_employee)),
        ("leave_type_id", json!(9_999_999)),
        ("company_id", json!(100)),
        ("actor_user_id", json!(4)),
    ] {
        let mut bad = input.clone();
        bad[field] = value;
        let review = preview(&plans, &browser, &[op(bad)]).await?;
        if review["valid"] != false {
            return Err(Error::Invalid);
        }
    }
    // Both operations initially review as new. The second then conflicts with
    // the first on the shared outer transaction: neither row nor audit survives.
    let mut first = input.clone();
    first["source"] = json!("pl5-rollback");
    let mut second = first.clone();
    second["quantity"] = json!("8.25");
    let batch = [op(first), op(second)];
    let review = preview(&plans, &browser, &batch).await?;
    println!(
        "second operation fails: {:?}",
        plans.commit(&scope, &browser, code(&review)?, &batch).await
    );
    same(
        "whole plan rolls back",
        state(db, "pl5-rollback").await?,
        (0, 0),
    )?;
    // Separate plans competing for one source/key cannot insert twice.
    let mut race = input.clone();
    race["entry_key"] = json!("race");
    let race_ops = [op(race)];
    let a = preview(&plans, &browser, &race_ops).await?;
    let second_login = users
        .login("manager@example.test", bilibi_walkthrough_fixture::PASSWORD)
        .await
        .map_err(|_| Error::Unauthorized)?;
    let second_browser = Browser {
        users: &users,
        token: &second_login.token,
    };
    let b = preview(&plans, &second_browser, &race_ops).await?;
    let (a_result, b_result) = tokio::join!(
        plans.commit(&scope, &browser, code(&a)?, &race_ops),
        plans.commit(&scope, &second_browser, code(&b)?, &race_ops),
    );
    println!(
        "competing plans: first={} second={}",
        a_result.is_ok(),
        b_result.is_ok()
    );
    if a_result.is_ok() == b_result.is_ok() {
        return Err(Error::Invalid);
    }
    same(
        "competing plans add one",
        state(db, "pl5-proof").await?,
        (3, 3),
    )?;
    let mut fresh = input.clone();
    fresh["entry_key"] = json!("revoked");
    let fresh_ops = [op(fresh)];
    let review = preview(&plans, &browser, &fresh_ops).await?;
    grants
        .set_direct(admin, scope.user(), POLICIES_MANAGE, Some(false))
        .await?;
    println!(
        "permission revoked after review: {:?}",
        plans
            .commit(&scope, &browser, code(&review)?, &fresh_ops)
            .await
    );
    grants
        .set_direct(admin, scope.user(), POLICIES_MANAGE, Some(true))
        .await?;
    let review = preview(&plans, &browser, &fresh_ops).await?;
    leave
        .set_type_status(&scope, company, leave_type.id, "archived")
        .await
        .map_err(|_| Error::Invalid)?;
    println!(
        "type archived after review: {:?}",
        plans
            .commit(&scope, &browser, code(&review)?, &fresh_ops)
            .await
    );
    leave
        .set_type_status(&scope, company, leave_type.id, "active")
        .await
        .map_err(|_| Error::Invalid)?;
    same(
        "refusals add no entry",
        state(db, "pl5-proof").await?,
        (3, 3),
    )?;
    let second_scope = users.sessions.check(&**client, &second_login.token).await?;
    users
        .logout(&second_login.token, &second_scope.csrf)
        .await?;
    users.logout(&login.token, &scope.csrf).await?;
    Ok(())
}
