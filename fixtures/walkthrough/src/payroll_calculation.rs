//! PY2: executable proof over PY1's prepared runs, through the public facade.
use super::{ADMIN, COMPANY, MANAGER, STAFF, TENANT, VIEWER};
use bilibi_authz::grants::Grants;
use bilibi_database::{pool, Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_payroll::{
    Calculation, NewContribution, Payroll, PayrollError as Refusal, APPROVE, MANAGE, VIEW,
};
use bilibi_people_workforce::{EmploymentChange, Workforce};
use bilibi_tenancy::Authentication;
use rust_decimal::Decimal;
use serde_json::Value;
use std::sync::Arc;
fn check(label: &str, yes: bool) -> Result<()> {
    if yes {
        println!("{label}: proved");
        Ok(())
    } else {
        eprintln!("{label}: FAILED");
        Err(Error::Internal)
    }
}
fn must<T>(result: bilibi_people_payroll::Result<T>) -> Result<T> {
    result.map_err(|e| {
        eprintln!("Payroll: {e:?}");
        Error::Internal
    })
}
fn denied<T>(
    label: &str,
    result: bilibi_people_payroll::Result<T>,
    predicate: fn(&Refusal) -> bool,
) -> Result<()> {
    match result {
        Err(e) if predicate(&e) => {
            println!("{label}: refused ({e})");
            Ok(())
        }
        Err(e) => {
            eprintln!("{label}: unexpected {e:?}");
            Err(Error::Internal)
        }
        Ok(_) => {
            eprintln!("{label}: unexpectedly accepted");
            Err(Error::Internal)
        }
    }
}
async fn sql_denied(c: &tokio_postgres::Client, label: &str, sql: &str, id: i64) -> Result<()> {
    let error = c.execute(sql, &[&id]).await.err().ok_or(Error::Internal)?;
    check(
        label,
        error
            .code()
            .is_some_and(|c| matches!(c.code(), "23514" | "23503" | "23505")),
    )
}
fn sorted_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort();
            format!(
                "{{{}}}",
                keys.into_iter()
                    .map(|k| format!(
                        "{}:{}",
                        serde_json::to_string(k).expect("JSON key"),
                        sorted_json(&map[k])
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        Value::Array(a) => format!(
            "[{}]",
            a.iter().map(sorted_json).collect::<Vec<_>>().join(",")
        ),
        _ => value.to_string(),
    }
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
    // Match the Platform walkthrough's explicitly selected development mode.
    if std::env::var("BILIBI_ENV").as_deref() == Ok("development") {
        services
            .get::<bilibi_base::step_up::StepUp>(bilibi_base::step_up::SERVICE)?
            .set_enforced(false);
    }
    let auth = services.get::<Authentication>("base/tenancy.authentication")?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    let payroll = services.get::<Payroll>(bilibi_people_payroll::SERVICE)?;
    let workforce = services.get::<Workforce>(bilibi_people_workforce::SERVICE)?;
    let c = db.get().await?;
    let admin = auth
        .authenticate(&**c, TENANT, ADMIN, "PY2 proof".into())
        .await?;
    let manager = auth
        .authenticate(&**c, TENANT, MANAGER, "PY2 proof".into())
        .await?;
    let staff = auth
        .authenticate(&**c, TENANT, STAFF, "PY2 proof".into())
        .await?;
    let viewer = auth
        .authenticate(&**c, TENANT, VIEWER, "PY2 proof".into())
        .await?;
    denied(
        "no Payroll grant",
        payroll.run_output(&viewer, COMPANY, 1).await,
        |e| matches!(e, Refusal::Unauthorized),
    )?;
    for actor in [STAFF, VIEWER] {
        for key in [MANAGE, VIEW, APPROVE] {
            grants.set_direct(&admin, actor, key, Some(true)).await?;
        }
    }
    let setup = must(payroll.setup(&manager, COMPANY).await)?;
    let run = setup
        .runs
        .iter()
        .find(|r| r.snapshot["period"]["code"] == "py1-period")
        .ok_or(Error::Internal)?;
    let future = setup
        .runs
        .iter()
        .find(|r| r.snapshot["period"]["code"] == "py1-next-period")
        .ok_or(Error::Internal)?;
    let item = setup
        .items
        .iter()
        .find(|i| i.code == "py1-item")
        .ok_or(Error::Internal)?;
    let employees = workforce
        .employees(&manager, COMPANY)
        .await
        .map_err(|_| Error::Internal)?
        .require_current()
        .map_err(|_| Error::Internal)?;
    let employee = employees
        .first()
        .ok_or(Error::Internal)?
        .reference
        .stable_id
        .parse::<i64>()
        .map_err(|_| Error::Internal)?;
    let input = NewContribution {
        employee_id: employee,
        item_id: item.id,
        source_key: "py2-earning".into(),
        evidence: "attestation:py2".into(),
        on_date: run.snapshot["period"]["starts_on"]
            .as_str()
            .ok_or(Error::Internal)?
            .parse()
            .map_err(|_| Error::Invalid)?,
        units: Decimal::from_str_exact("1.234567").map_err(|_| Error::Invalid)?,
        direction: "earning".into(),
    };
    denied(
        "empty locked run",
        payroll.calculate(&viewer, COMPANY, run.id).await,
        |e| matches!(e, Refusal::NoContributions),
    )?;
    denied(
        "unlocked run",
        payroll.calculate(&viewer, COMPANY, future.id).await,
        |e| matches!(e, Refusal::RunNotLocked),
    )?;
    for (label, value) in [
        ("zero units", "0"),
        ("negative units", "-1"),
        ("fractional precision", "0.0000001"),
        ("unit bound", "100000000000000"),
    ] {
        denied(
            label,
            payroll
                .intake(
                    &staff,
                    COMPANY,
                    run.id,
                    &NewContribution {
                        units: Decimal::from_str_exact(value).map_err(|_| Error::Invalid)?,
                        ..input.clone()
                    },
                )
                .await,
            |e| matches!(e, Refusal::Invalid(_)),
        )?;
    }
    denied(
        "unknown employee",
        payroll
            .intake(
                &staff,
                COMPANY,
                run.id,
                &NewContribution {
                    employee_id: 99999999,
                    ..input.clone()
                },
            )
            .await,
        |e| matches!(e, Refusal::IntakeUnavailable),
    )?;
    denied(
        "unfrozen item",
        payroll
            .intake(
                &staff,
                COMPANY,
                run.id,
                &NewContribution {
                    item_id: 99999999,
                    ..input.clone()
                },
            )
            .await,
        |e| matches!(e, Refusal::IntakeUnavailable),
    )?;
    denied(
        "date outside run",
        payroll
            .intake(
                &staff,
                COMPANY,
                run.id,
                &NewContribution {
                    on_date: input.on_date.yesterday().map_err(|_| Error::Invalid)?,
                    ..input.clone()
                },
            )
            .await,
        |e| matches!(e, Refusal::IntakeUnavailable),
    )?;
    denied(
        "missing evidence",
        payroll
            .intake(
                &staff,
                COMPANY,
                run.id,
                &NewContribution {
                    evidence: " ".into(),
                    ..input.clone()
                },
            )
            .await,
        |e| matches!(e, Refusal::Invalid(_)),
    )?;
    denied(
        "unknown direction",
        payroll
            .intake(
                &staff,
                COMPANY,
                run.id,
                &NewContribution {
                    direction: "other".into(),
                    ..input.clone()
                },
            )
            .await,
        |e| matches!(e, Refusal::Invalid(_)),
    )?;
    let first = must(payroll.intake(&staff, COMPANY, run.id, &input).await)?;
    let replay = must(payroll.intake(&staff, COMPANY, run.id, &input).await)?;
    check("same key returns same contribution", first.id == replay.id)?;
    for changed in [
        NewContribution {
            units: Decimal::ONE,
            ..input.clone()
        },
        NewContribution {
            evidence: "different".into(),
            ..input.clone()
        },
        NewContribution {
            direction: "deduction".into(),
            ..input.clone()
        },
    ] {
        denied(
            "key conflicts with changed attestation",
            payroll.intake(&staff, COMPANY, run.id, &changed).await,
            |e| matches!(e, Refusal::ReplayConflict),
        )?;
    }
    let future_input = NewContribution {
        on_date: future.snapshot["period"]["starts_on"]
            .as_str()
            .ok_or(Error::Internal)?
            .parse()
            .map_err(|_| Error::Invalid)?,
        ..input.clone()
    };
    denied(
        "contribution key is company-wide",
        payroll
            .intake(&staff, COMPANY, future.id, &future_input)
            .await,
        |e| matches!(e, Refusal::ReplayConflict),
    )?;
    let deduction = NewContribution {
        source_key: "py2-deduction".into(),
        units: Decimal::from_str_exact("0.000001").map_err(|_| Error::Invalid)?,
        direction: "deduction".into(),
        ..input.clone()
    };
    let mapping = &run.snapshot["mappings"][0];
    must(
        payroll
            .intake_mapped(
                &staff,
                COMPANY,
                run.id,
                mapping["source_kind"].as_str().ok_or(Error::Internal)?,
                mapping["source_key"].as_str().ok_or(Error::Internal)?,
                &NewContribution {
                    item_id: 0,
                    ..deduction.clone()
                },
            )
            .await,
    )?;
    denied(
        "missing mapping",
        payroll
            .intake_mapped(&staff, COMPANY, run.id, "leave", "missing", &input)
            .await,
        |e| matches!(e, Refusal::SourceNotMapped),
    )?;
    let allowance = NewContribution {
        source_key: "py2-allowance".into(),
        units: Decimal::ONE,
        item_id: 0,
        ..input.clone()
    };
    denied(
        "allowance cannot deduct",
        payroll
            .intake_attendance_allowance(
                &staff,
                COMPANY,
                run.id,
                "py1-rule",
                &NewContribution {
                    direction: "deduction".into(),
                    ..allowance.clone()
                },
            )
            .await,
        |e| matches!(e, Refusal::DirectionConflict),
    )?;
    denied(
        "missing allowance mapping",
        payroll
            .intake_attendance_allowance(&staff, COMPANY, run.id, "missing", &allowance)
            .await,
        |e| matches!(e, Refusal::AllowanceNotMapped),
    )?;
    let accepted = must(
        payroll
            .intake_attendance_allowance(&staff, COMPANY, run.id, "py1-rule", &allowance)
            .await,
    )?;
    check(
        "allowance resolves item and earning",
        accepted.item_id == item.id && accepted.direction == "earning",
    )?;
    let employer = NewContribution {
        source_key: "py2-employer".into(),
        units: Decimal::ONE,
        direction: "employer".into(),
        ..input.clone()
    };
    // Two connections racing an identical key produce one row and one replay.
    let (a, b) = tokio::join!(
        payroll.intake(&staff, COMPANY, run.id, &employer),
        payroll.intake(&staff, COMPANY, run.id, &employer)
    );
    check("concurrent intake serializes", must(a)?.id == must(b)?.id)?;
    let employment = workforce
        .employment(&manager, COMPANY, employee)
        .await
        .map_err(|_| Error::Internal)?
        .require_current()
        .map_err(|_| Error::Internal)?;
    let inactive = workforce
        .update_employment(
            &manager,
            COMPANY,
            employee,
            &EmploymentChange {
                status: Some("inactive".into()),
                ..EmploymentChange::at(employment.version)
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    denied(
        "membership rechecked at calculation",
        payroll.calculate(&viewer, COMPANY, run.id).await,
        |e| matches!(e, Refusal::EmployeeNotFound),
    )?;
    workforce
        .update_employment(
            &manager,
            COMPANY,
            employee,
            &EmploymentChange {
                status: Some(employment.status),
                ..EmploymentChange::at(inactive.version)
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    let (a, b) = tokio::join!(
        payroll.calculate(&viewer, COMPANY, run.id),
        payroll.calculate(&viewer, COMPANY, run.id)
    );
    let calculation = must(a)?;
    check(
        "concurrent calculation replays stored result",
        calculation.id == must(b)?.id,
    )?;
    let output = must(payroll.run_output(&manager, COMPANY, run.id).await)?;
    check(
        "every contribution has one persisted result",
        output.entries.len() == 4 && output.contributions.len() == 4,
    )?;
    let lines = calculation.snapshot["result"]["lines"]
        .as_array()
        .ok_or(Error::Internal)?;
    let earning = lines
        .iter()
        .find(|l| l["source_key"] == "py2-earning")
        .ok_or(Error::Internal)?;
    check(
        "six by six places remain exact",
        earning["amount"] == "15.241566651426",
    )?;
    check(
        "allowance uses frozen pay rate",
        lines
            .iter()
            .any(|l| l["source_key"] == "py2-allowance" && l["amount"] == "12.345678000000"),
    )?;
    check(
        "net excludes employer",
        calculation.snapshot["result"]["totals"][0]["net"] == "27.587232305748",
    )?;
    let digest: String = c
        .query_one(
            "SELECT encode(sha256(convert_to($1,'UTF8')),'hex')",
            &[&sorted_json(&calculation.snapshot)],
        )
        .await?
        .get(0);
    check(
        "canonical SHA-256 independently reproduced by PostgreSQL",
        digest == calculation.digest,
    )?;
    let before:i64=c.query_one("SELECT count(*) FROM base_audit_mutations WHERE auditable_type='people_payroll_calculations'",&[]).await?.get(0);
    let retry: Calculation = must(payroll.calculate(&staff, COMPANY, run.id).await)?;
    let after:i64=c.query_one("SELECT count(*) FROM base_audit_mutations WHERE auditable_type='people_payroll_calculations'",&[]).await?.get(0);
    check(
        "calculation retry preserves snapshot digest actor and audit",
        retry.id == calculation.id
            && retry.snapshot == calculation.snapshot
            && retry.digest == calculation.digest
            && retry.created_by_actor_id == VIEWER
            && before == after,
    )?;
    denied(
        "intake closes after calculation",
        payroll.intake(&staff, COMPANY, run.id, &input).await,
        |e| matches!(e, Refusal::IntakeUnavailable),
    )?;
    for (label, actor) in [
        ("creator and locker cannot approve", &manager),
        ("contributor cannot approve", &staff),
        ("calculator cannot approve", &viewer),
    ] {
        denied(
            label,
            payroll
                .approve_run(actor, COMPANY, run.id, "reviewed")
                .await,
            |e| matches!(e, Refusal::NotDecidable),
        )?;
    }
    denied(
        "decision needs a reason",
        payroll.approve_run(&admin, COMPANY, run.id, " ").await,
        |e| matches!(e, Refusal::InvalidDecision),
    )?;
    sql_denied(&c,"database independently refuses the creator's decision","INSERT INTO people_payroll_decisions (tenant_id,company_id,created_by_actor_id,run_id,outcome,reason) SELECT tenant_id,company_id,created_by_actor_id,id,'approved','self decision' FROM people_payroll_setup_snapshots WHERE id=$1",run.id).await?;
    sql_denied(&c,"database refuses documents before approval","INSERT INTO people_payroll_documents (tenant_id,company_id,created_by_actor_id,run_id,artifact_id,kind) SELECT tenant_id,company_id,created_by_actor_id,id,gen_random_uuid(),'report' FROM people_payroll_setup_snapshots WHERE id=$1",run.id).await?;
    let approved = must(
        payroll
            .approve_run(&admin, COMPANY, run.id, " Independently checked ")
            .await,
    )?;
    check(
        "independent decision trims reason",
        approved.created_by_actor_id == ADMIN && approved.reason == "Independently checked",
    )?;
    denied(
        "decision cannot be replaced",
        payroll.reject_run(&admin, COMPANY, run.id, "changed").await,
        |e| matches!(e, Refusal::NotDecidable),
    )?;
    let events=c.query("SELECT version,payload FROM base_events WHERE tenant_id=$1 AND name='people/payroll.run_approved'",&[&TENANT]).await?;
    check(
        "approval records exactly one v1 event",
        events.len() == 1 && events[0].get::<_, i32>(0) == 1,
    )?;
    let payload: Value = events[0].get(1);
    check(
        "event has exact sorted components and frozen currency",
        payload["run_id"] == run.id
            && payload["company_id"] == COMPANY
            && payload["currency"] == "XAA"
            && payload["components"]
                .as_array()
                .is_some_and(|a| a.len() == 3)
            && payload["components"][1]["amount"] == "27.587244651426",
    )?;
    println!("APPROVAL_EVENT={payload}");
    // A second run separates the locker role, and proves rejection emits nothing.
    must(
        payroll
            .intake(
                &manager,
                COMPANY,
                future.id,
                &NewContribution {
                    source_key: "py2-rejected".into(),
                    ..future_input.clone()
                },
            )
            .await,
    )?;
    must(payroll.lock_run(&staff, COMPANY, future.id).await)?;
    must(payroll.calculate(&manager, COMPANY, future.id).await)?;
    denied(
        "locker alone cannot decide",
        payroll
            .reject_run(&staff, COMPANY, future.id, "reviewed")
            .await,
        |e| matches!(e, Refusal::NotDecidable),
    )?;
    let rejected = must(
        payroll
            .reject_run(&viewer, COMPANY, future.id, "Source evidence refused")
            .await,
    )?;
    check(
        "independent rejection is final",
        rejected.outcome == "rejected",
    )?;
    let count:i64=c.query_one("SELECT count(*) FROM base_events WHERE tenant_id=$1 AND name='people/payroll.run_approved'",&[&TENANT]).await?.get(0);
    check("rejection publishes no event", count == 1)?;
    for (label,sql,id) in [
        ("immutable contribution","UPDATE people_payroll_contributions SET evidence='changed' WHERE id=$1",first.id),
        ("immutable calculation","DELETE FROM people_payroll_calculations WHERE id=$1",calculation.id),
        ("immutable result","UPDATE people_payroll_calculation_entries SET amount=0 WHERE id=$1",output.entries[0].id),
        ("immutable decision","DELETE FROM people_payroll_decisions WHERE id=$1",approved.id),
        ("late SQL contribution","INSERT INTO people_payroll_contributions (tenant_id,company_id,created_by_actor_id,run_id,employee_id,item_id,source_key,evidence,on_date,units,direction) SELECT tenant_id,company_id,created_by_actor_id,run_id,employee_id,item_id,'py2-late',evidence,on_date,units,direction FROM people_payroll_contributions WHERE id=$1",first.id),
        ("late SQL result","INSERT INTO people_payroll_calculation_entries (tenant_id,company_id,created_by_actor_id,run_id,employee_id,contribution_id,direction,amount) SELECT tenant_id,company_id,created_by_actor_id,run_id,employee_id,contribution_id,direction,amount FROM people_payroll_calculation_entries WHERE id=$1",output.entries[0].id),
        ("rejected document refused","INSERT INTO people_payroll_documents (tenant_id,company_id,created_by_actor_id,run_id,artifact_id,kind) SELECT tenant_id,company_id,created_by_actor_id,run_id,gen_random_uuid(),'report' FROM people_payroll_decisions WHERE id=$1",rejected.id),
    ] {sql_denied(&c,label,sql,id).await?;}
    grants
        .set_direct(&admin, STAFF, MANAGE, Some(false))
        .await?;
    denied(
        "revocation refuses replay",
        payroll.calculate(&staff, COMPANY, run.id).await,
        |e| matches!(e, Refusal::Unauthorized),
    )?;
    bounds_and_pages(
        &services,
        &manager,
        &admin,
        &viewer,
        payroll.as_ref(),
        workforce.as_ref(),
        employee,
        future_input.on_date,
    )
    .await?;
    println!(
        "Payroll calculation proof complete: run {}, calculation {}, digest {}",
        run.id, calculation.id, calculation.digest
    );
    Ok(())
}

/// Exercise the numeric boundary and a real lookup spanning two 1,000-ID pages.
#[allow(clippy::too_many_arguments)]
async fn bounds_and_pages(
    services: &Services,
    manager: &bilibi_tenancy::Scope,
    admin: &bilibi_tenancy::Scope,
    reviewer: &bilibi_tenancy::Scope,
    payroll: &Payroll,
    workforce: &Workforce,
    initial_employee: i64,
    previous_start: jiff::civil::Date,
) -> Result<()> {
    use bilibi_affiliation::{
        AffiliationInput, AffiliationService, EffectiveDates, OrganisationId,
    };
    use bilibi_people_payroll::{NewClassification, NewItem, NewPeriod};
    use bilibi_people_workforce::NewEmployment;
    use bilibi_person::{PersonInput, PersonKind, PersonService};
    let from = jiff::civil::date(previous_start.year() + 2, 1, 1);
    must(
        payroll
            .put_settings(manager, COMPANY, "proof-jurisdiction", &["XAA".into()])
            .await,
    )?;
    let class = must(
        payroll
            .create_classification(
                manager,
                COMPANY,
                &NewClassification {
                    code: "py2-bounds".into(),
                    name: "Calculation boundary proof".into(),
                    effective_from: from,
                    effective_to: None,
                },
            )
            .await,
    )?;
    let high = must(
        payroll
            .create_item(
                manager,
                COMPANY,
                &NewItem {
                    code: "py2-high-rate".into(),
                    name: "Decimal boundary proof".into(),
                    classification_id: class.id,
                    currency: "XAA".into(),
                    amount: Decimal::from_str_exact("99999999999999.999999")
                        .map_err(|_| Error::Invalid)?,
                    effective_from: from,
                    effective_to: None,
                },
            )
            .await,
    )?;
    let small = must(
        payroll
            .create_item(
                manager,
                COMPANY,
                &NewItem {
                    code: "py2-small-rate".into(),
                    name: "Membership proof".into(),
                    classification_id: class.id,
                    currency: "XAA".into(),
                    amount: Decimal::from_str_exact("0.000001").map_err(|_| Error::Invalid)?,
                    effective_from: from,
                    effective_to: None,
                },
            )
            .await,
    )?;
    let later = from.tomorrow().map_err(|_| Error::Invalid)?;
    let later_item = must(
        payroll
            .create_item(
                manager,
                COMPANY,
                &NewItem {
                    code: "py2-later-version".into(),
                    name: "Effective date proof".into(),
                    classification_id: class.id,
                    currency: "XAA".into(),
                    amount: Decimal::ONE,
                    effective_from: later,
                    effective_to: None,
                },
            )
            .await,
    )?;
    let period = must(
        payroll
            .create_period(
                manager,
                COMPANY,
                &NewPeriod {
                    code: "py2-membership".into(),
                    starts_on: from,
                    ends_on: later,
                    pay_on: later,
                },
            )
            .await,
    )?;
    let run = must(
        payroll
            .freeze_setup(manager, COMPANY, period.id, "XAA")
            .await,
    )?;
    let input = NewContribution {
        employee_id: initial_employee,
        item_id: high.id,
        source_key: "py2-too-large".into(),
        evidence: "attestation:range".into(),
        on_date: from,
        units: Decimal::from(1000),
        direction: "earning".into(),
    };
    denied(
        "unrepresentable twelve-place product",
        payroll.intake(manager, COMPANY, run.id, &input).await,
        |e| matches!(e, Refusal::DecimalRange),
    )?;
    denied(
        "frozen item must be effective on the contribution date",
        payroll
            .intake(
                manager,
                COMPANY,
                run.id,
                &NewContribution {
                    item_id: later_item.id,
                    source_key: "py2-item-date".into(),
                    units: Decimal::ONE,
                    ..input.clone()
                },
            )
            .await,
        |e| matches!(e, Refusal::IntakeUnavailable),
    )?;
    let precision = must(
        payroll
            .intake(
                manager,
                COMPANY,
                run.id,
                &NewContribution {
                    source_key: "py2-high-exact".into(),
                    units: Decimal::ONE,
                    ..input.clone()
                },
            )
            .await,
    )?;
    check(
        "large representable product accepted",
        precision.units == "1.000000",
    )?;
    let persons = services.get::<PersonService>(bilibi_person::SERVICE)?;
    let affiliations = services.get::<AffiliationService>(bilibi_affiliation::SERVICE)?;
    let mut last = None;
    for i in 0..1001 {
        let person = persons
            .create(
                manager,
                &PersonInput {
                    name: format!("Payroll membership proof {i}"),
                    kind: PersonKind::Human,
                    email: None,
                    phone: None,
                },
            )
            .await
            .map_err(|e| {
                eprintln!("page person: {e}");
                Error::Internal
            })?;
        let affiliation = affiliations
            .create(
                manager,
                &AffiliationInput {
                    person_id: person.id,
                    organisation_id: OrganisationId::new(COMPANY).map_err(|_| Error::Invalid)?,
                    title: None,
                    work_email: None,
                    work_phone: None,
                    dates: EffectiveDates::new(None, None).map_err(|_| Error::Invalid)?,
                },
            )
            .await?;
        let employee = workforce
            .create_employment(
                manager,
                COMPANY,
                &NewEmployment::new(
                    person.id.get(),
                    affiliation.id.get(),
                    format!("PY2-{i:04}"),
                    "full_time",
                    "active",
                ),
            )
            .await
            .map_err(|e| {
                eprintln!("page employee {i}: {e}");
                Error::Internal
            })?;
        must(
            payroll
                .intake(
                    manager,
                    COMPANY,
                    run.id,
                    &NewContribution {
                        employee_id: employee.id,
                        item_id: small.id,
                        source_key: format!("py2-page-{i:04}"),
                        units: Decimal::from_str_exact("0.000001").map_err(|_| Error::Invalid)?,
                        ..input.clone()
                    },
                )
                .await,
        )?;
        last = Some(employee);
    }
    let last = last.ok_or(Error::Internal)?;
    must(payroll.lock_run(manager, COMPANY, run.id).await)?;
    let inactive = workforce
        .update_employment(
            manager,
            COMPANY,
            last.id,
            &EmploymentChange {
                status: Some("inactive".into()),
                ..EmploymentChange::at(last.version)
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    denied(
        "tail of the second membership page is rechecked",
        payroll.calculate(manager, COMPANY, run.id).await,
        |e| matches!(e, Refusal::EmployeeNotFound),
    )?;
    workforce
        .update_employment(
            manager,
            COMPANY,
            last.id,
            &EmploymentChange {
                status: Some("active".into()),
                ..EmploymentChange::at(inactive.version)
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    let calculation = must(payroll.calculate(manager, COMPANY, run.id).await)?;
    check(
        "all 1002 employees calculated across lookup pages",
        calculation.snapshot["result"]["totals"]
            .as_array()
            .is_some_and(|v| v.len() == 1002),
    )?;
    must(
        payroll
            .approve_run(
                reviewer,
                COMPANY,
                run.id,
                "Membership pages and exact amounts checked",
            )
            .await,
    )?;
    // Distinct components must respect Base Events' array bound before a
    // decision is written. Keep this a separate run with its own period.
    let next = later.tomorrow().map_err(|_| Error::Invalid)?;
    let mut items = Vec::new();
    for i in 0..101 {
        items.push(must(
            payroll
                .create_item(
                    manager,
                    COMPANY,
                    &NewItem {
                        code: format!("py2-component-{i:03}"),
                        name: format!("Event component proof {i}"),
                        classification_id: class.id,
                        currency: "XAA".into(),
                        amount: Decimal::ONE,
                        effective_from: from,
                        effective_to: None,
                    },
                )
                .await,
        )?);
    }
    let period = must(
        payroll
            .create_period(
                manager,
                COMPANY,
                &NewPeriod {
                    code: "py2-event-bound".into(),
                    starts_on: next,
                    ends_on: next,
                    pay_on: next,
                },
            )
            .await,
    )?;
    let run = must(
        payroll
            .freeze_setup(manager, COMPANY, period.id, "XAA")
            .await,
    )?;
    for (i, item) in items.into_iter().enumerate() {
        must(
            payroll
                .intake(
                    manager,
                    COMPANY,
                    run.id,
                    &NewContribution {
                        item_id: item.id,
                        source_key: format!("py2-component-{i:03}"),
                        on_date: next,
                        units: Decimal::ONE,
                        ..input.clone()
                    },
                )
                .await,
        )?;
    }
    must(payroll.lock_run(manager, COMPANY, run.id).await)?;
    must(payroll.calculate(manager, COMPANY, run.id).await)?;
    denied(
        "oversized approval event refuses before decision",
        payroll
            .approve_run(admin, COMPANY, run.id, "reviewed")
            .await,
        |e| matches!(e, Refusal::ApprovalTooLarge),
    )?;
    check(
        "refused approval leaves no decision",
        must(payroll.run_output(manager, COMPANY, run.id).await)?
            .decision
            .is_none(),
    )?;
    must(
        payroll
            .reject_run(admin, COMPANY, run.id, "Too many components")
            .await,
    )?;
    Ok(())
}

walkthrough_command!("payroll_calculation", |url, _argument| async move {
    run(url).await
});
