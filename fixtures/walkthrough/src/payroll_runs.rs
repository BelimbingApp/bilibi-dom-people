//! PY5 executable page proof on a fresh disposable database.
use bilibi_authz::grants::Grants;
use bilibi_database::{pool, Error, MutationBoundary, Result};
use bilibi_module_registry::Services;
use bilibi_people_attendance::{Attendance, NewAllowanceRule};
use bilibi_people_claims::{Claims, NewCategory, NewClaimType};
use bilibi_people_leave::{Leave, NewLeaveType};
use bilibi_people_payroll::{
    NewAttendanceAllowanceMapping, NewClassification, NewItem, NewMapping, NewPeriod, Payroll,
    APPROVE, ATTENDANCE_MAPPINGS_MANAGE, MANAGE, VIEW,
};
use bilibi_people_reference_data::{NewEntry, ReferenceData};
use bilibi_people_workforce::{NewEmployment, Workforce};
use bilibi_settings::{Settings, Target};
use bilibi_tenancy::{Authentication, Scope};
use bilibi_ui::page::{LiveInput, LiveRequest, LiveState, PageRequest, PageService};
use jiff::civil::date;
use serde_json::{json, Value};
use std::{collections::HashMap, os::unix::fs::PermissionsExt, sync::Arc};
fn prove(label: &str, yes: bool) -> Result<()> {
    if !yes {
        eprintln!("FAILED: {label}");
        return Err(Error::Internal);
    }
    println!("proved: {label}");
    Ok(())
}
fn must<T>(result: bilibi_people_payroll::Result<T>) -> Result<T> {
    result.map_err(|e| {
        eprintln!("Payroll: {e}");
        Error::Internal
    })
}
async fn send(
    page: &mut dyn LiveState<Scope, Error>,
    scope: &Scope,
    event: Value,
) -> Result<String> {
    Ok(page
        .update(LiveRequest {
            scope,
            credential: "PY5 proof",
            input: LiveInput::Event(&event),
        })
        .await?
        .html)
}
async fn mount(
    pages: &PageService<Scope, Error>,
    scope: &Scope,
    run: i64,
) -> Result<Box<dyn LiveState<Scope, Error>>> {
    let query = HashMap::from([
        ("company_id".into(), "1".into()),
        ("run_id".into(), run.to_string()),
    ]);
    let mut page = pages
        .0
        .mount(PageRequest {
            operation: "runs",
            scope: Some(scope),
            path: &HashMap::new(),
            query: &query,
            body: &[],
            framed: false,
        })
        .await?
        .ok_or(Error::Internal)?;
    page.update(LiveRequest {
        scope,
        credential: "PY5 proof",
        input: LiveInput::Refresh,
    })
    .await?;
    Ok(page)
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
    let auth = services.get::<Authentication>("base/tenancy.authentication")?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    grants.reconcile().await?;
    let c = db.get().await?;
    let manager = auth.authenticate(&**c, 1, 1, "PY5 proof".into()).await?;
    let viewer = auth.authenticate(&**c, 1, 2, "PY5 proof".into()).await?;
    let admin = auth.authenticate(&**c, 1, 4, "PY5 proof".into()).await?;
    drop(c);
    for key in [
        VIEW,
        MANAGE,
        APPROVE,
        ATTENDANCE_MAPPINGS_MANAGE,
        bilibi_people_leave::POLICIES_MANAGE,
        bilibi_people_claims::MANAGE,
        bilibi_people_attendance::ALLOWANCES_MANAGE,
    ] {
        grants.set_direct(&admin, 1, key, Some(true)).await?;
    }
    for key in [VIEW, bilibi_company::TENANT_WIDE] {
        grants.set_direct(&admin, 2, key, Some(true)).await?;
    }
    let payroll = services.get::<Payroll>(bilibi_people_payroll::SERVICE)?;
    let workforce = services.get::<Workforce>(bilibi_people_workforce::SERVICE)?;
    // Keep this slice independent of the other modules' default proof blocks.
    // Platform fixture account 1 holds human Person 1 and Company 1 affiliation 1.
    if workforce
        .employees(&manager, 1)
        .await
        .map_err(|_| Error::Internal)?
        .require_current()
        .map_err(|_| Error::Internal)?
        .is_empty()
    {
        services
            .get::<ReferenceData>(bilibi_people_reference_data::SERVICE)?
            .create_entry(
                &admin,
                1,
                &NewEntry::new("employment_type", "py5_type", "Payroll proof type"),
            )
            .await
            .map_err(|_| Error::Internal)?;
        workforce
            .create_employment(
                &admin,
                1,
                &NewEmployment::new(1, 1, "PY5-001", "py5_type", "active"),
            )
            .await
            .map_err(|_| Error::Internal)?;
    }

    let employee: i64 = workforce
        .employees(&manager, 1)
        .await
        .map_err(|_| Error::Internal)?
        .require_current()
        .map_err(|_| Error::Internal)?
        .first()
        .ok_or(Error::Internal)?
        .reference
        .stable_id
        .parse()
        .map_err(|_| Error::Internal)?;
    must(
        payroll
            .put_settings(&manager, 1, "proof", &["XAA".into()])
            .await,
    )?;
    let class = must(
        payroll
            .create_classification(
                &manager,
                1,
                &NewClassification {
                    code: "py5_class".into(),
                    name: "Run proof classification".into(),
                    effective_from: date(2040, 1, 1),
                    effective_to: None,
                },
            )
            .await,
    )?;
    let item = must(
        payroll
            .create_item(
                &manager,
                1,
                &NewItem {
                    code: "py5_item".into(),
                    name: "Run proof item".into(),
                    classification_id: class.id,
                    currency: "XAA".into(),
                    amount: "12.345678".parse().map_err(|_| Error::Invalid)?,
                    effective_from: date(2040, 1, 1),
                    effective_to: None,
                },
            )
            .await,
    )?;
    let leave = services.get::<Leave>(bilibi_people_leave::SERVICE)?;
    let leave_type = leave
        .create_type(
            &manager,
            1,
            &NewLeaveType::new("py5_leave", "Run proof leave", "day"),
        )
        .await
        .map_err(|_| Error::Internal)?;
    let claims = services.get::<Claims>(bilibi_people_claims::SERVICE)?;
    let category = claims
        .create_category(
            &manager,
            1,
            &NewCategory {
                code: "py5_category".into(),
                name: "Run proof category".into(),
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
                code: "py5_claim".into(),
                name: "Run proof claim".into(),
                receipt_requirement: "never".into(),
                eligibility: None,
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    for (kind, id) in [("leave", leave_type.id), ("claims", claim_type.id)] {
        must(
            payroll
                .create_mapping(
                    &manager,
                    1,
                    &NewMapping {
                        source_kind: kind.into(),
                        source_key: id.to_string(),
                        item_id: item.id,
                        effective_from: date(2040, 1, 1),
                        effective_to: None,
                    },
                )
                .await,
        )?;
    }
    services
        .get::<Attendance>(bilibi_people_attendance::SERVICE)?
        .create_allowance_rule(
            &manager,
            1,
            &NewAllowanceRule {
                code: "py5_rule".into(),
                name: "Run proof allowance".into(),
                unit: "day".into(),
                value: "1.50".parse().map_err(|_| Error::Invalid)?,
                currency: "XAA".into(),
                effective_from: date(2040, 1, 1),
                effective_until: None,
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    must(
        payroll
            .create_attendance_allowance_mapping(
                &manager,
                1,
                &NewAttendanceAllowanceMapping {
                    attendance_rule_code: "py5_rule".into(),
                    item_id: item.id,
                    effective_from: date(2040, 1, 1),
                    effective_to: None,
                },
            )
            .await,
    )?;
    let mut run_ids = vec![];
    for month in 1..=3 {
        let period = must(
            payroll
                .create_period(
                    &manager,
                    1,
                    &NewPeriod {
                        code: format!("py5_period_{month}"),
                        starts_on: date(2040, month, 1),
                        ends_on: date(2040, month, 28),
                        pay_on: date(2040, month, 28),
                    },
                )
                .await,
        )?;
        run_ids.push(must(payroll.freeze_setup(&manager, 1, period.id, "XAA").await)?.id);
    }
    let pages = services.get::<PageService<Scope, Error>>("people/payroll.run_pages")?;
    let run = run_ids[0];
    let mut page = mount(&pages, &manager, run).await?;
    let html = send(
        &mut *page,
        &manager,
        json!({"type":"clear-flash","key":"error"}),
    )
    .await?;
    prove(
        "three intake forms and frozen facts",
        html.contains("contribution-form")
            && html.contains("mapped-contribution-form")
            && html.contains("attendance-contribution-form")
            && html.contains("py5_period_1"),
    )?;
    send(&mut *page, &manager, json!({"type":"prepare_lock"})).await?;
    prove(
        "preparing lock leaves run writable",
        must(payroll.run_output(&manager, 1, run).await)?
            .run
            .locked_at
            .is_none(),
    )?;
    send(&mut *page, &manager, json!({"type":"cancel_action"})).await?;
    send(&mut *page, &manager, json!({"type":"confirm_action"})).await?;
    prove(
        "cancelled confirmation cannot be forged",
        must(payroll.run_output(&manager, 1, run).await)?
            .run
            .locked_at
            .is_none(),
    )?;
    let mut input = json!({"type":"intake","input[source_key]":"py5_direct","input[employee_id]":employee.to_string(),"input[item_id]":item.id.to_string(),"input[on_date]":"2040-01-10","input[units]":"bad","input[direction]":"earning","input[evidence]":"evidence/py5/direct"});
    let html = send(&mut *page, &manager, input.clone()).await?;
    prove(
        "decimal refusal preserves fields and gives summary",
        html.contains("must be an exact decimal") && html.contains("evidence/py5/direct"),
    )?;
    input["input[units]"] = json!("1.234567");
    let html = send(&mut *page, &manager, input.clone()).await?;
    prove(
        "direct contribution accepted",
        html.contains("Payroll action completed.") && html.contains("evidence/py5/direct"),
    )?;
    send(&mut *page, &manager, input.clone()).await?;
    prove(
        "replayed contribution is not duplicated",
        must(payroll.run_output(&manager, 1, run).await)?
            .contributions
            .len()
            == 1,
    )?;
    input["input[units]"] = json!("2");
    let html = send(&mut *page, &manager, input.clone()).await?;
    prove(
        "replay conflict is recoverable",
        html.contains(":replay_conflict"),
    )?;
    for (kind, id) in [("leave", leave_type.id), ("claims", claim_type.id)] {
        input["type"] = json!("intake_mapped");
        input["input[source_key]"] = json!(format!("py5_{kind}"));
        input["input[source_kind]"] = json!(kind);
        input["input[mapping_source]"] = json!(id.to_string());
        let html = send(&mut *page, &manager, input.clone()).await?;
        prove(
            "mapped contribution accepted",
            html.contains("Payroll action completed."),
        )?;
    }
    input["type"] = json!("intake_attendance");
    input["input[source_key]"] = json!("py5_attendance");
    input["input[mapping_source]"] = json!("py5_rule");
    input["input[direction]"] = json!("");
    let html = send(&mut *page, &manager, input.clone()).await?;
    prove(
        "allowance contribution accepted at mapped rate",
        html.contains("Payroll action completed.")
            && must(payroll.run_output(&manager, 1, run).await)?
                .contributions
                .len()
                == 4,
    )?;
    send(&mut *page, &manager, json!({"type":"prepare_lock"})).await?;
    grants.set_direct(&admin, 1, MANAGE, Some(false)).await?;
    let html = send(&mut *page, &manager, json!({"type":"confirm_action"})).await?;
    prove(
        "revoked grant refuses pending lock",
        html.replace("&#39;", "'")
            .replace("&#x27;", "'")
            .contains("You cannot change this company's payroll runs.")
            && must(payroll.run_output(&manager, 1, run).await)?
                .run
                .locked_at
                .is_none(),
    )?;
    grants.set_direct(&admin, 1, MANAGE, Some(true)).await?;
    send(&mut *page, &manager, json!({"type":"prepare_lock"})).await?;
    send(
        &mut *page,
        &manager,
        json!({"type":"confirm_action","run_id":run_ids[1].to_string()}),
    )
    .await?;
    prove(
        "confirmation ignores replacement run id",
        must(payroll.run_output(&manager, 1, run).await)?
            .run
            .locked_at
            .is_some()
            && must(payroll.run_output(&manager, 1, run_ids[1]).await)?
                .run
                .locked_at
                .is_none(),
    )?;
    let html = send(&mut *page, &manager, json!({"type":"calculate"})).await?;
    prove(
        "calculation requires confirmation",
        html.contains("payroll-calculate-confirm")
            && must(payroll.run_output(&manager, 1, run).await)?
                .calculation
                .is_none(),
    )?;
    let html = send(&mut *page, &manager, json!({"type":"confirm_action"})).await?;
    prove(
        "exact results, digest, totals and closed intake",
        html.contains("15.241566651426")
            && html.contains("Replay digest:")
            && html.contains("Employee totals")
            && !html.contains("id=\"contribution-form\""),
    )?;
    send(
        &mut *page,
        &manager,
        json!({"type":"decide","decision[outcome]":"approved","decision[reason]":"Own work"}),
    )
    .await?;
    let html = send(&mut *page, &manager, json!({"type":"confirm_action"})).await?;
    prove(
        "self approval refused",
        html.contains(":not_decidable")
            && must(payroll.run_output(&manager, 1, run).await)?
                .decision
                .is_none(),
    )?;
    let mut reviewer = mount(&pages, &admin, run).await?;
    let html=send(&mut *reviewer,&admin,json!({"type":"decide","decision[outcome]":"approved","decision[reason]":"Independent review"})).await?;
    prove(
        "approval holds reason without writing",
        html.contains("Independent review")
            && html.contains("payroll-decision-confirm")
            && must(payroll.run_output(&admin, 1, run).await)?
                .decision
                .is_none(),
    )?;
    send(&mut *reviewer,&admin,json!({"type":"confirm_action","decision[outcome]":"rejected","decision[reason]":"replacement"})).await?;
    let decision = must(payroll.run_output(&admin, 1, run).await)?
        .decision
        .ok_or(Error::Internal)?;
    prove(
        "approval uses server-held outcome and reason",
        decision.outcome == "approved" && decision.reason == "Independent review",
    )?;
    // Configure private storage through Base Settings, just for this disposable proof.
    let root = std::env::var("BILIBI_PAYROLL_RUN_PROOF_OUT").map_err(|_| Error::Invalid)?;
    std::fs::create_dir_all(&root).map_err(|_| Error::Internal)?;
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
        .map_err(|_| Error::Internal)?;
    let mut c = db.get().await?;
    let mut tx = services
        .get::<MutationBoundary>("base/database.mutations")?
        .begin(&mut c, admin.mutation_context("walkthrough.py5.storage"))
        .await?;
    for (key, value) in [
        ("artifacts.storage_root", json!(root)),
        ("artifacts.retention_days", json!(30)),
    ] {
        services
            .get::<Settings>("base/settings.store")?
            .put(&mut tx, &admin, Target::Global, key, value)
            .await?;
    }
    tx.commit().await?;
    drop(c);
    let html = send(&mut *page, &manager, json!({"type":"generate_report"})).await?;
    prove(
        "report generation through page",
        html.contains("Download PDF") && html.contains("Payroll action completed."),
    )?;
    send(
        &mut *page,
        &manager,
        json!({"type":"generate_payslip","employee":employee.to_string()}),
    )
    .await?;
    let output = must(payroll.run_output(&manager, 1, run).await)?;
    prove("report and payslip records", output.documents.len() == 2)?;
    for doc in &output.documents {
        prove(
            "download is a PDF",
            must(payroll.read_document(&manager, 1, &doc.artifact_id).await)?
                .bytes
                .starts_with(b"%PDF-"),
        )?;
    }
    let mut readonly = mount(&pages, &viewer, run).await?;
    let html = send(&mut *readonly, &viewer, json!({"type":"generate_report"})).await?;
    prove(
        "read-only controls withheld and forged write refused",
        html.contains("Read-only payroll access")
            && html
                .replace("&#39;", "'")
                .replace("&#x27;", "'")
                .contains("You cannot change this company's payroll runs.")
            && !html.contains("data-event=\"generate_report\""),
    )?;
    // Leave the third run uncalculated for the browser's full workflow.
    let second = run_ids[1];
    let mut rejected = mount(&pages, &manager, second).await?;
    input["type"] = json!("intake");
    input["input[source_key]"] = json!("py5_rejected");
    input["input[on_date]"] = json!("2040-02-10");
    input["input[direction]"] = json!("earning");
    send(&mut *rejected, &manager, input).await?;
    for event in [
        "prepare_lock",
        "confirm_action",
        "calculate",
        "confirm_action",
    ] {
        send(&mut *rejected, &manager, json!({"type":event})).await?;
    }
    let mut reviewer = mount(&pages, &admin, second).await?;
    send(&mut *reviewer,&admin,json!({"type":"decide","decision[outcome]":"rejected","decision[reason]":"Independent rejection"})).await?;
    let html = send(&mut *reviewer, &admin, json!({"type":"confirm_action"})).await?;
    prove(
        "rejection is final and withholds document generation",
        html.contains("Independent rejection") && !html.contains("data-event=\"generate_report\""),
    )?;
    println!("Browser run: /people/payroll/runs?company_id=1&run_id={} ; employee={employee}; item={}; date=2040-03-10",run_ids[2],item.id);
    Ok(())
}
