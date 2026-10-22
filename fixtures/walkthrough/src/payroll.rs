//! PY1 executable proof: actual owner facades, transactions and PostgreSQL guards.
use super::{ADMIN, COMPANY, MANAGER, OTHER_TENANT_COMPANY, TENANT, VIEWER};
use bilibi_authz::grants::Grants;
use bilibi_database::{pool, Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_attendance::{Attendance, NewAllowanceRule};
use bilibi_people_claims::{Claims, NewCategory, NewClaimType};
use bilibi_people_leave::{Leave, NewLeaveType};
use bilibi_people_payroll::{
    NewAttendanceAllowanceMapping, NewClassification, NewItem, NewMapping, NewPeriod, Payroll,
    PayrollError as Refusal, CAPABILITIES, MANAGE,
};
use bilibi_tenancy::Authentication;
use jiff::civil::date;
use rust_decimal::Decimal;
use serde_json::json;
use std::sync::Arc;
fn must<T: std::fmt::Debug>(label: &str, result: bilibi_people_payroll::Result<T>) -> Result<T> {
    match result {
        Ok(v) => {
            println!("{label}: {v:?}");
            Ok(v)
        }
        Err(e) => {
            eprintln!("{label}: {e:?}");
            Err(Error::Internal)
        }
    }
}
fn check(label: &str, yes: bool) -> Result<()> {
    if yes {
        println!("{label}: proved");
        Ok(())
    } else {
        eprintln!("{label}: FAILED");
        Err(Error::Internal)
    }
}
fn denied<T>(
    label: &str,
    result: bilibi_people_payroll::Result<T>,
    expected: fn(&Refusal) -> bool,
) -> Result<()> {
    match result {
        Err(e) if expected(&e) => {
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
            .is_some_and(|c| c.code() == "23514" || c.code() == "23503"),
    )
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
    if std::env::var("BILIBI_ENV").as_deref() == Ok("development") {
        services
            .get::<bilibi_base::step_up::StepUp>(bilibi_base::step_up::SERVICE)?
            .set_enforced(false);
    }
    let auth = services.get::<Authentication>("base/tenancy.authentication")?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    let payroll = services.get::<Payroll>(bilibi_people_payroll::SERVICE)?;
    let leave = services.get::<Leave>(bilibi_people_leave::SERVICE)?;
    let claims = services.get::<Claims>(bilibi_people_claims::SERVICE)?;
    let attendance = services.get::<Attendance>(bilibi_people_attendance::SERVICE)?;
    let c = db.get().await?;
    let admin = auth
        .authenticate(&**c, TENANT, ADMIN, "payroll proof".into())
        .await?;
    let manager = auth
        .authenticate(&**c, TENANT, MANAGER, "payroll proof".into())
        .await?;
    let viewer = auth
        .authenticate(&**c, TENANT, VIEWER, "payroll proof".into())
        .await?;
    for key in CAPABILITIES.into_iter().chain([
        bilibi_people_attendance::ALLOWANCES_MANAGE,
        bilibi_people_leave::POLICIES_MANAGE,
        bilibi_people_leave::SELF_VIEW,
        bilibi_people_claims::MANAGE,
        bilibi_people_claims::SUBMIT,
    ]) {
        grants.set_direct(&admin, MANAGER, key, Some(true)).await?;
    }
    let initial = must("read empty setup", payroll.setup(&manager, COMPANY).await)?;
    check(
        "settings default empty",
        initial.country.is_empty() && initial.currencies.is_empty(),
    )?;
    denied(
        "viewer cannot read setup",
        payroll.setup(&viewer, COMPANY).await,
        |e| matches!(e, Refusal::Unauthorized),
    )?;
    denied(
        "foreign company refused",
        payroll.setup(&manager, OTHER_TENANT_COMPANY).await,
        |e| matches!(e, Refusal::NotFound | Refusal::Unauthorized),
    )?;
    let impersonated = auth
        .authenticate_session(&**c, TENANT, MANAGER, Some(ADMIN), "payroll proof".into())
        .await?;
    denied(
        "impersonated write refused",
        payroll
            .put_settings(&impersonated, COMPANY, "proof", &["XAA".into()])
            .await,
        |e| matches!(e, Refusal::Unauthorized),
    )?;
    denied(
        "invalid settings refused",
        payroll
            .put_settings(&manager, COMPANY, "proof", &["xaa".into()])
            .await,
        |e| matches!(e, Refusal::InvalidSettings),
    )?;
    must(
        "configure company",
        payroll
            .put_settings(
                &manager,
                COMPANY,
                "proof-jurisdiction",
                &["XAA".into(), "XBB".into(), "XAA".into()],
            )
            .await,
    )?;
    let today: jiff::civil::Date = c
        .query_one("SELECT to_char(current_date,'YYYY-MM-DD')", &[])
        .await?
        .get::<_, String>(0)
        .parse()
        .map_err(|_| Error::Internal)?;
    let start = date(today.year() - 1, 1, 1);
    let end = date(today.year(), 12, 31);
    let future = date(today.year() + 1, 1, 1);
    let future_end = date(today.year() + 1, 12, 31);
    let class = NewClassification {
        code: "py1-class".into(),
        name: "Payroll proof classification".into(),
        effective_from: start,
        effective_to: Some(future_end),
    };
    let classification = must(
        "classification",
        payroll
            .create_classification(&manager, COMPANY, &class)
            .await,
    )?;
    denied(
        "classification overlap",
        payroll
            .create_classification(&manager, COMPANY, &class)
            .await,
        |e| matches!(e, Refusal::OverlappingVersion),
    )?;
    let bad_dates = NewClassification {
        code: "py1-invalid-dates".into(),
        effective_to: Some(start.yesterday().map_err(|_| Error::Invalid)?),
        ..class.clone()
    };
    denied(
        "reversed dates",
        payroll
            .create_classification(&manager, COMPANY, &bad_dates)
            .await,
        |e| matches!(e, Refusal::Invalid(_)),
    )?;
    let new_item = NewItem {
        code: "py1-item".into(),
        name: "Payroll proof item".into(),
        classification_id: classification.id,
        currency: "XAA".into(),
        amount: "12.345678".parse().map_err(|_| Error::Invalid)?,
        effective_from: start,
        effective_to: Some(future_end),
    };
    let item = must(
        "exact item",
        payroll.create_item(&manager, COMPANY, &new_item).await,
    )?;
    check("six places round-trip", item.amount == "12.345678")?;
    denied(
        "item overlap",
        payroll.create_item(&manager, COMPANY, &new_item).await,
        |e| matches!(e, Refusal::OverlappingVersion),
    )?;
    for amount in ["-1", "0.0000001", "100000000000000"] {
        denied(
            "invalid exact amount",
            payroll
                .create_item(
                    &manager,
                    COMPANY,
                    &NewItem {
                        code: format!("py1-amount-{amount}"),
                        amount: amount.parse().map_err(|_| Error::Invalid)?,
                        ..new_item.clone()
                    },
                )
                .await,
            |e| matches!(e, Refusal::InvalidItem),
        )?;
    }
    denied(
        "item outside classification",
        payroll
            .create_item(
                &manager,
                COMPANY,
                &NewItem {
                    code: "py1-outside".into(),
                    effective_to: None,
                    ..new_item.clone()
                },
            )
            .await,
        |e| matches!(e, Refusal::InvalidItem),
    )?;
    denied(
        "unconfigured currency",
        payroll
            .create_item(
                &manager,
                COMPANY,
                &NewItem {
                    code: "py1-currency".into(),
                    currency: "XCC".into(),
                    ..new_item.clone()
                },
            )
            .await,
        |e| matches!(e, Refusal::InvalidItem),
    )?;
    let other_item = must(
        "second currency item",
        payroll
            .create_item(
                &manager,
                COMPANY,
                &NewItem {
                    code: "py1-other-item".into(),
                    currency: "XBB".into(),
                    amount: Decimal::ZERO,
                    ..new_item.clone()
                },
            )
            .await,
    )?;
    let window = NewPeriod {
        code: "py1-period".into(),
        starts_on: start,
        ends_on: end,
        pay_on: end,
    };
    let period = must(
        "pay period",
        payroll.create_period(&manager, COMPANY, &window).await,
    )?;
    denied(
        "overlapping period",
        payroll
            .create_period(
                &manager,
                COMPANY,
                &NewPeriod {
                    code: "py1-overlap".into(),
                    ..window.clone()
                },
            )
            .await,
        |e| matches!(e, Refusal::Invalid(_)),
    )?;
    denied(
        "early payment date",
        payroll
            .create_period(
                &manager,
                COMPANY,
                &NewPeriod {
                    code: "py1-early".into(),
                    pay_on: start,
                    ..window.clone()
                },
            )
            .await,
        |e| matches!(e, Refusal::Invalid(_)),
    )?;
    let next_period = must(
        "adjacent period",
        payroll
            .create_period(
                &manager,
                COMPANY,
                &NewPeriod {
                    code: "py1-next-period".into(),
                    starts_on: future,
                    ends_on: future_end,
                    pay_on: future_end,
                },
            )
            .await,
    )?;
    let mapped_type = leave
        .create_type(
            &manager,
            COMPANY,
            &NewLeaveType::new("py1_mapped", "Mapped leave proof", "day"),
        )
        .await
        .map_err(|e| {
            eprintln!("{e}");
            Error::Internal
        })?;
    let mut inactive = NewLeaveType::new("py1_requested", "Archived leave proof", "day");
    inactive.balance_required = false;
    let requested_type = leave
        .create_type(&manager, COMPANY, &inactive)
        .await
        .map_err(|_| Error::Internal)?;
    let mut request_date = today;
    // The shared Leave proof reserves today and a future interval.
    for _ in 0..7 {
        request_date = request_date.yesterday().map_err(|_| Error::Invalid)?;
    }
    while request_date.weekday().to_monday_one_offset() > 5 {
        request_date = request_date.yesterday().map_err(|_| Error::Invalid)?;
    }
    leave
        .submit_request(
            &manager,
            COMPANY,
            &bilibi_people_leave::NewRequest::new(requested_type.id, request_date, "py1-request"),
        )
        .await
        .map_err(|e| {
            eprintln!("leave request: {e:?}");
            Error::Internal
        })?;
    leave
        .set_type_status(&manager, COMPANY, requested_type.id, "archived")
        .await
        .map_err(|_| Error::Internal)?;
    let category = claims
        .create_category(
            &manager,
            COMPANY,
            &NewCategory {
                code: "py1-group".into(),
                name: "Payroll claim proof".into(),
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    let claim_type = claims
        .create_claim_type(
            &manager,
            COMPANY,
            &NewClaimType {
                category_id: category.id,
                code: "py1-type".into(),
                name: "Inactive requested claim proof".into(),
                receipt_requirement: "never".into(),
                eligibility: None,
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    claims
        .put_currencies(&manager, COMPANY, &["XAA".into()])
        .await
        .map_err(|_| Error::Internal)?;
    claims
        .create_policy(
            &manager,
            COMPANY,
            &bilibi_people_claims::NewPolicy {
                claim_type_id: claim_type.id,
                effective_from: start,
                effective_to: Some(end),
                currency: "XAA".into(),
                per_claim_limit: None,
                monthly_limit: None,
                yearly_limit: None,
                receipt_threshold: None,
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    claims
        .submit_request(
            &manager,
            COMPANY,
            &bilibi_people_claims::NewRequest {
                claim_type_id: claim_type.id,
                incurred_on: today,
                amount: Decimal::ONE,
                currency: "XAA".into(),
                description: Some("Payroll proof".into()),
                receipt_number: None,
                confirm_duplicate: false,
            },
        )
        .await
        .map_err(|e| {
            eprintln!("claim request: {e:?}");
            Error::Internal
        })?;
    claims
        .set_claim_type_active(&manager, COMPANY, claim_type.id, false)
        .await
        .map_err(|_| Error::Internal)?;
    let mapping = NewMapping {
        source_kind: "leave".into(),
        source_key: mapped_type.id.to_string(),
        item_id: item.id,
        effective_from: start,
        effective_to: Some(future_end),
    };
    must(
        "Leave mapping",
        payroll.create_mapping(&manager, COMPANY, &mapping).await,
    )?;
    denied(
        "same currency mapping overlap",
        payroll.create_mapping(&manager, COMPANY, &mapping).await,
        |e| matches!(e, Refusal::OverlappingVersion),
    )?;
    must(
        "same source in second currency",
        payroll
            .create_mapping(
                &manager,
                COMPANY,
                &NewMapping {
                    item_id: other_item.id,
                    ..mapping.clone()
                },
            )
            .await,
    )?;
    denied(
        "unknown source",
        payroll
            .create_mapping(
                &manager,
                COMPANY,
                &NewMapping {
                    source_key: "999999999".into(),
                    ..mapping.clone()
                },
            )
            .await,
        |e| matches!(e, Refusal::InvalidMapping),
    )?;
    let mapped_claim = claims
        .create_claim_type(
            &manager,
            COMPANY,
            &NewClaimType {
                category_id: category.id,
                code: "py1-mapped-claim".into(),
                name: "Mapped claim proof".into(),
                receipt_requirement: "never".into(),
                eligibility: None,
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    must(
        "Claims mapping",
        payroll
            .create_mapping(
                &manager,
                COMPANY,
                &NewMapping {
                    source_kind: "claims".into(),
                    source_key: mapped_claim.id.to_string(),
                    ..mapping.clone()
                },
            )
            .await,
    )?;
    denied(
        "mapping outside item interval",
        payroll
            .create_mapping(
                &manager,
                COMPANY,
                &NewMapping {
                    effective_to: None,
                    ..mapping.clone()
                },
            )
            .await,
        |e| matches!(e, Refusal::InvalidMapping),
    )?;
    let new_rule = NewAllowanceRule {
        code: "py1-rule".into(),
        name: "Payroll allowance proof".into(),
        unit: "hour".into(),
        value: Decimal::ONE,
        currency: "XAA".into(),
        effective_from: start,
        effective_until: None,
    };
    attendance
        .create_allowance_rule(&manager, COMPANY, &new_rule)
        .await
        .map_err(|_| Error::Internal)?;
    let attendance_mapping = NewAttendanceAllowanceMapping {
        attendance_rule_code: new_rule.code.clone(),
        item_id: item.id,
        effective_from: start,
        effective_to: Some(future_end),
    };
    let saved_attendance = must(
        "Attendance mapping",
        payroll
            .create_attendance_allowance_mapping(&manager, COMPANY, &attendance_mapping)
            .await,
    )?;
    denied(
        "Attendance overlap",
        payroll
            .create_attendance_allowance_mapping(&manager, COMPANY, &attendance_mapping)
            .await,
        |e| matches!(e, Refusal::OverlappingVersion),
    )?;
    denied(
        "Attendance currency mismatch",
        payroll
            .create_attendance_allowance_mapping(
                &manager,
                COMPANY,
                &NewAttendanceAllowanceMapping {
                    item_id: other_item.id,
                    ..attendance_mapping.clone()
                },
            )
            .await,
        |e| matches!(e, Refusal::InvalidAttendanceMapping),
    )?;
    let preview = must(
        "Attendance choices",
        payroll
            .attendance_allowances(&manager, COMPANY, start)
            .await,
    )?;
    check(
        "active rule included",
        preview.sources.iter().any(|r| r.code == new_rule.code),
    )?;
    let run = must(
        "freeze setup",
        payroll
            .freeze_setup(&manager, COMPANY, period.id, "XAA")
            .await,
    )?;
    let unmapped = run.snapshot["unmapped_sources"]
        .as_array()
        .ok_or(Error::Internal)?;
    let requested_key = requested_type.id.to_string();
    let claim_key = claim_type.id.to_string();
    check(
        "archived leave with a pending request reported",
        unmapped.iter().any(|v| {
            v["source_kind"] == "leave" && v["source_key"].as_str() == Some(requested_key.as_str())
        }),
    )?;
    check(
        "inactive claim with live request reported",
        unmapped.iter().any(|v| {
            v["source_kind"] == "claims" && v["source_key"].as_str() == Some(claim_key.as_str())
        }),
    )?;
    check(
        "only run currency items frozen",
        run.snapshot["items"]
            .as_array()
            .ok_or(Error::Internal)?
            .iter()
            .all(|i| i["currency"] == "XAA"),
    )?;
    check(
        "exact frozen amount",
        run.snapshot["items"][0]["amount"] == "12.345678",
    )?;
    denied(
        "duplicate run",
        payroll
            .create_run(&manager, COMPANY, period.id, "XAA")
            .await,
        |e| matches!(e, Refusal::RunUnavailable),
    )?;
    denied(
        "unknown run currency",
        payroll
            .freeze_setup(&manager, COMPANY, period.id, "XCC")
            .await,
        |e| matches!(e, Refusal::RunUnavailable),
    )?;
    sql_denied(
        &c,
        "draft snapshot cannot change",
        "UPDATE people_payroll_setup_snapshots SET country='changed' WHERE id=$1",
        run.id,
    )
    .await?;
    sql_denied(
        &c,
        "draft run cannot be deleted",
        "DELETE FROM people_payroll_setup_snapshots WHERE id=$1",
        run.id,
    )
    .await?;
    sql_denied(&c,"cross-company item parent refused","INSERT INTO people_payroll_items (tenant_id,company_id,created_by_actor_id,code,name,classification_id,currency,amount,effective_from) SELECT tenant_id,company_id+999999,created_by_actor_id,'py1-foreign','Foreign proof',id,'XAA',1,effective_from FROM people_payroll_classifications WHERE id=$1",classification.id).await?;
    let locked = must(
        "lock run",
        payroll.lock_run(&manager, COMPANY, run.id).await,
    )?;
    check(
        "sealed creator and locker",
        locked.created_by_actor_id == MANAGER
            && locked.locked_by_actor_id == Some(MANAGER)
            && locked.locked_at.is_some(),
    )?;
    denied(
        "repeat lock refused",
        payroll.lock_run(&manager, COMPANY, run.id).await,
        |e| matches!(e, Refusal::Locked),
    )?;
    for (label,sql,id) in [
        ("immutable classification","UPDATE people_payroll_classifications SET name='changed' WHERE id=$1",classification.id),
        ("immutable item","DELETE FROM people_payroll_items WHERE id=$1",item.id),
        ("immutable period","UPDATE people_payroll_pay_windows SET code='changed' WHERE id=$1",period.id),
        ("immutable mapping","DELETE FROM people_payroll_mappings WHERE item_id=$1",item.id),
        ("immutable Attendance mapping","UPDATE people_payroll_allowance_item_mappings SET attendance_rule_code='changed' WHERE id=$1",saved_attendance.id),
        ("locked run cannot unlock","UPDATE people_payroll_setup_snapshots SET locked_at=NULL,locked_by_actor_id=NULL WHERE id=$1",run.id),
        ("locked run cannot change","UPDATE people_payroll_setup_snapshots SET country='changed' WHERE id=$1",run.id),
        ("locked run cannot delete","DELETE FROM people_payroll_setup_snapshots WHERE id=$1",run.id),
    ] {sql_denied(&c,label,sql,id).await?;}
    attendance
        .create_allowance_rule(
            &manager,
            COMPANY,
            &NewAllowanceRule {
                effective_from: future,
                currency: "XBB".into(),
                ..new_rule.clone()
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    let future_run = must(
        "freeze later changed rule",
        payroll
            .freeze_setup(&manager, COMPANY, next_period.id, "XAA")
            .await,
    )?;
    check(
        "changed rule currency omitted from mappings",
        future_run.snapshot["attendance_mappings"]
            .as_array()
            .ok_or(Error::Internal)?
            .is_empty(),
    )?;
    check(
        "changed rule currency reported",
        future_run.snapshot["unmapped_sources"]
            .as_array()
            .ok_or(Error::Internal)?
            .iter()
            .any(|v| {
                v["source_kind"] == "attendance"
                    && v["source_key"] == "py1-rule"
                    && v["reason"] == "currency mismatch"
            }),
    )?;
    must(
        "settings change after freeze",
        payroll
            .put_settings(&manager, COMPANY, "changed-jurisdiction", &["XBB".into()])
            .await,
    )?;
    let successor_start = future_end.tomorrow().map_err(|_| Error::Invalid)?;
    let successor_class = must(
        "adjacent classification version",
        payroll
            .create_classification(
                &manager,
                COMPANY,
                &NewClassification {
                    effective_from: successor_start,
                    effective_to: None,
                    ..class.clone()
                },
            )
            .await,
    )?;
    must(
        "adjacent pay item version",
        payroll
            .create_item(
                &manager,
                COMPANY,
                &NewItem {
                    classification_id: successor_class.id,
                    currency: "XBB".into(),
                    effective_from: successor_start,
                    effective_to: None,
                    ..new_item.clone()
                },
            )
            .await,
    )?;
    let after = must(
        "reread frozen setup",
        payroll.setup(&manager, COMPANY).await,
    )?;
    let same = after
        .runs
        .iter()
        .find(|r| r.id == run.id)
        .ok_or(Error::Internal)?;
    check(
        "settings and rule changes do not change frozen snapshot",
        same.snapshot == run.snapshot && same.country == "proof-jurisdiction",
    )?;
    grants
        .set_direct(&admin, MANAGER, MANAGE, Some(false))
        .await?;
    denied(
        "revoked grant refuses next write",
        payroll.lock_run(&manager, COMPANY, future_run.id).await,
        |e| matches!(e, Refusal::Unauthorized),
    )?;
    grants
        .set_direct(&admin, MANAGER, MANAGE, Some(true))
        .await?;
    let companies = services.get::<bilibi_company::CompanyService>(bilibi_company::SERVICE)?;
    let archived = match companies
        .create_company(
            &admin,
            &bilibi_company::NewCompany {
                name: "Payroll archived company proof".into(),
                code: Some("py1-archived".into()),
                ..Default::default()
            },
        )
        .await?
    {
        bilibi_company::CreateOutcome::Created(c) => c,
        _ => return Err(Error::Internal),
    };
    companies
        .archive_company(&admin, archived.id, None)
        .await
        .map_err(|e| {
            eprintln!("archive proof company: {e}");
            Error::Internal
        })?;
    denied(
        "archived company refuses writes",
        payroll
            .put_settings(&admin, archived.id, "proof", &["XAA".into()])
            .await,
        |e| matches!(e, Refusal::CompanyArchived),
    )?;
    // One facade race: both connections request the same overlapping version.
    let race = NewClassification {
        code: "py1-race".into(),
        ..class
    };
    let (a, b) = tokio::join!(
        payroll.create_classification(&manager, COMPANY, &race),
        payroll.create_classification(&manager, COMPANY, &race)
    );
    check(
        "concurrent overlapping writers serialize",
        matches!(
            (&a, &b),
            (Ok(_), Err(Refusal::OverlappingVersion)) | (Err(Refusal::OverlappingVersion), Ok(_))
        ),
    )?;
    let audit_count: i64 = c.query_one("SELECT count(*) FROM base_audit_mutations WHERE tenant_id=$1 AND auditable_type LIKE 'people_payroll_%' AND actor_id=$2", &[&TENANT, &MANAGER]).await?.get(0);
    check(
        "Payroll writes captured with the signed-in actor",
        audit_count >= 10,
    )?;
    println!(
        "Payroll foundation proof complete: snapshot {} remains immutable.",
        run.id
    );
    println!(
        "{}",
        json!({"run_id":run.id,"future_run_id":future_run.id,"snapshot":run.snapshot})
    );
    Ok(())
}
