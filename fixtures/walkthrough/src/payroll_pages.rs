//! PY4 disposable running-system proof. No test harness or production seed.
use bilibi_authz::grants::Grants;
use bilibi_database::{pool, Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_attendance::{Attendance, NewAllowanceRule};
use bilibi_people_claims::{Claims, NewCategory, NewClaimType};
use bilibi_people_leave::{Leave, NewLeaveType};
use bilibi_people_payroll::{Payroll, ATTENDANCE_MAPPINGS_MANAGE, MANAGE, VIEW};
use bilibi_tenancy::{Authentication, Scope};
use bilibi_ui::page::{LiveInput, LiveRequest, LiveState, PageRequest, PageService};
use jiff::civil::date;
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc};
const PAGES: &str = "people/payroll.setup_pages";
fn prove(label: &str, yes: bool) -> Result<()> {
    if !yes {
        eprintln!("FAILED: {label}");
        return Err(Error::Internal);
    }
    println!("proved: {label}");
    Ok(())
}
async fn send(
    page: &mut dyn LiveState<Scope, Error>,
    scope: &Scope,
    event: Value,
) -> Result<String> {
    Ok(page
        .update(LiveRequest {
            scope,
            credential: "payroll pages proof",
            input: LiveInput::Event(&event),
        })
        .await?
        .html)
}
pub(super) async fn run(url: &str) -> Result<()> {
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
    services
        .get::<bilibi_base::step_up::StepUp>(bilibi_base::step_up::SERVICE)?
        .set_enforced(false);
    let grants = services.get::<Grants>("base/authz.grants")?;
    grants.reconcile().await?;
    let auth = services.get::<Authentication>("base/tenancy.authentication")?;
    let c = db.get().await?;
    let admin = auth
        .authenticate(&**c, 1, 4, "payroll pages proof".into())
        .await?;
    let manager = auth
        .authenticate(&**c, 1, 1, "payroll pages proof".into())
        .await?;
    let viewer = auth
        .authenticate(&**c, 1, 2, "payroll pages proof".into())
        .await?;
    drop(c);
    for key in [
        VIEW,
        MANAGE,
        ATTENDANCE_MAPPINGS_MANAGE,
        bilibi_people_leave::POLICIES_MANAGE,
        bilibi_people_claims::MANAGE,
        bilibi_people_attendance::ALLOWANCES_MANAGE,
    ] {
        grants.set_direct(&admin, 1, key, Some(true)).await?;
    }
    grants.set_direct(&admin, 2, VIEW, Some(true)).await?;
    // This fixture viewer is unlinked, so grant company reach separately
    // from Payroll's read capability. It still has no setup write grant.
    grants
        .set_direct(&admin, 2, bilibi_company::TENANT_WIDE, Some(true))
        .await?;
    let payroll = services.get::<Payroll>(bilibi_people_payroll::SERVICE)?;
    let pages = services.get::<PageService<Scope, Error>>(PAGES)?;
    let query = HashMap::from([("company_id".into(), "1".into())]);
    let path = HashMap::new();
    let request = |operation, scope| PageRequest {
        operation,
        scope: Some(scope),
        path: &path,
        query: &query,
        body: &[],
        framed: false,
    };
    let html = pages.0.render(request("setup", &manager)).await?.html;
    prove(
        "setup initial empty states",
        html.contains("No classifications yet.") && html.contains("No frozen runs yet."),
    )?;
    prove(
        "attendance link with explicit grant",
        html.contains("attendance-mapping-link"),
    )?;
    let mut page = pages
        .0
        .mount(request("setup", &manager))
        .await?
        .ok_or(Error::Internal)?;
    page.update(LiveRequest {
        scope: &manager,
        credential: "proof",
        input: LiveInput::Refresh,
    })
    .await?;
    let html = send(
        &mut *page,
        &manager,
        json!({"type":"save_settings","country":"proof","currencies":"xaa"}),
    )
    .await?;
    prove(
        "invalid settings are recoverable",
        html.contains("Enter a country identifier and uppercase three-letter currency codes."),
    )?;
    let html = send(
        &mut *page,
        &manager,
        json!({"type":"save_settings","country":"proof","currencies":"XAA"}),
    )
    .await?;
    prove("settings save", html.contains("Payroll setup saved."))?;
    let html=send(&mut *page,&manager,json!({"type":"create_classification","record[code]":"page_class","record[name]":"Page classification","record[effective_from]":"2035-01-01","record[effective_to]":"2035-12-31"})).await?;
    prove("classification form", html.contains("Page classification"))?;
    let class = payroll
        .setup(&manager, 1)
        .await
        .map_err(|_| Error::Internal)?
        .classifications[0]
        .id;
    let html=send(&mut *page,&manager,json!({"type":"create_item","record[code]":"page_item","record[name]":"Page item","record[classification_id]":class.to_string(),"record[currency]":"XAA","record[amount]":"12.345678","record[effective_from]":"2035-01-01","record[effective_to]":"2035-12-31"})).await?;
    prove("pay item exact amount", html.contains("12.345678"))?;
    let item = payroll
        .setup(&manager, 1)
        .await
        .map_err(|_| Error::Internal)?
        .items[0]
        .id;
    let html=send(&mut *page,&manager,json!({"type":"create_period","record[code]":"page_period","record[starts_on]":"2035-01-01","record[ends_on]":"2035-01-31","record[pay_on]":"2035-01-30"})).await?;
    prove(
        "period refusal has field error summary",
        html.contains("period-form-errors") && html.contains("must be on or after the period end"),
    )?;
    let html=send(&mut *page,&manager,json!({"type":"create_period","record[code]":"page_period","record[starts_on]":"2035-01-01","record[ends_on]":"2035-01-31","record[pay_on]":"2035-01-31"})).await?;
    prove("period form", html.contains("page_period"))?;
    let leave = services.get::<Leave>(bilibi_people_leave::SERVICE)?;
    let leave_type = leave
        .create_type(
            &manager,
            1,
            &NewLeaveType::new("page_leave", "Page leave", "day"),
        )
        .await
        .map_err(|_| Error::Internal)?;
    let claims = services.get::<Claims>(bilibi_people_claims::SERVICE)?;
    let category = claims
        .create_category(
            &manager,
            1,
            &NewCategory {
                code: "page_category".into(),
                name: "Page category".into(),
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    let claim_type = claims
        .create_claim_type(
            &manager,
            1,
            &NewClaimType {
                category_id: category.id,
                code: "page_claim".into(),
                name: "Page claim".into(),
                receipt_requirement: "never".into(),
                eligibility: None,
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    for source in [
        format!("leave:{}", leave_type.id),
        format!("claims:{}", claim_type.id),
    ] {
        let html=send(&mut *page,&manager,json!({"type":"create_mapping","record[source]":source,"record[item_id]":item.to_string(),"record[effective_from]":"2035-01-01","record[effective_to]":"2035-12-31"})).await?;
        prove(
            "Leave/Claims mapping form",
            html.contains("Payroll setup saved."),
        )?;
    }
    let attendance = services.get::<Attendance>(bilibi_people_attendance::SERVICE)?;
    attendance
        .create_allowance_rule(
            &manager,
            1,
            &NewAllowanceRule {
                code: "page_rule".into(),
                name: "Page allowance".into(),
                unit: "day".into(),
                value: "1.50".parse().map_err(|_| Error::Invalid)?,
                currency: "XAA".into(),
                effective_from: date(2035, 1, 1),
                effective_until: Some(date(2035, 12, 31)),
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    let mut ap = pages
        .0
        .mount(request("attendance_mappings", &manager))
        .await?
        .ok_or(Error::Internal)?;
    let html = ap
        .update(LiveRequest {
            scope: &manager,
            credential: "proof",
            input: LiveInput::Refresh,
        })
        .await?
        .html;
    prove(
        "future allowance rule offered",
        html.contains("Page allowance") && html.contains("Not mapped"),
    )?;
    let mapping = json!({"type":"create","mapping[attendance_rule_code]":"page_rule","mapping[item_id]":item.to_string(),"mapping[effective_from]":"2035-01-01","mapping[effective_to]":"2035-01-31"});
    let html = send(&mut *ap, &manager, mapping.clone()).await?;
    prove(
        "attendance mapping history",
        html.contains("Attendance allowance mapped.") && html.contains("Mapping history"),
    )?;
    let html = send(&mut *ap, &manager, mapping).await?;
    prove(
        "overlap page sentence",
        html.contains("This rule already has a pay item for those dates."),
    )?;
    let period = payroll
        .setup(&manager, 1)
        .await
        .map_err(|_| Error::Internal)?
        .periods[0]
        .id;
    let html = send(
        &mut *page,
        &manager,
        json!({"type":"create_run","period_id":period.to_string(),"currency":"XAA"}),
    )
    .await?;
    prove(
        "freeze holds selection and asks first",
        html.contains("payroll-freeze-confirm")
            && payroll
                .setup(&manager, 1)
                .await
                .map_err(|_| Error::Internal)?
                .runs
                .is_empty(),
    )?;
    send(&mut *page, &manager, json!({"type":"cancel_freeze"})).await?;
    send(
        &mut *page,
        &manager,
        json!({"type":"confirm_freeze","period_id":period.to_string(),"currency":"XAA"}),
    )
    .await?;
    prove(
        "cancelled freeze cannot be forged",
        payroll
            .setup(&manager, 1)
            .await
            .map_err(|_| Error::Internal)?
            .runs
            .is_empty(),
    )?;
    send(
        &mut *page,
        &manager,
        json!({"type":"create_run","period_id":period.to_string(),"currency":"XAA"}),
    )
    .await?;
    send(
        &mut *page,
        &manager,
        json!({"type":"confirm_freeze","period_id":"999999","currency":"BAD"}),
    )
    .await?;
    let data = payroll
        .setup(&manager, 1)
        .await
        .map_err(|_| Error::Internal)?;
    prove(
        "freeze uses server-held selection",
        data.runs.len() == 1 && data.runs[0].period_id == period && data.runs[0].currency == "XAA",
    )?;
    let run = data.runs[0].id;
    send(
        &mut *page,
        &manager,
        json!({"type":"prepare_lock","id":run.to_string()}),
    )
    .await?;
    grants.set_direct(&admin, 1, MANAGE, Some(false)).await?;
    let html = send(&mut *page, &manager, json!({"type":"lock_run"})).await?;
    prove(
        "grant revoked before confirmation",
        html.replace("&#39;", "'")
            .replace("&#x27;", "'")
            .contains("You cannot change this company's payroll setup.")
            && payroll
                .setup(&manager, 1)
                .await
                .map_err(|_| Error::Internal)?
                .runs[0]
                .locked_at
                .is_none(),
    )?;
    grants.set_direct(&admin, 1, MANAGE, Some(true)).await?;
    send(
        &mut *page,
        &manager,
        json!({"type":"prepare_lock","id":run.to_string()}),
    )
    .await?;
    send(
        &mut *page,
        &manager,
        json!({"type":"select_company","company_id":"2"}),
    )
    .await?;
    send(
        &mut *page,
        &manager,
        json!({"type":"lock_run","id":run.to_string()}),
    )
    .await?;
    prove(
        "company switch clears lock selection",
        payroll
            .setup(&manager, 1)
            .await
            .map_err(|_| Error::Internal)?
            .runs[0]
            .locked_at
            .is_none(),
    )?;
    send(
        &mut *page,
        &manager,
        json!({"type":"select_company","company_id":"1"}),
    )
    .await?;
    send(
        &mut *page,
        &manager,
        json!({"type":"prepare_lock","id":run.to_string()}),
    )
    .await?;
    let html = send(
        &mut *page,
        &manager,
        json!({"type":"lock_run","id":"999999"}),
    )
    .await?;
    prove(
        "locked run displays its UTC instant",
        html.contains("<time"),
    )?;
    prove(
        "permanent lock uses selected run",
        payroll
            .setup(&manager, 1)
            .await
            .map_err(|_| Error::Internal)?
            .runs[0]
            .locked_at
            .is_some(),
    )?;
    let html = pages.0.render(request("setup", &viewer)).await?.html;
    prove(
        "viewer has withheld sentence and no write forms",
        html.contains("A payroll operator can add records.")
            && !html.contains("id=\"payroll-settings\"")
            && !html.contains("attendance-mapping-link"),
    )?;
    let mut vp = pages
        .0
        .mount(request("setup", &viewer))
        .await?
        .ok_or(Error::Internal)?;
    vp.update(LiveRequest {
        scope: &viewer,
        credential: "proof",
        input: LiveInput::Refresh,
    })
    .await?;
    let html = send(
        &mut *vp,
        &viewer,
        json!({"type":"save_settings","country":"forged","currencies":"BAD"}),
    )
    .await?;
    prove(
        "viewer forged write refused",
        html.replace("&#39;", "'")
            .replace("&#x27;", "'")
            .contains("You cannot change this company's payroll setup.")
            && payroll
                .setup(&manager, 1)
                .await
                .map_err(|_| Error::Internal)?
                .country
                == "proof",
    )?;
    let html = pages.0.render(request("mappings", &manager)).await?.html;
    prove(
        "mapping variant focuses the shared section",
        html.contains("id=\"payroll-mappings-section\" tabindex=\"-1\" autofocus"),
    )?;
    attendance
        .create_allowance_rule(
            &manager,
            1,
            &NewAllowanceRule {
                code: "browser_rule".into(),
                name: "Browser allowance".into(),
                unit: "day".into(),
                value: "2.50".parse().map_err(|_| Error::Invalid)?,
                currency: "XAA".into(),
                effective_from: date(2026, 1, 1),
                effective_until: Some(date(2026, 12, 31)),
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    println!(
        "Payroll page proof complete. Browser accounts are ready in this disposable database."
    );
    Ok(())
}

walkthrough_command!(
    "payroll_pages",
    |url, _argument| async move { run(url).await }
);
