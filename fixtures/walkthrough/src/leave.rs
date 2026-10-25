//! The Leave block of the walkthrough (slice PL1): the leave year and
//! request rules, types, policy versions, grants, opening and adjustment
//! entries and balances, each beside its refusals, and PostgreSQL refusing
//! what the facade never attempts (an edited or deleted ledger entry, a
//! negative entitlement, an unknown entry type, a second live request on the
//! same half-day slot). Carry-forward (PL2) is stood in for by one SQL row
//! so the closed-year refusals show.
use crate::saved;
use bilibi_authz::grants::Grants;
use bilibi_database::Result;
use bilibi_module_registry::Services;
use bilibi_people_leave::{
    Balance, Leave, LeaveError, NewEntry, NewLeaveType, NewPolicy, RequestRules, POLICIES_MANAGE,
    SERVICE,
};
use bilibi_tenancy::Scope;
use jiff::civil::date;
use rust_decimal::Decimal;
use tokio_postgres::types::ToSql;

fn show<T: std::fmt::Debug>(step: &str, result: std::result::Result<T, LeaveError>) -> Option<T> {
    match result {
        Ok(value) => {
            println!("{step}: {value:?}");
            Some(value)
        }
        Err(LeaveError::Invalid(errors)) => {
            let fields = errors
                .iter()
                .map(|error| format!("{} {}", error.field, error.message))
                .collect::<Vec<_>>()
                .join("; ");
            println!("{step}: refused (invalid input: {fields})");
            None
        }
        Err(error) => {
            println!("{step}: refused ({error})");
            None
        }
    }
}

fn print_balances(heading: &str, balances: &[Balance]) {
    println!("{heading}: {} type(s)", balances.len());
    for balance in balances {
        println!(
            "  {} ({}, {}): entitlement {} opening {} adjustment {} carried {} taken {} expired {} = balance {}; pending {} available {}",
            balance.leave_type.name,
            balance.leave_type.unit,
            balance.leave_type.status,
            balance.entitlement,
            balance.opening,
            balance.adjustment,
            balance.carried_forward,
            balance.taken,
            balance.expired,
            balance.balance,
            balance.pending,
            balance.available
        );
    }
}

/// Runs a statement the facade never makes and prints PostgreSQL's answer.
async fn sql(
    client: &tokio_postgres::Client,
    step: &str,
    statement: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Option<i64> {
    match client.query_opt(statement, params).await {
        Ok(row) => {
            let id = row.as_ref().and_then(|row| row.try_get::<_, i64>(0).ok());
            println!(
                "{step}: done{}",
                id.map_or(String::new(), |id| format!(" (row {id})"))
            );
            id
        }
        Err(error) => {
            println!(
                "{step}: refused ({})",
                error
                    .as_db_error()
                    .map(|db| db.message().to_owned())
                    .unwrap_or_else(|| error.to_string())
            );
            None
        }
    }
}

fn entry(
    type_id: i64,
    entry_type: &str,
    quantity: &str,
    on: jiff::civil::Date,
    source: &str,
    key: &str,
) -> NewEntry {
    NewEntry {
        leave_type_id: type_id,
        entry_type: entry_type.into(),
        quantity: quantity.parse().unwrap_or_default(),
        occurred_on: on,
        source: source.into(),
        entry_key: key.into(),
        note: None,
    }
}

fn dec(text: &str) -> Decimal {
    text.parse().unwrap_or_default()
}

/// The Leave walkthrough for Company 001 (`company`), where the manager and
/// Sam Staff are working employees, and Company 002 (`other_company`),
/// where the contractor is.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run(
    services: &Services,
    client: &tokio_postgres::Client,
    admin: &Scope,
    manager: &Scope,
    viewer: &Scope,
    manager_user: i64,
    company: i64,
    other_company: i64,
    manager_employment: i64,
    staff_employment: i64,
    contractor_employment: i64,
) -> Result<()> {
    let leave = services.get::<Leave>(SERVICE)?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    println!("\n== Leave (people/leave): year, rules, types, policies, grants, entries, balances");

    // The leave year and request rules, before and after the grant.
    show(
        &format!("manager reads company {company}'s leave year"),
        leave.rules(manager, company).await,
    );
    show(
        "manager (without people.leave.policies.manage) sets the leave year to April",
        leave.put_rules(manager, company, 4).await,
    );
    grants
        .set_direct(admin, manager_user, POLICIES_MANAGE, Some(true))
        .await?;
    println!("admin grants manager@example.test {POLICIES_MANAGE} directly");
    show(
        "manager sets the leave year to month 13",
        leave.put_rules(manager, company, 13).await,
    );
    show(
        "manager sets the leave year to April",
        leave.put_rules(manager, company, 4).await,
    );
    show(
        "viewer sets the leave year to January",
        leave.put_rules(viewer, company, 1).await,
    );
    show(
        &format!("manager reads company {company}'s request rules"),
        leave.request_rules(manager, company).await,
    );
    show(
        "manager saves request rules: weekdays 6, 1, 2, 3, 4, 5, 6 and 45 days of backdating",
        leave
            .put_request_rules(
                manager,
                company,
                &RequestRules {
                    working_weekdays: vec![6, 1, 2, 3, 4, 5, 6],
                    backdate_days: 45,
                },
            )
            .await,
    );
    show(
        "manager saves request rules with no weekday",
        leave
            .put_request_rules(
                manager,
                company,
                &RequestRules {
                    working_weekdays: vec![],
                    backdate_days: 10,
                },
            )
            .await,
    );
    show(
        "manager saves request rules with weekday 8 and 400 days of backdating",
        leave
            .put_request_rules(
                manager,
                company,
                &RequestRules {
                    working_weekdays: vec![8],
                    backdate_days: 400,
                },
            )
            .await,
    );

    // Types.
    let annual = saved(show(
        "manager adds the type annual (day, paid, balance required)",
        leave
            .create_type(
                manager,
                company,
                &NewLeaveType::new("annual", "Annual leave", "day"),
            )
            .await,
    ))?;
    let unpaid = saved(show(
        "manager adds the type unpaid (day, unpaid, no balance required)",
        leave
            .create_type(
                manager,
                company,
                &NewLeaveType {
                    paid: false,
                    balance_required: false,
                    ..NewLeaveType::new("unpaid", "Unpaid leave", "day")
                },
            )
            .await,
    ))?;
    let overtime = saved(show(
        "manager adds the type overtime_hours (hour)",
        leave
            .create_type(
                manager,
                company,
                &NewLeaveType::new("overtime_hours", "Time off in lieu", "hour"),
            )
            .await,
    ))?;
    show(
        "manager adds a type with the code 'Annual Leave', a blank name and the unit 'week'",
        leave
            .create_type(
                manager,
                company,
                &NewLeaveType::new("Annual Leave", "  ", "week"),
            )
            .await,
    );
    show(
        "manager adds the type annual again",
        leave
            .create_type(
                manager,
                company,
                &NewLeaveType::new("annual", "Annual leave again", "day"),
            )
            .await,
    );
    show(
        "viewer adds a type",
        leave
            .create_type(
                viewer,
                company,
                &NewLeaveType::new("viewer", "Viewer's type", "day"),
            )
            .await,
    );
    show(
        &format!("manager adds a type at company {other_company} with tenant-wide reach"),
        leave
            .create_type(
                manager,
                other_company,
                &NewLeaveType::new("annual", "Annual leave", "day"),
            )
            .await,
    );
    show(
        "manager sets the unpaid type's status to 'deleted'",
        leave
            .set_type_status(manager, company, unpaid.id, "deleted")
            .await,
    );
    show(
        "manager archives the unpaid type",
        leave
            .set_type_status(manager, company, unpaid.id, "archived")
            .await,
    );
    show(
        &format!(
            "manager archives type #{} in company {company}, where it does not exist",
            i64::MAX
        ),
        leave
            .set_type_status(manager, company, i64::MAX, "archived")
            .await,
    );
    if let Some(types) = show(
        &format!("manager lists company {company}'s types"),
        leave.list_types(manager, company).await,
    ) {
        for leave_type in &types {
            println!(
                "  #{} {} {} ({}, paid {}, balance required {}, {})",
                leave_type.id,
                leave_type.code,
                leave_type.name,
                leave_type.unit,
                leave_type.paid,
                leave_type.balance_required,
                leave_type.status
            );
        }
    }

    // Policy versions.
    show(
        "manager adds annual v1 from 2025-04-01: 14.00 days, carry-forward cap 5.00",
        leave
            .add_policy(
                manager,
                company,
                annual.id,
                &NewPolicy {
                    effective_from: date(2025, 4, 1),
                    entitlement: dec("14"),
                    carry_forward_cap: Some(dec("5")),
                },
            )
            .await,
    );
    show(
        "manager adds annual v2 from 2025-04-01 (not after the latest)",
        leave
            .add_policy(
                manager,
                company,
                annual.id,
                &NewPolicy {
                    effective_from: date(2025, 4, 1),
                    entitlement: dec("15"),
                    carry_forward_cap: None,
                },
            )
            .await,
    );
    show(
        "manager adds an annual version with an entitlement of 10000",
        leave
            .add_policy(
                manager,
                company,
                annual.id,
                &NewPolicy {
                    effective_from: date(2026, 1, 1),
                    entitlement: dec("10000"),
                    carry_forward_cap: None,
                },
            )
            .await,
    );
    show(
        "manager adds an annual version with a cap of 1.234",
        leave
            .add_policy(
                manager,
                company,
                annual.id,
                &NewPolicy {
                    effective_from: date(2026, 1, 1),
                    entitlement: dec("15"),
                    carry_forward_cap: Some(dec("1.234")),
                },
            )
            .await,
    );
    show(
        "manager adds a version to the archived unpaid type",
        leave
            .add_policy(
                manager,
                company,
                unpaid.id,
                &NewPolicy {
                    effective_from: date(2026, 1, 1),
                    entitlement: dec("0"),
                    carry_forward_cap: None,
                },
            )
            .await,
    );
    show(
        "manager adds overtime_hours v1 from 2025-04-01: 40.00 hours, no cap",
        leave
            .add_policy(
                manager,
                company,
                overtime.id,
                &NewPolicy {
                    effective_from: date(2025, 4, 1),
                    entitlement: dec("40"),
                    carry_forward_cap: None,
                },
            )
            .await,
    );
    show(
        "viewer adds an annual version",
        leave
            .add_policy(
                viewer,
                company,
                annual.id,
                &NewPolicy {
                    effective_from: date(2027, 1, 1),
                    entitlement: dec("1"),
                    carry_forward_cap: None,
                },
            )
            .await,
    );
    show(
        "manager reads annual's policy on 2026-06-15",
        leave
            .policy_on(manager, company, annual.id, date(2026, 6, 15))
            .await,
    );
    show(
        "manager reads annual's policy on 2025-03-31",
        leave
            .policy_on(manager, company, annual.id, date(2025, 3, 31))
            .await,
    );

    // Grants: once per employee, type and leave year.
    show(
        "manager grants the 2026 entitlements (two working employees, two active types with a policy)",
        leave.grant_entitlements(manager, company, 2026).await,
    );
    show(
        "manager grants the 2026 entitlements again",
        leave.grant_entitlements(manager, company, 2026).await,
    );
    show(
        "manager grants the 1800 entitlements",
        leave.grant_entitlements(manager, company, 1800).await,
    );
    show(
        "viewer grants the 2026 entitlements",
        leave.grant_entitlements(viewer, company, 2026).await,
    );
    show(
        "manager adds annual v2 from 2026-01-01, after the latest but before the 2026 grants",
        leave
            .add_policy(
                manager,
                company,
                annual.id,
                &NewPolicy {
                    effective_from: date(2026, 1, 1),
                    entitlement: dec("16"),
                    carry_forward_cap: Some(dec("8")),
                },
            )
            .await,
    );
    show(
        "manager adds annual v2 from 2026-04-02: 16.00 days, cap 8.00 (closes v1 on 2026-04-01)",
        leave
            .add_policy(
                manager,
                company,
                annual.id,
                &NewPolicy {
                    effective_from: date(2026, 4, 2),
                    entitlement: dec("16"),
                    carry_forward_cap: Some(dec("8")),
                },
            )
            .await,
    );
    if let Some(policies) = show(
        &format!("manager lists company {company}'s policy versions"),
        leave.list_policies(manager, company).await,
    ) {
        for policy in &policies {
            println!(
                "  type #{} v{}: {} to {} entitlement {} cap {}",
                policy.leave_type_id,
                policy.version,
                policy.effective_from,
                policy
                    .effective_to
                    .map_or("open".to_owned(), |date| date.to_string()),
                policy.entitlement,
                policy
                    .carry_forward_cap
                    .map_or("none".to_owned(), |cap| cap.to_string())
            );
        }
    }
    show(
        "manager sets the leave year to July now that the ledger has entries",
        leave.put_rules(manager, company, 7).await,
    );

    // Opening and adjustment entries, idempotent by source and key.
    let opening = entry(
        annual.id,
        "opening",
        "3.5",
        date(2026, 4, 1),
        "migration",
        "opening:annual:E-002",
    );
    show(
        "manager records an opening balance of 3.50 annual days for Sam Staff (source migration, key opening:annual:E-002)",
        leave.record_entry(manager, company, staff_employment, &opening).await,
    );
    show(
        "manager records the same entry again",
        leave
            .record_entry(manager, company, staff_employment, &opening)
            .await,
    );
    show(
        "manager records the same key with 4.00 days",
        leave
            .record_entry(
                manager,
                company,
                staff_employment,
                &NewEntry {
                    quantity: dec("4"),
                    ..opening.clone()
                },
            )
            .await,
    );
    show(
        "manager records the same key for the manager instead",
        leave
            .record_entry(manager, company, manager_employment, &opening)
            .await,
    );
    show(
        "manager records an entry from the source 'policy'",
        leave
            .record_entry(
                manager,
                company,
                staff_employment,
                &entry(annual.id, "opening", "1", date(2026, 4, 1), "policy", "x"),
            )
            .await,
    );
    show(
        "manager records an entry of 0",
        leave
            .record_entry(
                manager,
                company,
                staff_employment,
                &entry(annual.id, "adjustment", "0", date(2026, 4, 1), "hr", "zero"),
            )
            .await,
    );
    show(
        "manager records an entry of 1.234",
        leave
            .record_entry(
                manager,
                company,
                staff_employment,
                &entry(
                    annual.id,
                    "adjustment",
                    "1.234",
                    date(2026, 4, 1),
                    "hr",
                    "fine",
                ),
            )
            .await,
    );
    show(
        "manager records an entry of type 'taken'",
        leave
            .record_entry(
                manager,
                company,
                staff_employment,
                &entry(annual.id, "taken", "-1", date(2026, 4, 1), "hr", "taken"),
            )
            .await,
    );
    show(
        "manager records an adjustment of -1.50 annual days for Sam Staff with a note",
        leave
            .record_entry(
                manager,
                company,
                staff_employment,
                &NewEntry {
                    note: Some("  Taken before go-live  ".into()),
                    ..entry(
                        annual.id,
                        "adjustment",
                        "-1.5",
                        date(2026, 5, 10),
                        "hr",
                        "adjustment:annual:E-002:1",
                    )
                },
            )
            .await,
    );
    show(
        "manager records an opening balance on the archived unpaid type",
        leave
            .record_entry(
                manager,
                company,
                staff_employment,
                &entry(
                    unpaid.id,
                    "opening",
                    "2",
                    date(2026, 4, 1),
                    "migration",
                    "opening:unpaid:E-002",
                ),
            )
            .await,
    );
    show(
        &format!("manager records an opening balance for the contractor (employee #{contractor_employment} of company {other_company}) in company {company}"),
        leave
            .record_entry(manager, company, contractor_employment, &entry(annual.id, "opening", "2", date(2026, 4, 1), "migration", "opening:annual:C-001"))
            .await,
    );
    show(
        "viewer records an opening balance",
        leave
            .record_entry(
                viewer,
                company,
                staff_employment,
                &entry(
                    annual.id,
                    "opening",
                    "2",
                    date(2026, 4, 1),
                    "migration",
                    "viewer",
                ),
            )
            .await,
    );

    // Balances and entries.
    if let Some(balances) = show(
        "manager reads Sam Staff's 2026 balances",
        leave
            .balances(manager, company, staff_employment, 2026)
            .await,
    ) {
        print_balances("Sam Staff, leave year 2026", &balances);
    }
    if let Some(entries) = show(
        "manager reads Sam Staff's 2026 entries",
        leave
            .entries(manager, company, staff_employment, 2026)
            .await,
    ) {
        for entry in &entries {
            println!(
                "  {} {} {} {} (type #{}, {}:{}, policy v{:?}, note {:?})",
                entry.occurred_on,
                entry.entry_type,
                entry.quantity,
                entry.unit,
                entry.leave_type_id,
                entry.source,
                entry.entry_key,
                entry.policy_version,
                entry.note
            );
        }
    }
    if let Some(balances) = show(
        "viewer reads the manager's 2026 balances (a capability-free read)",
        leave
            .balances(viewer, company, manager_employment, 2026)
            .await,
    ) {
        print_balances("the manager, leave year 2026", &balances);
    }
    show(
        &format!("manager reads the contractor's balances in company {company}"),
        leave
            .balances(manager, company, contractor_employment, 2026)
            .await,
    );

    // A pending request reserves its quantity: PL2's `submit_request` stands
    // in as one SQL row here, so `balances` shows pending and available.
    let request = sql(
        client,
        "SQL stands in for a pending request of 1.00 annual day for Sam Staff on 2026-06-01",
        "INSERT INTO people_leave_applications (tenant_id, company_id, employee_id, leave_type_id, leave_year, starts_on, ends_on, day_part, quantity, unit, status, request_key, requested_by_user_id) \
         VALUES (1, $1, $2, $3, 2026, '2026-06-01', '2026-06-01', 'full', 1.00, 'day', 'pending', 'walkthrough:1', $4) RETURNING id",
        &[&company, &staff_employment, &annual.id, &manager_user],
    )
    .await;
    if let Some(request) = request {
        sql(
            client,
            "SQL holds the request's slots for 2026-06-01 (am and pm)",
            "INSERT INTO people_leave_application_dates (tenant_id, company_id, employee_id, request_id, on_date, am, pm, quantity, active) VALUES (1, $1, $2, $3, '2026-06-01', true, true, 1.00, true) RETURNING id",
            &[&company, &staff_employment, &request],
        )
        .await;
        if let Some(balances) = show(
            "manager reads Sam Staff's 2026 balances with the pending request",
            leave
                .balances(manager, company, staff_employment, 2026)
                .await,
        ) {
            print_balances(
                "Sam Staff, leave year 2026, with a pending request",
                &balances,
            );
        }
        if let Some(second) = sql(
            client,
            "SQL stands in for a second request of Sam Staff",
            "INSERT INTO people_leave_applications (tenant_id, company_id, employee_id, leave_type_id, leave_year, starts_on, ends_on, day_part, quantity, unit, status, request_key, requested_by_user_id) \
             VALUES (1, $1, $2, $3, 2026, '2026-06-01', '2026-06-01', 'am', 0.50, 'day', 'pending', 'walkthrough:2', $4) RETURNING id",
            &[&company, &staff_employment, &annual.id, &manager_user],
        )
        .await
        {
            sql(
                client,
                "SQL holds the second request's morning slot for the same date",
                "INSERT INTO people_leave_application_dates (tenant_id, company_id, employee_id, request_id, on_date, am, pm, quantity, active) VALUES (1, $1, $2, $3, '2026-06-01', true, false, 0.50, true) RETURNING id",
                &[&company, &staff_employment, &second],
            )
            .await;
            sql(client, "SQL removes the second stand-in request", "DELETE FROM people_leave_applications WHERE id = $1", &[&second]).await;
        }
        sql(
            client,
            "SQL removes the stand-in request's slots",
            "DELETE FROM people_leave_application_dates WHERE request_id = $1",
            &[&request],
        )
        .await;
        sql(
            client,
            "SQL removes the stand-in request",
            "DELETE FROM people_leave_applications WHERE id = $1",
            &[&request],
        )
        .await;
    }

    // A carried-forward year is closed: PL2's carry-forward stands in as one
    // SQL row closing Sam Staff's annual years up to 2027.
    sql(
        client,
        "SQL stands in for carry-forward closing Sam Staff's annual leave through 2027",
        "INSERT INTO people_leave_ledger_entries (tenant_id, company_id, employee_id, leave_type_id, leave_year, entry_type, quantity, unit, occurred_on, source, entry_key, actor_user_id) \
         VALUES (1, $1, $2, $3, 2028, 'carried_forward', 0.00, 'day', '2028-04-01', 'carry_forward', 'walkthrough:close:2027', $4) RETURNING id",
        &[&company, &staff_employment, &annual.id, &manager_user],
    )
    .await;
    show(
        "manager records an adjustment for Sam Staff's annual leave in 2026 after carry-forward closed it",
        leave
            .record_entry(manager, company, staff_employment, &entry(annual.id, "adjustment", "1", date(2026, 7, 1), "hr", "late"))
            .await,
    );
    show(
        "manager grants the 2027 entitlements (Sam Staff's annual year is closed)",
        leave.grant_entitlements(manager, company, 2027).await,
    );

    // PostgreSQL itself refuses what the facade never attempts.
    sql(
        client,
        "SQL changes a ledger entry's quantity",
        "UPDATE people_leave_ledger_entries SET quantity = 99 WHERE employee_id = $1 RETURNING id",
        &[&staff_employment],
    )
    .await;
    sql(
        client,
        "SQL deletes a ledger entry",
        "DELETE FROM people_leave_ledger_entries WHERE employee_id = $1 RETURNING id",
        &[&staff_employment],
    )
    .await;
    sql(
        client,
        "SQL inserts a policy with a negative entitlement",
        "INSERT INTO people_leave_policies (tenant_id, company_id, leave_type_id, version, effective_from, entitlement) VALUES (1, $1, $2, 99, '2030-01-01', -1) RETURNING id",
        &[&company, &annual.id],
    )
    .await;
    sql(
        client,
        "SQL inserts a ledger entry of type 'bogus'",
        "INSERT INTO people_leave_ledger_entries (tenant_id, company_id, employee_id, leave_type_id, leave_year, entry_type, quantity, unit, occurred_on, source, entry_key) VALUES (1, $1, $2, $3, 2026, 'bogus', 1, 'day', '2026-04-01', 'hr', 'bogus') RETURNING id",
        &[&company, &staff_employment, &annual.id],
    )
    .await;
    sql(
        client,
        &format!("SQL inserts a ledger entry for the contractor (company {other_company}) under company {company}"),
        "INSERT INTO people_leave_ledger_entries (tenant_id, company_id, employee_id, leave_type_id, leave_year, entry_type, quantity, unit, occurred_on, source, entry_key) VALUES (1, $1, $2, $3, 2026, 'opening', 1, 'day', '2026-04-01', 'hr', 'wrong-company') RETURNING id",
        &[&company, &contractor_employment, &annual.id],
    )
    .await;
    sql(
        client,
        "SQL inserts a type with the unit 'week'",
        "INSERT INTO people_leave_catalog_types (tenant_id, company_id, code, name, unit, paid, status) VALUES (1, $1, 'weekly', 'Weekly', 'week', true, 'active') RETURNING id",
        &[&company],
    )
    .await;
    println!(
        "With a People page mounted later, manager@example.test holds {POLICIES_MANAGE} for the Leave policies page; the three Leave settings appear under Administration › System › Settings for that account."
    );
    Ok(())
}

walkthrough_step!("leave", after = [], |ctx| async move {
    let client = ctx.db.get().await?;
    // Leave (slice PL1): its own block, in `leave.rs`.
    crate::leave::run(
        &ctx.services,
        &client,
        &ctx.admin,
        &ctx.manager,
        &ctx.viewer,
        crate::MANAGER,
        crate::COMPANY,
        crate::OTHER_COMPANY,
        ctx.manager_employment.id,
        ctx.staff_employment.id,
        ctx.contractor_employment.id,
    )
    .await?;

    Ok(())
});
