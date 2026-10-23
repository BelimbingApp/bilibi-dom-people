//! PC4: run the registered live page and its owner on a disposable database.
use bilibi_database::{Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_claims::{
    Claims, ClaimsError, NewAssignment, NewCategory, NewClaimType, NewPolicy, MANAGE,
};
use bilibi_tenancy::{Authentication, Scope};
use bilibi_ui::page::{LiveBody, LiveInput, LiveRequest, LiveState, PageRequest, PageService};
use jiff::civil::date;
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc};

fn require(ok: bool, detail: &str) -> Result<()> {
    if ok {
        println!("PC4: {detail}");
        Ok(())
    } else {
        eprintln!("PC4 proof failed: {detail}");
        Err(Error::Invalid)
    }
}
fn saved<T>(value: std::result::Result<T, ClaimsError>) -> Result<T> {
    value.map_err(|e| {
        eprintln!("PC4: {e}");
        Error::Invalid
    })
}
async fn event(
    page: &mut dyn LiveState<Scope, Error>,
    scope: &Scope,
    payload: Value,
) -> Result<LiveBody> {
    page.update(LiveRequest {
        scope,
        credential: "pc4-proof",
        input: LiveInput::Event(&payload),
    })
    .await
}
pub(crate) async fn run(url: &str, action: &str) -> Result<()> {
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = bilibi_database::pool(url)?;
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
        .authenticate(&**client, 1, 4, "pc4-proof".into())
        .await?;
    let manager = auth
        .authenticate(&**client, 1, 1, "pc4-proof".into())
        .await?;
    let grants = services.get::<bilibi_authz::grants::Grants>("base/authz.grants")?;
    if matches!(action, "revoke" | "restore") {
        grants
            .set_direct(&admin, 1, MANAGE, Some(action == "restore"))
            .await?;
        return Ok(());
    }
    if action == "archive" {
        services
            .get::<bilibi_company::CompanyService>(bilibi_company::SERVICE)?
            .archive_company(&admin, 2, Some("PC4 disposable page proof"))
            .await
            .map_err(|_| Error::Invalid)?;
        return Ok(());
    }
    let claims = services.get::<Claims>(bilibi_people_claims::SERVICE)?;
    if action == "readonly" {
        let before = saved(claims.categories(&manager, 2).await)?.len();
        let pages = services.get::<PageService<Scope, Error>>("people/claims.setup_pages")?;
        let query = HashMap::from([("company_id".into(), "2".into())]);
        let mut page = pages
            .0
            .mount(PageRequest {
                operation: "setup",
                scope: Some(&manager),
                path: &HashMap::new(),
                query: &query,
                body: &[],
                framed: false,
            })
            .await?
            .ok_or(Error::Internal)?;
        let body = page
            .update(LiveRequest {
                scope: &manager,
                credential: "pc4-proof",
                input: LiveInput::Refresh,
            })
            .await?;
        require(
            body.html.contains("claim-setup-archived")
                && body.html.contains("claim-policies-table")
                && !body.html.contains("data-event=\"create_category\"")
                && !body.html.contains("data-event=\"save_assignment_members\""),
            "authorized archived setup remains readable with write controls withheld",
        )?;
        let body = event(page.as_mut(), &manager, json!({"type":"create_category", "category[code]":"pc4_archived_refused", "category[name]":"Refused archived category"})).await?;
        require(
            body.html.contains("This company is archived and read-only")
                && body.query.get("company_id").map(String::as_str) == Some("2")
                && saved(claims.categories(&manager, 2).await)?.len() == before,
            "a forged stale write keeps the selected company and refuses without mutation",
        )?;
        return Ok(());
    }
    if action != "proof" {
        return Err(Error::Invalid);
    }
    let company = 2;
    let stamp = jiff::Timestamp::now().as_second();
    let category = saved(
        claims
            .create_category(
                &manager,
                company,
                &NewCategory {
                    code: format!("pc4_members_{stamp}"),
                    name: "Assignment proof category".into(),
                },
            )
            .await,
    )?;
    let mut types = vec![];
    for n in 0..70 {
        types.push(
            saved(
                claims
                    .create_claim_type(
                        &manager,
                        company,
                        &NewClaimType {
                            category_id: category.id,
                            code: format!("pc4_member_{stamp}_{n:03}"),
                            name: format!("Assignment proof type {n:03}"),
                            receipt_requirement: "never".into(),
                            eligibility: Some("assigned_only".into()),
                        },
                    )
                    .await,
            )?
            .id,
        );
    }
    let employees = saved(claims.assignable_employees(&manager, company).await)?;
    require(
        !employees.is_empty(),
        "company has an assignable employment",
    )?;
    let assignment = saved(
        claims
            .create_assignment(
                &manager,
                company,
                &NewAssignment {
                    code: format!("pc4_members_{stamp}"),
                    name: "Assignment membership proof".into(),
                    effective_from: date(2025, 1, 1),
                    effective_to: None,
                },
            )
            .await,
    )?;
    let currencies = saved(claims.currencies(&manager, company).await)?;
    let currency = currencies.first().cloned().unwrap_or_else(|| "ABC".into());
    if currencies.is_empty() {
        saved(
            claims
                .put_currencies(&manager, company, std::slice::from_ref(&currency))
                .await,
        )?;
    }
    let policy = saved(
        claims
            .create_policy(
                &manager,
                company,
                &NewPolicy {
                    claim_type_id: types[69],
                    effective_from: date(2025, 1, 1),
                    effective_to: None,
                    currency,
                    per_claim_limit: None,
                    monthly_limit: None,
                    yearly_limit: None,
                    receipt_threshold: None,
                },
            )
            .await,
    )?;
    let pages = services.get::<PageService<Scope, Error>>("people/claims.setup_pages")?;
    let query = HashMap::from([("company_id".into(), company.to_string())]);
    let mut page = pages
        .0
        .mount(PageRequest {
            operation: "setup",
            scope: Some(&manager),
            path: &HashMap::new(),
            query: &query,
            body: &[],
            framed: false,
        })
        .await?
        .ok_or(Error::Internal)?;
    let first = page
        .update(LiveRequest {
            scope: &manager,
            credential: "pc4-proof",
            input: LiveInput::Refresh,
        })
        .await?;
    require(
        first.html.contains("claim-policies-table")
            && first.html.contains("claim-assignments-table"),
        "five tables render from the registered page",
    )?;
    let mut payload = json!({ "type": "save_assignment_members", "assignment_id": assignment.id.to_string(),
        "filters[claim_type_ids][]": std::iter::once(String::new()).chain(types.iter().map(i64::to_string)).collect::<Vec<_>>(),
        "filters[employee_ids][]": std::iter::once(String::new()).chain(employees.iter().map(|e| e.id.to_string())).collect::<Vec<_>>() });
    // Alphabetically precede the real keys, as serde_json sorts object keys.
    for n in 0..80 {
        payload[format!("a_filler_{n:03}")] = json!("ignored");
    }
    let body = event(page.as_mut(), &manager, payload).await?;
    require(
        body.html.contains("Assignment members saved."),
        "membership event succeeds beyond 64 posted fields",
    )?;
    let current = saved(claims.assignments(&manager, company).await)?
        .into_iter()
        .find(|a| a.id == assignment.id)
        .ok_or(Error::NotFound)?;
    require(
        current.claim_type_ids.len() == 70
            && current.claim_type_ids.contains(&types[69])
            && current.employee_ids.len() == employees.len(),
        "all seventy types and employment IDs were stored",
    )?;
    let invalid = event(page.as_mut(), &manager, json!({"type":"save_assignment_members", "assignment_id":assignment.id.to_string(), "filters[employee_ids][]":["invalid"]})).await?;
    require(
        invalid
            .html
            .contains("Choose claim types and employees of this company."),
        "malformed membership refuses without clearing stored members",
    )?;
    let empty = event(
        page.as_mut(),
        &manager,
        json!({"type":"save_assignment_members", "assignment_id":assignment.id.to_string(), "filters[claim_type_ids][]":[""], "filters[employee_ids][]":[""]}),
    )
    .await?;
    require(
        empty.html.contains("Assignment members saved."),
        "hidden empty values clear both selections",
    )?;
    let current = saved(claims.assignments(&manager, company).await)?
        .into_iter()
        .find(|a| a.id == assignment.id)
        .ok_or(Error::NotFound)?;
    require(
        current.claim_type_ids.is_empty() && current.employee_ids.is_empty(),
        "empty selections persisted",
    )?;
    let end = json!({"type":"end_policy", "policy_id":policy.id.to_string(), "effective_to":"2025-12-31"});
    let body = event(page.as_mut(), &manager, end.clone()).await?;
    require(
        body.html.contains("claim-setup-confirm"),
        "ending a policy opens ConfirmDialog",
    )?;
    event(page.as_mut(), &manager, json!({"type":"cancel_end"})).await?;
    require(
        saved(claims.policies(&manager, company).await)?
            .iter()
            .any(|p| p.id == policy.id && p.effective_to.is_none()),
        "Cancel leaves the policy open",
    )?;
    event(page.as_mut(), &manager, end.clone()).await?;
    grants.set_direct(&admin, 1, MANAGE, Some(false)).await?;
    let refused = event(page.as_mut(), &manager, json!({"type":"confirm_end"})).await?;
    grants.set_direct(&admin, 1, MANAGE, Some(true)).await?;
    require(
        refused.html.contains("You no longer have permission")
            && !refused.html.contains("claim-policies-table"),
        "revoked permission refuses confirmation and clears setup records",
    )?;
    require(
        saved(claims.policies(&manager, company).await)?
            .iter()
            .any(|p| p.id == policy.id && p.effective_to.is_none()),
        "revoked confirmation wrote nothing",
    )?;
    event(page.as_mut(), &manager, end).await?;
    let body = event(
        page.as_mut(),
        &manager,
        json!({"type":"confirm_end", "policy_id":"9999999", "effective_to":"2030-01-01"}),
    )
    .await?;
    require(
        body.html.contains("Policy end date saved."),
        "confirmation uses the server-held record and date",
    )?;
    require(
        saved(claims.policies(&manager, company).await)?
            .iter()
            .any(|p| p.id == policy.id && p.effective_to == Some(date(2025, 12, 31))),
        "reviewed policy end persisted",
    )?;
    event(page.as_mut(), &manager, json!({"type":"end_assignment", "assignment_id":assignment.id.to_string(), "effective_to":"2025-12-31"})).await?;
    let body = event(
        page.as_mut(),
        &manager,
        json!({"type":"select_company", "company_id":"1"}),
    )
    .await?;
    require(
        !body.html.contains("claim-setup-confirm")
            && body.query.get("company_id").map(String::as_str) == Some("1"),
        "changing company clears pending confirmation and updates the address",
    )?;
    require(
        saved(claims.assignments(&manager, company).await)?
            .iter()
            .any(|a| a.id == assignment.id && a.effective_to.is_none()),
        "switching company did not end the assignment",
    )?;
    println!("PC4 live page proof complete");
    Ok(())
}
