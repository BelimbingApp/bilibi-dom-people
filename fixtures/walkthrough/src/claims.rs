//! The Claims block of the People walkthrough (port plan row PC1): through
//! the Claims facade the manager (Tenant Owner) sets Company 001's claim
//! currencies, builds a catalog of a category and three claim types with the
//! three receipt rules and both eligibilities, adds, overlaps and ends
//! policies, opens an assignment to Sam Staff for the assigned-only type and
//! reads what is open to whom; the viewer, the admin (unlinked), staff (no
//! grant) and the contractor (another company) show the refusals. It ends
//! with PostgreSQL refusing what the facade never attempts.
use bilibi_database::{Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_claims::{
    Claims, ClaimsError, NewAssignment, NewCategory, NewClaimType, NewPolicy, OpenClaimType,
    SERVICE as CLAIMS,
};
use bilibi_tenancy::{Authentication, Scope};
use jiff::civil::date;
use rust_decimal::Decimal;
use std::str::FromStr;

/// What the Workforce block hands over: the accounts it authenticated, the
/// staff account it linked, and the employments it created at `company`
/// (the manager's and Sam Staff's) beside the contractor's `other_company`.
pub(crate) struct Context<'a> {
    pub admin: &'a Scope,
    pub manager: &'a Scope,
    pub viewer: &'a Scope,
    pub contractor: &'a Scope,
    pub staff_user: i64,
    pub tenant: i64,
    pub company: i64,
    pub other_company: i64,
    pub manager_employment: i64,
    pub staff_employment: i64,
}

fn show<T: std::fmt::Debug>(step: &str, result: std::result::Result<T, ClaimsError>) -> Option<T> {
    match result {
        Ok(value) => {
            println!("{step}: {value:?}");
            Some(value)
        }
        Err(ClaimsError::Invalid(errors)) => {
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

fn saved<T>(record: Option<T>) -> Result<T> {
    record.ok_or(Error::Internal)
}

fn amount(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a literal amount")
}

fn print_open(heading: &str, open: Option<Vec<OpenClaimType>>) {
    let Some(open) = open else {
        return;
    };
    println!("{heading}: {} open claim type(s)", open.len());
    for item in open {
        println!(
            "  {} › {} [{}] receipts {} ({}) policy #{} {} {}..{} per-claim {:?} monthly {:?} yearly {:?} threshold {:?}",
            item.category_name,
            item.claim_type.name,
            item.claim_type.code,
            item.claim_type.receipt_requirement,
            item.claim_type.eligibility,
            item.policy.id,
            item.policy.currency,
            item.policy.effective_from,
            item.policy
                .effective_to
                .map_or("open".to_owned(), |date| date.to_string()),
            item.policy.per_claim_limit,
            item.policy.monthly_limit,
            item.policy.yearly_limit,
            item.policy.receipt_threshold,
        );
    }
}

async fn refused(client: &tokio_postgres::Client, step: &str, sql: &str, id: i64) {
    match client.execute(sql, &[&id]).await {
        Ok(rows) => println!("{step}: changed {rows} row(s)"),
        Err(error) => println!(
            "{step}: refused ({})",
            error
                .as_db_error()
                .map(|db| db.message().to_owned())
                .unwrap_or_else(|| error.to_string())
        ),
    }
}

/// Runs the block against the Workforce block's records.
pub(crate) async fn run(
    services: &Services,
    client: &tokio_postgres::Client,
    context: Context<'_>,
) -> Result<()> {
    println!("\n== Claims (people/claims): currencies, catalog, policies, assignments");
    let Context {
        admin,
        manager,
        viewer,
        contractor,
        staff_user,
        tenant,
        company,
        other_company,
        manager_employment,
        staff_employment,
    } = context;
    let claims = services.get::<Claims>(CLAIMS)?;
    let authentication = services.get::<Authentication>("base/tenancy.authentication")?;
    let staff = authentication
        .authenticate(client, tenant, staff_user, "walkthrough".into())
        .await?;

    // Currencies: empty until an operator chooses them.
    show(
        &format!("manager reads company {company}'s claim currencies"),
        claims.currencies(manager, company).await,
    );
    show(
        "viewer sets the claim currencies",
        claims
            .put_currencies(viewer, company, &["USD".into()])
            .await,
    );
    show(
        "manager sets the claim currencies to 'usd', 'MYR' and 'myr' again",
        claims
            .put_currencies(
                manager,
                company,
                &["usd".into(), "MYR".into(), " myr ".into()],
            )
            .await,
    );
    show(
        "manager sets a two-letter currency",
        claims
            .put_currencies(manager, company, &["US".into()])
            .await,
    );
    show(
        "manager sets twenty-one currencies",
        claims
            .put_currencies(manager, company, &vec!["USD".into(); 21])
            .await,
    );

    // Catalog: one category, three claim types.
    let travel = saved(show(
        "manager adds the category Travel",
        claims
            .create_category(
                manager,
                company,
                &NewCategory {
                    code: "travel".into(),
                    name: " Travel ".into(),
                },
            )
            .await,
    ))?;
    show(
        "manager adds the category code 'travel' again",
        claims
            .create_category(
                manager,
                company,
                &NewCategory {
                    code: "travel".into(),
                    name: "Travel again".into(),
                },
            )
            .await,
    );
    show(
        "manager adds a category with a blank code and a 121-character name",
        claims
            .create_category(
                manager,
                company,
                &NewCategory {
                    code: "  ".into(),
                    name: "n".repeat(121),
                },
            )
            .await,
    );
    show(
        "viewer adds a category",
        claims
            .create_category(
                viewer,
                company,
                &NewCategory {
                    code: "x".into(),
                    name: "X".into(),
                },
            )
            .await,
    );
    let mileage = saved(show(
        "manager adds the claim type Mileage (receipts always, all employees)",
        claims
            .create_claim_type(
                manager,
                company,
                &NewClaimType {
                    category_id: travel.id,
                    code: "mileage".into(),
                    name: "Mileage".into(),
                    receipt_requirement: "always".into(),
                    eligibility: None,
                },
            )
            .await,
    ))?;
    let meals = saved(show(
        "manager adds the claim type Meals (receipts above a threshold)",
        claims
            .create_claim_type(
                manager,
                company,
                &NewClaimType {
                    category_id: travel.id,
                    code: "meals".into(),
                    name: "Meals".into(),
                    receipt_requirement: "above_threshold".into(),
                    eligibility: Some("all_employees".into()),
                },
            )
            .await,
    ))?;
    let phone = saved(show(
        "manager adds the claim type Phone (no receipts, assigned only)",
        claims
            .create_claim_type(
                manager,
                company,
                &NewClaimType {
                    category_id: travel.id,
                    code: "phone".into(),
                    name: "Phone".into(),
                    receipt_requirement: "never".into(),
                    eligibility: Some("assigned_only".into()),
                },
            )
            .await,
    ))?;
    show(
        "manager adds a claim type in a category that does not exist",
        claims
            .create_claim_type(
                manager,
                company,
                &NewClaimType {
                    category_id: i64::MAX,
                    code: "taxi".into(),
                    name: "Taxi".into(),
                    receipt_requirement: "always".into(),
                    eligibility: None,
                },
            )
            .await,
    );
    show(
        "manager adds a claim type with the receipt rule 'maybe' and the eligibility 'friends'",
        claims
            .create_claim_type(
                manager,
                company,
                &NewClaimType {
                    category_id: travel.id,
                    code: "taxi".into(),
                    name: "Taxi".into(),
                    receipt_requirement: "maybe".into(),
                    eligibility: Some("friends".into()),
                },
            )
            .await,
    );
    show(
        &format!("manager lists company {company}'s categories"),
        claims.categories(manager, company).await,
    );
    show(
        &format!("viewer lists company {company}'s claim types"),
        claims.claim_types(viewer, company).await,
    );
    show(
        &format!("manager lists company {other_company}'s claim types"),
        claims.claim_types(manager, other_company).await,
    );
    show(
        "manager lists a company that does not exist",
        claims.claim_types(manager, i64::MAX).await,
    );

    // Policies: a period, a currency from the list, limits and the threshold.
    let policy = |claim_type_id, from, to, currency: &str| NewPolicy {
        claim_type_id,
        effective_from: from,
        effective_to: to,
        currency: currency.into(),
        per_claim_limit: None,
        monthly_limit: None,
        yearly_limit: None,
        receipt_threshold: None,
    };
    let mileage_2026 = saved(show(
        "manager adds a Mileage policy for 2026 in USD with a 100.00 per-claim limit",
        claims
            .create_policy(
                manager,
                company,
                &NewPolicy {
                    per_claim_limit: Some(amount("100.00")),
                    monthly_limit: Some(amount("1000")),
                    ..policy(mileage.id, date(2026, 1, 1), None, "usd")
                },
            )
            .await,
    ))?;
    show(
        "manager adds a Mileage policy from June 2026 (overlapping)",
        claims
            .create_policy(
                manager,
                company,
                &policy(mileage.id, date(2026, 6, 1), None, "USD"),
            )
            .await,
    );
    show(
        "manager adds a Meals policy in EUR (not a company currency)",
        claims
            .create_policy(
                manager,
                company,
                &policy(meals.id, date(2026, 1, 1), None, "EUR"),
            )
            .await,
    );
    show(
        "manager adds a Meals policy without a receipt threshold",
        claims
            .create_policy(
                manager,
                company,
                &policy(meals.id, date(2026, 1, 1), None, "MYR"),
            )
            .await,
    );
    show(
        "manager adds a Meals policy ending before it starts, with a three-decimal limit and a negative yearly limit",
        claims
            .create_policy(
                manager,
                company,
                &NewPolicy {
                    per_claim_limit: Some(amount("10.005")),
                    yearly_limit: Some(amount("-1")),
                    receipt_threshold: Some(amount("25")),
                    ..policy(meals.id, date(2026, 3, 1), Some(date(2026, 2, 1)), "MYR")
                },
            )
            .await,
    );
    show(
        "manager adds a policy for a claim type that does not exist",
        claims
            .create_policy(
                manager,
                company,
                &policy(i64::MAX, date(2026, 1, 1), None, "MYR"),
            )
            .await,
    );
    let meals_policy = saved(show(
        "manager adds an open-ended Meals policy in MYR with a 25.00 receipt threshold",
        claims
            .create_policy(
                manager,
                company,
                &NewPolicy {
                    receipt_threshold: Some(amount("25.00")),
                    yearly_limit: Some(amount("999999999999.99")),
                    ..policy(meals.id, date(2026, 1, 1), None, "MYR")
                },
            )
            .await,
    ))?;
    show(
        "manager adds a Mileage policy with a 1,000,000,000,000.00 limit",
        claims
            .create_policy(
                manager,
                company,
                &NewPolicy {
                    per_claim_limit: Some(amount("1000000000000")),
                    ..policy(mileage.id, date(2030, 1, 1), None, "USD")
                },
            )
            .await,
    );
    saved(show(
        "manager adds an open-ended Phone policy in MYR with a threshold (dropped: no receipts)",
        claims
            .create_policy(
                manager,
                company,
                &NewPolicy {
                    receipt_threshold: Some(amount("5")),
                    ..policy(phone.id, date(2026, 1, 1), None, "MYR")
                },
            )
            .await,
    ))?;
    show(
        "manager ends the 2026 Mileage policy on 2026-12-31",
        claims
            .end_policy(manager, company, mileage_2026.id, date(2026, 12, 31))
            .await,
    );
    show(
        "manager ends it again",
        claims
            .end_policy(manager, company, mileage_2026.id, date(2026, 11, 30))
            .await,
    );
    show(
        "manager ends the Meals policy before it started",
        claims
            .end_policy(manager, company, meals_policy.id, date(2025, 12, 31))
            .await,
    );
    show(
        "manager ends a policy that does not exist",
        claims
            .end_policy(manager, company, i64::MAX, date(2026, 12, 31))
            .await,
    );
    show(
        "manager adds a Mileage policy from 2027 (no overlap now)",
        claims
            .create_policy(
                manager,
                company,
                &policy(mileage.id, date(2027, 1, 1), None, "USD"),
            )
            .await,
    );
    show(
        &format!("manager lists company {company}'s policies"),
        claims.policies(manager, company).await,
    );

    // What is open to whom.
    print_open(
        "open claim types today, every eligibility",
        show(
            "manager reads the claim types open today",
            claims.open_claim_types(manager, company, None, None).await,
        ),
    );
    print_open(
        "open to the manager's own employee",
        show(
            "manager reads the claim types open to themselves",
            claims.self_open_claim_types(manager, company).await,
        ),
    );
    show(
        "manager resolves their own employee",
        claims.self_service_employee(manager, company).await,
    );
    show(
        "admin (grant-all, no affiliation) reads the claim types open to themselves",
        claims.self_open_claim_types(admin, company).await,
    );
    show(
        "staff (linked, without the submit grant) reads the claim types open to themselves",
        claims.self_open_claim_types(&staff, company).await,
    );
    show(
        &format!("contractor reads the claim types open to themselves at company {other_company}"),
        claims
            .self_open_claim_types(contractor, other_company)
            .await,
    );
    show(
        &format!("manager reads the claim types open to themselves at company {other_company}"),
        claims.self_open_claim_types(manager, other_company).await,
    );

    // Assignments open the assigned-only type to Sam Staff.
    let field = saved(show(
        "manager adds the assignment Field staff from 2026-01-01",
        claims
            .create_assignment(
                manager,
                company,
                &NewAssignment {
                    code: "field".into(),
                    name: "Field staff".into(),
                    effective_from: date(2026, 1, 1),
                    effective_to: None,
                },
            )
            .await,
    ))?;
    show(
        "manager adds an assignment ending before it starts",
        claims
            .create_assignment(
                manager,
                company,
                &NewAssignment {
                    code: "early".into(),
                    name: "Early".into(),
                    effective_from: date(2026, 2, 1),
                    effective_to: Some(date(2026, 1, 1)),
                },
            )
            .await,
    );
    show(
        &format!("manager lists company {company}'s assignable employees"),
        claims.assignable_employees(manager, company).await,
    );
    show(
        "manager sets Field staff's members to Phone and Sam Staff",
        claims
            .set_assignment_members(manager, company, field.id, &[phone.id], &[staff_employment])
            .await,
    );
    show(
        "manager sets the same members again (no change)",
        claims
            .set_assignment_members(manager, company, field.id, &[phone.id], &[staff_employment])
            .await,
    );
    show(
        "manager sets a member employee that does not exist",
        claims
            .set_assignment_members(manager, company, field.id, &[phone.id], &[i64::MAX])
            .await,
    );
    show(
        "manager sets a member claim type that does not exist",
        claims
            .set_assignment_members(manager, company, field.id, &[i64::MAX], &[staff_employment])
            .await,
    );
    show(
        "manager sets members with an id of zero",
        claims
            .set_assignment_members(manager, company, field.id, &[0], &[])
            .await,
    );
    show(
        "viewer sets Field staff's members",
        claims
            .set_assignment_members(viewer, company, field.id, &[], &[])
            .await,
    );
    show(
        &format!("manager lists company {company}'s assignments"),
        claims.assignments(manager, company).await,
    );
    print_open(
        "open to Sam Staff today",
        show(
            "manager reads the claim types open to Sam Staff today",
            claims
                .open_claim_types(manager, company, None, Some(staff_employment))
                .await,
        ),
    );
    print_open(
        "open to the manager's employee today",
        show(
            "manager reads the claim types open to their own employee today",
            claims
                .open_claim_types(manager, company, None, Some(manager_employment))
                .await,
        ),
    );
    show(
        "manager ends Field staff on 2026-12-31",
        claims
            .end_assignment(manager, company, field.id, date(2026, 12, 31))
            .await,
    );
    show(
        "manager ends it again",
        claims
            .end_assignment(manager, company, field.id, date(2027, 1, 31))
            .await,
    );
    print_open(
        "open to Sam Staff on 2027-01-15 (assignment ended, 2026 Mileage policy ended)",
        show(
            "manager reads the claim types open to Sam Staff on 2027-01-15",
            claims
                .open_claim_types(
                    manager,
                    company,
                    Some(date(2027, 1, 15)),
                    Some(staff_employment),
                )
                .await,
        ),
    );

    // Deactivation closes the catalog without deleting anything.
    show(
        "manager deactivates the category Travel",
        claims
            .set_category_active(manager, company, travel.id, false)
            .await,
    );
    print_open(
        "open while Travel is inactive",
        show(
            "manager reads the claim types open today",
            claims.open_claim_types(manager, company, None, None).await,
        ),
    );
    show(
        "manager reactivates the category Travel",
        claims
            .set_category_active(manager, company, travel.id, true)
            .await,
    );
    show(
        "manager deactivates the claim type Phone",
        claims
            .set_claim_type_active(manager, company, phone.id, false)
            .await,
    );
    show(
        "manager deactivates a claim type that does not exist",
        claims
            .set_claim_type_active(manager, company, i64::MAX, false)
            .await,
    );
    show(
        "manager reactivates the claim type Phone",
        claims
            .set_claim_type_active(manager, company, phone.id, true)
            .await,
    );

    // PostgreSQL itself refuses what the facade never attempts.
    for (step, sql) in [
        (
            "SQL lowercases a policy's currency",
            "UPDATE people_claim_policy_versions SET currency = 'usd' WHERE id = $1",
        ),
        (
            "SQL sets a negative per-claim limit",
            "UPDATE people_claim_policy_versions SET per_claim_limit = -1 WHERE id = $1",
        ),
        (
            "SQL ends a policy before it starts",
            "UPDATE people_claim_policy_versions SET effective_to = effective_from - 1 WHERE id = $1",
        ),
        (
            "SQL gives a claim type the receipt rule 'maybe'",
            "UPDATE people_claim_catalog_types SET receipt_requirement = 'maybe' WHERE company_id = (SELECT company_id FROM people_claim_policy_versions WHERE id = $1)",
        ),
        (
            "SQL moves a claim type to Company 002",
            "UPDATE people_claim_catalog_types SET company_id = 2 WHERE id = (SELECT claim_type_id FROM people_claim_policy_versions WHERE id = $1)",
        ),
        (
            "SQL inserts a second category coded 'travel'",
            "INSERT INTO people_claim_catalog_groups (tenant_id, company_id, code, name) SELECT tenant_id, company_id, 'travel', 'Travel twice' FROM people_claim_policy_versions WHERE id = $1",
        ),
        (
            "SQL assigns the contractor (Company 002) to a Company 001 assignment",
            "INSERT INTO people_claim_assignment_employees (tenant_id, company_id, assignment_id, employee_id) SELECT a.tenant_id, a.company_id, a.id, e.id FROM people_claim_employee_enrolments a, people_workforce_employments e WHERE e.company_id = 2 AND a.company_id = (SELECT company_id FROM people_claim_policy_versions WHERE id = $1)",
        ),
        (
            "SQL submits a claim with the status 'paid'",
            "INSERT INTO people_claim_submissions (tenant_id, company_id, employee_id, claim_type_id, claim_policy_id, incurred_on, amount, currency, status, submitted_by_actor_id, submitted_at) SELECT p.tenant_id, p.company_id, e.id, p.claim_type_id, p.id, p.effective_from, 10, p.currency, 'paid', 1, now() FROM people_claim_policy_versions p JOIN people_workforce_employments e ON e.company_id = p.company_id WHERE p.id = $1 LIMIT 1",
        ),
        (
            "SQL approves a claim it inserts without a decider",
            "INSERT INTO people_claim_submissions (tenant_id, company_id, employee_id, claim_type_id, claim_policy_id, incurred_on, amount, currency, status, approved_amount, submitted_by_actor_id, submitted_at) SELECT p.tenant_id, p.company_id, e.id, p.claim_type_id, p.id, p.effective_from, 10, p.currency, 'approved', 10, 1, now() FROM people_claim_policy_versions p JOIN people_workforce_employments e ON e.company_id = p.company_id WHERE p.id = $1 LIMIT 1",
        ),
    ] {
        refused(client, step, sql, mileage_2026.id).await;
    }
    Ok(())
}

/// After PW1 creates a pending employment, prove that it is assignable even
/// though it is absent from Workforce's working-staff directory.
pub(crate) async fn all_status_members(
    services: &Services,
    manager: &Scope,
    company: i64,
) -> Result<()> {
    let claims = services.get::<Claims>(CLAIMS)?;
    let employees = saved(show(
        "Claims picker includes pending employments",
        claims.assignable_employees(manager, company).await,
    ))?;
    let pending = employees
        .iter()
        .find(|employee| employee.employee_number == "E-003")
        .ok_or(Error::Internal)?;
    let assignment = saved(show(
        "manager creates an assignment for pending staff",
        claims
            .create_assignment(
                manager,
                company,
                &NewAssignment {
                    code: "pending-staff".into(),
                    name: "Pending staff".into(),
                    effective_from: date(2027, 1, 1),
                    effective_to: None,
                },
            )
            .await,
    ))?;
    let types = claims
        .claim_types(manager, company)
        .await
        .map_err(|_| Error::Internal)?;
    let phone = types
        .iter()
        .find(|claim_type| claim_type.code == "phone")
        .ok_or(Error::Internal)?;
    saved(show(
        "manager assigns a pending employee to Phone",
        claims
            .set_assignment_members(manager, company, assignment.id, &[phone.id], &[pending.id])
            .await,
    ))?;
    Ok(())
}

walkthrough_step!("claims", after = ["skills_catalog"], |ctx| async move {
    let client = ctx.db.get().await?;
    // Claims (slice PC1): currencies, catalog, policies and assignments, on
    // the employments created above.
    crate::claims::run(
        &ctx.services,
        &client,
        crate::claims::Context {
            admin: &ctx.admin,
            manager: &ctx.manager,
            viewer: &ctx.viewer,
            contractor: &ctx.contractor,
            staff_user: crate::STAFF,
            tenant: crate::TENANT,
            company: crate::COMPANY,
            other_company: crate::OTHER_COMPANY,
            manager_employment: ctx.manager_employment.id,
            staff_employment: ctx.staff_employment.id,
        },
    )
    .await?;

    Ok(())
});

walkthrough_step!(
    "claims_all_status",
    after = ["leave_after_unlink"],
    |ctx| async move {
        // Claims assignment pickers include the pending employment PW1 created.
        crate::claims::all_status_members(&ctx.services, &ctx.manager, crate::COMPANY).await?;

        Ok(())
    }
);
