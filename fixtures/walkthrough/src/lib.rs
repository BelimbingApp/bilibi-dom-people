//! `DATABASE_URL=… cargo run -p bilibi-people-walkthrough`: the People
//! walkthrough fixture for slice proofs. Against a freshly created disposable
//! database it migrates the composed app, installs the Platform walkthrough
//! fixture (its tenants, accounts, Persons, affiliations and companies),
//! gives `manager@example.test` the Tenant Owner role that carries the
//! Workforce employment capabilities, and through the Workforce facade
//! creates the employments of the fixture's affiliated accounts: the manager
//! at Company 001, a new human Person affiliated with Company 001 and linked
//! to `staff@example.test` who reports to the manager, and the contractor at
//! Company 002 (reached with the manager's tenant-wide reach). It shows the
//! refusals beside the writes (a viewer without the grant, an affiliation of
//! another company, an affiliation already employed, an agent Person, a taken
//! number, an unknown status, an end before the start), reads the company,
//! its working employees, one employee, employees by id and the manager's
//! direct reports, narrows and restores the company's working statuses, asks
//! the `Authorization` helpers what each account may reach, and ends with
//! PostgreSQL refusing what the facade never attempts.
//!
//! A `bilibi-web run` against the same database then shows no People entry
//! in the sidebar for any account: the People root has groups but no leaf
//! yet, and Base Menu hides an empty container.
//!
//! Never a product dependency, and never run against a real install: the
//! Platform fixture refuses a database that already holds other accounts.
use bilibi_affiliation::{
    AffiliationInput, AffiliationService, EffectiveDates, OrganisationId, PersonId,
};
use bilibi_authz::{
    grants::Grants,
    roles::{self, Query, Sort},
};
use bilibi_database::{pool, Error, Result};
use bilibi_module_registry::{verify_composition, Services};
use bilibi_people_workforce::{
    authorization, Employee, Employment, NewEmployment, Workforce, WorkforceError,
    EMPLOYMENT_MANAGE, EMPLOYMENT_VIEW, SERVICE, SETTINGS_MANAGE,
};
use bilibi_person::{PersonInput, PersonKind, PersonService};
use bilibi_tenancy::{Authentication, Scope};
use bilibi_user::UserService;
use jiff::civil::date;
use std::{fmt::Display, sync::Arc};

/// Tenant 1 (the platform operator), its primary company (001), its sibling
/// Company 002, and tenant 2's private company.
const TENANT: i64 = 1;
const COMPANY: i64 = 1;
const OTHER_COMPANY: i64 = 2;
const OTHER_TENANT_COMPANY: i64 = 100;
/// The fixture's accounts: the manager (affiliated with Company 001), the
/// viewer and staff (unlinked), the grant-all admin and the contractor
/// (affiliated with Company 002).
const MANAGER: i64 = 1;
const VIEWER: i64 = 2;
const STAFF: i64 = 3;
const ADMIN: i64 = 4;
const CONTRACTOR: i64 = 7;
/// The fixture's Persons and affiliations (`fixtures/walkthrough/src/operator.sql`).
const MANAGER_PERSON: i64 = 1;
const MANAGER_AFFILIATION: i64 = 1;
const CONTRACTOR_PERSON: i64 = 2;
const CONTRACTOR_AFFILIATION: i64 = 2;
const AGENT_PERSON: i64 = 3;

fn show<T: std::fmt::Debug>(
    step: &str,
    result: std::result::Result<T, WorkforceError>,
) -> Option<T> {
    match result {
        Ok(value) => {
            println!("{step}: {value:?}");
            Some(value)
        }
        Err(WorkforceError::Invalid(errors)) => {
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

/// A record saved by a step the walkthrough cannot go on without.
fn saved<T>(record: Option<T>) -> Result<T> {
    record.ok_or(Error::Internal)
}

fn print_employees(heading: &str, employees: &[Employee]) {
    println!("{heading}: {} employee(s)", employees.len());
    for employee in employees {
        println!(
            "  #{} {} {} ({}; supervisor {})",
            employee.reference.stable_id,
            employee.employee_number,
            employee.display_name,
            employee.email.as_deref().unwrap_or("no work email shown"),
            employee
                .supervisor_reference
                .as_ref()
                .map_or("none".to_owned(), |supervisor| format!(
                    "#{}",
                    supervisor.stable_id
                ))
        );
    }
}

fn print_employment(step: &str, employment: &Employment) {
    println!(
        "{step}: employment #{} in company {}: person {} affiliation {} number {} type {} status {} from {} to {} supervisor {:?} version {}",
        employment.id,
        employment.platform_company_id,
        employment.person_id,
        employment.affiliation_id,
        employment.employee_number,
        employment.employment_type,
        employment.status,
        employment
            .employment_start
            .map_or("open".to_owned(), |date| date.to_string()),
        employment
            .employment_end
            .map_or("open".to_owned(), |date| date.to_string()),
        employment.supervisor_employment_id,
        employment.version
    );
}

async fn tenant_owner_role(
    client: &impl tokio_postgres::GenericClient,
    actor: &Scope,
) -> Result<i64> {
    let page = roles::list(
        client,
        actor,
        &[],
        &Query {
            search: "tenant_owner".into(),
            sort: Sort::Code,
            descending: false,
            page: 1,
            page_size: 50,
        },
    )
    .await?;
    page.entries
        .iter()
        .find(|role| role.code.as_deref() == Some("tenant_owner"))
        .map(|role| role.id)
        .ok_or(Error::NotFound)
}

/// Runs a statement the facade never makes and prints PostgreSQL's answer.
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

fn describe<T: Display, E: Display>(step: &str, result: std::result::Result<T, E>) {
    match result {
        Ok(value) => println!("{step}: {value}"),
        Err(error) => println!("{step}: refused ({error})"),
    }
}

/// Runs the walkthrough against `url`, a freshly created disposable database.
pub async fn run(url: &str) -> Result<()> {
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = pool(url)?;
    let modules = bilibi_module_registry::discover()?;
    verify_composition(
        &modules,
        bilibi_domains::MODULES,
        bilibi_extensions::MODULES,
    )?;
    let migrated = bilibi_database::migrate(&db, &modules).await?;
    println!(
        "mounted Domain modules: {:?}; {} migration(s) applied",
        bilibi_domains::MODULES,
        migrated.applied.len()
    );
    bilibi_walkthrough_fixture::install(&db).await?;
    let mut services = Services::default();
    services.provide("bootstrap.database", Arc::new(db.clone()))?;
    // What this program is made of, for Base Software's status and reviews.
    services.provide("bootstrap.composition", Arc::new(bilibi_web::composition()))?;
    services.install(&modules)?;
    // Match the web host's documented development mode for this disposable
    // fixture; other environments retain the Platform's step-up enforcement.
    if std::env::var("BILIBI_ENV").as_deref() == Ok("development") {
        services
            .get::<bilibi_base::step_up::StepUp>(bilibi_base::step_up::SERVICE)?
            .set_enforced(false);
    }
    // Reconciliation creates each tenant's contributed system roles, among
    // them Tenant Owner with the two Workforce employment capabilities.
    let grants = services.get::<Grants>("base/authz.grants")?;
    grants.reconcile().await?;
    let authentication = services.get::<Authentication>("base/tenancy.authentication")?;
    let workforce = services.get::<Workforce>(SERVICE)?;
    let persons = services.get::<PersonService>(bilibi_person::SERVICE)?;
    let affiliations = services.get::<AffiliationService>(bilibi_affiliation::SERVICE)?;
    let users = services.get::<UserService>("core/user.accounts")?;

    let client = db.get().await?;
    let admin = authentication
        .authenticate(&**client, TENANT, ADMIN, "walkthrough".into())
        .await?;
    let role = tenant_owner_role(&**client, &admin).await?;
    grants.assign_role(&admin, MANAGER, role, true).await?;
    println!("manager@example.test holds Tenant Owner (role {role}) in tenant {TENANT}");
    let manager = authentication
        .authenticate(&**client, TENANT, MANAGER, "walkthrough".into())
        .await?;
    let viewer = authentication
        .authenticate(&**client, TENANT, VIEWER, "walkthrough".into())
        .await?;
    let contractor = authentication
        .authenticate(&**client, TENANT, CONTRACTOR, "walkthrough".into())
        .await?;

    // The company before any employment: its native identity, its working
    // statuses and an empty workforce.
    show(
        &format!("manager reads company {COMPANY}"),
        workforce.company(&manager, COMPANY).await,
    );
    show(
        &format!("manager reads company {COMPANY}'s working statuses"),
        workforce.working_statuses(&manager, COMPANY).await,
    );
    if let Some(read) = show(
        &format!("manager lists company {COMPANY}'s employees"),
        workforce.employees(&manager, COMPANY).await,
    ) {
        print_employees(
            "before any employment",
            read.value.as_deref().unwrap_or(&[]),
        );
    }

    // The manager's own employment, then the refusals a form shows.
    let manager_employment = saved(show(
        "manager employs their own affiliation at Company 001",
        workforce
            .create_employment(
                &manager,
                COMPANY,
                &NewEmployment {
                    employment_start: Some(date(2024, 1, 1)),
                    ..NewEmployment::new(
                        MANAGER_PERSON,
                        MANAGER_AFFILIATION,
                        "E-001",
                        "full_time",
                        "active",
                    )
                },
            )
            .await,
    ))?;
    print_employment("saved", &manager_employment);
    show(
        "manager employs the same affiliation again",
        workforce
            .create_employment(
                &manager,
                COMPANY,
                &NewEmployment::new(
                    MANAGER_PERSON,
                    MANAGER_AFFILIATION,
                    "E-001-AGAIN",
                    "full_time",
                    "active",
                ),
            )
            .await,
    );
    show(
        "viewer employs the contractor's affiliation at Company 002",
        workforce
            .create_employment(
                &viewer,
                OTHER_COMPANY,
                &NewEmployment::new(
                    CONTRACTOR_PERSON,
                    CONTRACTOR_AFFILIATION,
                    "C-001",
                    "contractor",
                    "active",
                ),
            )
            .await,
    );
    show(
        "manager employs the contractor's Company 002 affiliation at Company 001",
        workforce
            .create_employment(
                &manager,
                COMPANY,
                &NewEmployment::new(
                    CONTRACTOR_PERSON,
                    CONTRACTOR_AFFILIATION,
                    "E-009",
                    "contractor",
                    "active",
                ),
            )
            .await,
    );
    show(
        "manager employs with a blank number, the status 'fired' and an end before the start",
        workforce
            .create_employment(
                &manager,
                COMPANY,
                &NewEmployment {
                    employment_start: Some(date(2026, 3, 1)),
                    employment_end: Some(date(2026, 2, 1)),
                    ..NewEmployment::new(
                        CONTRACTOR_PERSON,
                        CONTRACTOR_AFFILIATION,
                        "  ",
                        "full_time",
                        "fired",
                    )
                },
            )
            .await,
    );

    // Staff: a new human Person affiliated with Company 001, linked to the
    // fixture's staff account, employed under the manager.
    let staff_person = persons
        .create(
            &manager,
            &PersonInput {
                name: "Sam Staff".into(),
                kind: PersonKind::Human,
                email: None,
                phone: None,
            },
        )
        .await
        .map_err(|error| {
            println!("manager creates the Person Sam Staff: refused ({error})");
            Error::Internal
        })?;
    println!(
        "manager creates the Person Sam Staff: person {}",
        staff_person.id
    );
    let staff_affiliation = affiliations
        .create(
            &manager,
            &AffiliationInput {
                person_id: staff_person.id,
                organisation_id: OrganisationId::new(COMPANY).map_err(|_| Error::Invalid)?,
                title: Some("Operations Staff".into()),
                work_email: Some("staff@example.test".into()),
                work_phone: None,
                dates: EffectiveDates::new(None, None).map_err(|_| Error::Invalid)?,
            },
        )
        .await?;
    println!(
        "manager affiliates Sam Staff with Company 001: affiliation {}",
        staff_affiliation.id
    );
    match users
        .link_affiliation(&manager, STAFF, staff_affiliation.id)
        .await
    {
        Ok(links) => println!(
            "manager links staff@example.test to that affiliation: person {:?} affiliation {:?} organisation {:?}",
            links.person, links.affiliation, links.organisation
        ),
        Err(error) => println!("manager links staff@example.test to that affiliation: refused ({error})"),
    }
    show(
        "manager employs Sam Staff with the manager's number",
        workforce
            .create_employment(
                &manager,
                COMPANY,
                &NewEmployment::new(
                    staff_person.id.get(),
                    staff_affiliation.id.get(),
                    "E-001",
                    "full_time",
                    "probation",
                ),
            )
            .await,
    );
    show(
        "manager employs Sam Staff under an employment that does not exist",
        workforce
            .create_employment(
                &manager,
                COMPANY,
                &NewEmployment {
                    supervisor_employment_id: Some(i64::MAX),
                    ..NewEmployment::new(
                        staff_person.id.get(),
                        staff_affiliation.id.get(),
                        "E-002",
                        "full_time",
                        "probation",
                    )
                },
            )
            .await,
    );
    let staff_employment = saved(show(
        "manager employs Sam Staff on probation under themselves",
        workforce
            .create_employment(
                &manager,
                COMPANY,
                &NewEmployment {
                    employment_start: Some(date(2026, 10, 1)),
                    supervisor_employment_id: Some(manager_employment.id),
                    ..NewEmployment::new(
                        staff_person.id.get(),
                        staff_affiliation.id.get(),
                        "E-002",
                        "full_time",
                        "probation",
                    )
                },
            )
            .await,
    ))?;
    print_employment("saved", &staff_employment);

    // An agent Person is never an employee, even with an affiliation.
    let agent_affiliation = affiliations
        .create(
            &manager,
            &AffiliationInput {
                person_id: PersonId::new(AGENT_PERSON).map_err(|_| Error::Invalid)?,
                organisation_id: OrganisationId::new(COMPANY).map_err(|_| Error::Invalid)?,
                title: Some("Assistant".into()),
                work_email: None,
                work_phone: None,
                dates: EffectiveDates::new(None, None).map_err(|_| Error::Invalid)?,
            },
        )
        .await?;
    show(
        "manager employs the AI Agent Person",
        workforce
            .create_employment(
                &manager,
                COMPANY,
                &NewEmployment::new(
                    AGENT_PERSON,
                    agent_affiliation.id.get(),
                    "A-001",
                    "agent",
                    "active",
                ),
            )
            .await,
    );

    // The contractor at Company 002, reached through the manager's
    // tenant-wide reach; the contractor holds no People grant.
    show(
        "contractor employs their own affiliation at Company 002",
        workforce
            .create_employment(
                &contractor,
                OTHER_COMPANY,
                &NewEmployment::new(
                    CONTRACTOR_PERSON,
                    CONTRACTOR_AFFILIATION,
                    "C-001",
                    "contractor",
                    "active",
                ),
            )
            .await,
    );
    let contractor_employment = saved(show(
        "manager, with tenant-wide reach, employs the contractor at Company 002",
        workforce
            .create_employment(
                &manager,
                OTHER_COMPANY,
                &NewEmployment {
                    employment_start: Some(date(2025, 6, 1)),
                    employment_end: Some(date(2027, 5, 31)),
                    ..NewEmployment::new(
                        CONTRACTOR_PERSON,
                        CONTRACTOR_AFFILIATION,
                        "C-001",
                        "contractor",
                        "active",
                    )
                },
            )
            .await,
    ))?;
    print_employment("saved", &contractor_employment);

    // The read seam every People module builds on.
    if let Some(read) = show(
        &format!("manager lists company {COMPANY}'s employees"),
        workforce.employees(&manager, COMPANY).await,
    ) {
        print_employees(
            "working employees of Company 001",
            read.value.as_deref().unwrap_or(&[]),
        );
    }
    if let Some(read) = show(
        &format!("viewer lists company {COMPANY}'s employees"),
        workforce.employees(&viewer, COMPANY).await,
    ) {
        print_employees(
            "working employees of Company 001, as the viewer sees them",
            read.value.as_deref().unwrap_or(&[]),
        );
    }
    show(
        &format!("manager reads employee #{}", staff_employment.id),
        workforce
            .employee(&manager, COMPANY, staff_employment.id)
            .await,
    );
    show(
        &format!(
            "manager reads employee #{} in company {COMPANY}",
            contractor_employment.id
        ),
        workforce
            .employee(&manager, COMPANY, contractor_employment.id)
            .await,
    );
    if let Some(read) = show(
        "manager reads employees by id (both, one twice, one that does not exist)",
        workforce
            .employees_by_ids(
                &manager,
                COMPANY,
                &[
                    staff_employment.id,
                    manager_employment.id,
                    staff_employment.id,
                    i64::MAX,
                ],
            )
            .await,
    ) {
        print_employees("employees by id", read.value.as_deref().unwrap_or(&[]));
    }
    show(
        "manager reads employees by 1,001 ids",
        workforce
            .employees_by_ids(&manager, COMPANY, &vec![1; 1_001])
            .await,
    );
    show(
        "manager reads employees by an id of zero",
        workforce.employees_by_ids(&manager, COMPANY, &[0]).await,
    );
    if let Some(read) = show(
        &format!(
            "manager reads employee #{}'s direct reports",
            manager_employment.id
        ),
        workforce
            .direct_reports(&manager, COMPANY, manager_employment.id)
            .await,
    ) {
        print_employees(
            "direct reports of the manager",
            read.value.as_deref().unwrap_or(&[]),
        );
    }
    show(
        &format!(
            "manager reads employee #{}'s direct reports",
            staff_employment.id
        ),
        workforce
            .direct_reports(&manager, COMPANY, staff_employment.id)
            .await,
    );
    if let Some(read) = show(
        &format!("manager lists company {OTHER_COMPANY}'s employees"),
        workforce.employees(&manager, OTHER_COMPANY).await,
    ) {
        print_employees(
            "working employees of Company 002",
            read.value.as_deref().unwrap_or(&[]),
        );
    }
    show(
        "manager lists a company that does not exist",
        workforce.employees(&manager, i64::MAX).await,
    );
    show(
        &format!("manager lists tenant 2's company {OTHER_TENANT_COMPANY}"),
        workforce.employees(&manager, OTHER_TENANT_COMPANY).await,
    );

    // Working statuses: the admin narrows them to `active`, so Sam Staff
    // (on probation) leaves the working staff until the default returns.
    show(
        "manager (Tenant Owner, without the settings capability) sets the working statuses",
        workforce
            .put_working_statuses(&manager, COMPANY, &["active".into()])
            .await,
    );
    show(
        "admin sets the working statuses to 'fired'",
        workforce
            .put_working_statuses(&admin, COMPANY, &["fired".into()])
            .await,
    );
    show(
        "admin sets the working statuses to nothing",
        workforce.put_working_statuses(&admin, COMPANY, &[]).await,
    );
    show(
        "admin sets the working statuses to active only",
        workforce
            .put_working_statuses(&admin, COMPANY, &["active".into()])
            .await,
    );
    if let Some(read) = show(
        &format!("manager lists company {COMPANY}'s employees"),
        workforce.employees(&manager, COMPANY).await,
    ) {
        print_employees(
            "working employees with active only",
            read.value.as_deref().unwrap_or(&[]),
        );
    }
    show(
        &format!(
            "manager reads employee #{} while probation is not a working status",
            staff_employment.id
        ),
        workforce
            .employee(&manager, COMPANY, staff_employment.id)
            .await,
    );
    show(
        "admin restores active and probation (given out of order)",
        workforce
            .put_working_statuses(&admin, COMPANY, &["active".into(), "probation".into()])
            .await,
    );
    show(
        &format!("manager reads company {COMPANY}'s working statuses"),
        workforce.working_statuses(&manager, COMPANY).await,
    );

    // The Authorization helpers: who may reach which company for what.
    describe(
        "manager's selectable companies for employment.manage",
        authorization::selectable_companies(&**client, &manager, EMPLOYMENT_MANAGE)
            .await
            .map(|companies| companies.len()),
    );
    describe(
        "viewer's selectable companies for employment.manage",
        authorization::selectable_companies(&**client, &viewer, EMPLOYMENT_MANAGE)
            .await
            .map(|companies| companies.len()),
    );
    describe(
        "contractor's selectable companies for employment.view",
        authorization::selectable_companies(&**client, &contractor, EMPLOYMENT_VIEW)
            .await
            .map(|companies| companies.len()),
    );
    describe(
        &format!("manager is authorized for employment.view in company {COMPANY}"),
        authorization::authorize_company(&**client, &manager, COMPANY, EMPLOYMENT_VIEW)
            .await
            .map(|company| format!("{} ({})", company.name, company.code)),
    );
    describe(
        &format!("viewer is authorized for employment.view in company {COMPANY}"),
        authorization::authorize(&**client, &viewer, COMPANY, EMPLOYMENT_VIEW).await,
    );
    describe(
        &format!("manager is authorized for settings.manage in company {COMPANY}"),
        authorization::authorize(&**client, &manager, COMPANY, SETTINGS_MANAGE).await,
    );
    describe(
        &format!("admin is authorized for settings.manage in company {OTHER_TENANT_COMPANY}"),
        authorization::authorize(&**client, &admin, OTHER_TENANT_COMPANY, SETTINGS_MANAGE).await,
    );
    describe(
        "manager is authorized for a capability nobody declared",
        authorization::authorize(&**client, &manager, COMPANY, "people.nothing.view").await,
    );
    println!(
        "manager may manage employments in company {OTHER_COMPANY}: {}",
        authorization::allowed(&**client, &manager, OTHER_COMPANY, EMPLOYMENT_MANAGE).await
    );

    // PostgreSQL itself refuses what the facade never attempts.
    for (step, sql) in [
        (
            "SQL sets an employment's status to 'fired'",
            "UPDATE people_workforce_employments SET status='fired' WHERE id=$1",
        ),
        (
            "SQL makes an employment its own supervisor",
            "UPDATE people_workforce_employments SET supervisor_employment_id=id WHERE id=$1",
        ),
        (
            "SQL moves an employment to Company 002",
            "UPDATE people_workforce_employments SET company_id=2 WHERE id=$1",
        ),
        (
            "SQL ends an employment before it started",
            "UPDATE people_workforce_employments SET employment_end=employment_start-1 WHERE id=$1",
        ),
        (
            "SQL inserts a second employment of the manager's affiliation",
            "INSERT INTO people_workforce_employments(tenant_id,company_id,person_id,affiliation_id,employee_number,employment_type,status) SELECT tenant_id,company_id,person_id,affiliation_id,'E-X',employment_type,status FROM people_workforce_employments WHERE id=$1",
        ),
        (
            "SQL gives the contractor the manager's supervisor from Company 001",
            "UPDATE people_workforce_employments SET supervisor_employment_id=$1 WHERE company_id=2",
        ),
    ] {
        refused(&client, step, sql, manager_employment.id).await;
    }

    println!(
        "Run bilibi-web against this database and sign in as manager@example.test (password {}): the sidebar shows no People entry, because the People root and its groups have no leaf yet and Base Menu hides an empty container. staff@example.test is now linked to Sam Staff's affiliation with Company 001.",
        bilibi_walkthrough_fixture::PASSWORD
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Attendance core (port plan row PA1). Runs after `run` against the same
// database: the records above are already there, so this block only
// resolves the services again, grants the Attendance keys the fixture's
// accounts need, and exercises the rules, shift templates, clocking
// locations, clock facts with their day projection, and self-service days.
// ---------------------------------------------------------------------------

use bilibi_people_attendance::{
    Attendance, AttendanceError, ClockInput, Day, EventType, NewClockingLocation, NewShiftTemplate,
    RuleChanges, RULES_MANAGE, SELF_VIEW,
};
use jiff::{civil::time, Timestamp};
use rust_decimal::Decimal;

fn attendance_step<T: std::fmt::Debug>(
    step: &str,
    result: std::result::Result<T, AttendanceError>,
) -> Option<T> {
    match result {
        Ok(value) => {
            println!("{step}: {value:?}");
            Some(value)
        }
        Err(AttendanceError::Invalid(errors)) => {
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

fn print_days(heading: &str, days: &[Day]) {
    println!("{heading}: {} day(s)", days.len());
    for day in days {
        println!(
            "  {} {:?} in {} out {} worked {} min",
            day.on_date,
            day.status,
            day.first_in_at.map_or("-".to_owned(), |at| at.to_string()),
            day.last_out_at.map_or("-".to_owned(), |at| at.to_string()),
            day.worked_minutes
        );
    }
}

fn at(text: &str) -> Timestamp {
    text.parse().expect("a literal RFC 3339 instant")
}

fn decimal(text: &str) -> Decimal {
    text.parse().expect("a literal decimal")
}

/// The attendance core walkthrough; `url` is the database `run` prepared.
pub async fn attendance(url: &str) -> Result<()> {
    let db = pool(url)?;
    let modules = bilibi_module_registry::discover()?;
    let mut services = Services::default();
    services.provide("bootstrap.database", Arc::new(db.clone()))?;
    services.provide("bootstrap.composition", Arc::new(bilibi_web::composition()))?;
    services.install(&modules)?;
    if std::env::var("BILIBI_ENV").as_deref() == Ok("development") {
        services
            .get::<bilibi_base::step_up::StepUp>(bilibi_base::step_up::SERVICE)?
            .set_enforced(false);
    }
    let grants = services.get::<Grants>("base/authz.grants")?;
    let authentication = services.get::<Authentication>("base/tenancy.authentication")?;
    let workforce = services.get::<Workforce>(SERVICE)?;
    let attendance = services.get::<Attendance>(bilibi_people_attendance::SERVICE)?;
    let client = db.get().await?;
    let admin = authentication
        .authenticate(&**client, TENANT, ADMIN, "walkthrough".into())
        .await?;
    // Bilimbi grants the Attendance keys to no system role; the admin, who
    // holds every key, grants the manager the rules and the staff account
    // its own view.
    grants
        .set_direct(&admin, MANAGER, RULES_MANAGE, Some(true))
        .await?;
    grants
        .set_direct(&admin, STAFF, SELF_VIEW, Some(true))
        .await?;
    println!("admin grants manager@example.test {RULES_MANAGE} and staff@example.test {SELF_VIEW}");
    let manager = authentication
        .authenticate(&**client, TENANT, MANAGER, "walkthrough".into())
        .await?;
    let viewer = authentication
        .authenticate(&**client, TENANT, VIEWER, "walkthrough".into())
        .await?;
    let staff = authentication
        .authenticate(&**client, TENANT, STAFF, "walkthrough".into())
        .await?;
    let employees = workforce
        .employees(&manager, COMPANY)
        .await
        .map_err(|_| Error::Internal)?
        .value
        .unwrap_or_default();
    let employee_id = |number: &str| -> Result<i64> {
        employees
            .iter()
            .find(|employee| employee.employee_number == number)
            .and_then(|employee| employee.reference.stable_id.parse().ok())
            .ok_or(Error::NotFound)
    };
    let manager_employee = employee_id("E-001")?;
    let staff_employee = employee_id("E-002")?;

    // Rules: the defaults, the refusals, then the company's own values.
    attendance_step(
        &format!("manager reads company {COMPANY}'s attendance rules"),
        attendance.rules(&manager, COMPANY).await,
    );
    attendance_step(
        "viewer (without the grant) sets the time zone",
        attendance
            .put_rules(
                &viewer,
                COMPANY,
                &RuleChanges {
                    timezone: Some("Asia/Kuala_Lumpur".into()),
                    ..RuleChanges::default()
                },
            )
            .await,
    );
    attendance_step(
        "manager sets a time zone the database does not know",
        attendance
            .put_rules(
                &manager,
                COMPANY,
                &RuleChanges {
                    timezone: Some("Mars/Olympus_Mons".into()),
                    ..RuleChanges::default()
                },
            )
            .await,
    );
    attendance_step(
        "manager sets a 25-hour shift length and a 0-day window",
        attendance
            .put_rules(
                &manager,
                COMPANY,
                &RuleChanges {
                    max_shift_hours: Some(25),
                    adjustment_window_days: Some(0),
                    ..RuleChanges::default()
                },
            )
            .await,
    );
    attendance_step(
        "manager sets Asia/Kuala_Lumpur, employee clocking on, 12-hour shifts, a 14-day window",
        attendance
            .put_rules(
                &manager,
                COMPANY,
                &RuleChanges {
                    timezone: Some("Asia/Kuala_Lumpur".into()),
                    self_clock_enabled: Some(true),
                    max_shift_hours: Some(12),
                    location_required: Some(false),
                    adjustment_window_days: Some(14),
                },
            )
            .await,
    );
    attendance_step(
        &format!("manager sets company {OTHER_TENANT_COMPANY}'s rules"),
        attendance
            .put_rules(
                &manager,
                OTHER_TENANT_COMPANY,
                &RuleChanges {
                    max_shift_hours: Some(8),
                    ..RuleChanges::default()
                },
            )
            .await,
    );

    // Shift templates: add, refuse, retire.
    attendance_step(
        "manager adds shift DAY 09:00-18:00 with a 60-minute break",
        attendance
            .create_shift_template(
                &manager,
                COMPANY,
                &NewShiftTemplate {
                    code: "DAY".into(),
                    name: "Day shift".into(),
                    starts_at: time(9, 0, 0, 0),
                    ends_at: time(18, 0, 0, 0),
                    break_minutes: 60,
                },
            )
            .await,
    );
    let night = attendance_step(
        "manager adds shift NIGHT 22:00-06:00 with a 30-minute break (crosses midnight)",
        attendance
            .create_shift_template(
                &manager,
                COMPANY,
                &NewShiftTemplate {
                    code: "NIGHT".into(),
                    name: "Night shift".into(),
                    starts_at: time(22, 0, 0, 0),
                    ends_at: time(6, 0, 0, 0),
                    break_minutes: 30,
                },
            )
            .await,
    );
    if let Some(night) = &night {
        println!(
            "  NIGHT spans {} minutes, {} to {}",
            night.span_minutes(),
            night.starts_at(),
            night.ends_at()
        );
    }
    attendance_step(
        "manager adds shift DAY again",
        attendance
            .create_shift_template(
                &manager,
                COMPANY,
                &NewShiftTemplate {
                    code: "DAY".into(),
                    name: "Second day shift".into(),
                    starts_at: time(8, 0, 0, 0),
                    ends_at: time(17, 0, 0, 0),
                    break_minutes: 0,
                },
            )
            .await,
    );
    attendance_step(
        "manager adds a shift ending when it starts, with a code starting with a dot",
        attendance
            .create_shift_template(
                &manager,
                COMPANY,
                &NewShiftTemplate {
                    code: ".odd".into(),
                    name: "Odd".into(),
                    starts_at: time(9, 0, 0, 0),
                    ends_at: time(9, 0, 0, 0),
                    break_minutes: 0,
                },
            )
            .await,
    );
    attendance_step(
        "manager adds a 2-hour shift with a 3-hour break",
        attendance
            .create_shift_template(
                &manager,
                COMPANY,
                &NewShiftTemplate {
                    code: "SHORT".into(),
                    name: "Short".into(),
                    starts_at: time(9, 0, 0, 0),
                    ends_at: time(11, 0, 0, 0),
                    break_minutes: 180,
                },
            )
            .await,
    );
    attendance_step(
        "viewer adds a shift",
        attendance
            .create_shift_template(
                &viewer,
                COMPANY,
                &NewShiftTemplate {
                    code: "X".into(),
                    name: "X".into(),
                    starts_at: time(9, 0, 0, 0),
                    ends_at: time(10, 0, 0, 0),
                    break_minutes: 0,
                },
            )
            .await,
    );
    if let Some(night) = &night {
        attendance_step(
            "manager retires NIGHT",
            attendance
                .set_shift_template_status(&manager, COMPANY, night.id, "retired")
                .await,
        );
        attendance_step(
            "manager deletes NIGHT",
            attendance
                .set_shift_template_status(&manager, COMPANY, night.id, "deleted")
                .await,
        );
        attendance_step(
            &format!("manager retires NIGHT in company {OTHER_COMPANY}"),
            attendance
                .set_shift_template_status(&manager, OTHER_COMPANY, night.id, "retired")
                .await,
        );
    }
    attendance_step(
        "manager lists the shift templates",
        attendance.list_shift_templates(&manager, COMPANY).await,
    );

    // Clocking locations: add, refuse, retire.
    let hq = attendance_step(
        "manager adds clocking location HQ (3.139003, 101.686855) radius 200 m",
        attendance
            .create_clocking_location(
                &manager,
                COMPANY,
                &NewClockingLocation {
                    code: "HQ".into(),
                    name: "Head office".into(),
                    latitude: decimal("3.139003"),
                    longitude: decimal("101.686855"),
                    radius_meters: 200,
                },
            )
            .await,
    );
    attendance_step(
        "manager adds a location at latitude 91 with a radius of 0",
        attendance
            .create_clocking_location(
                &manager,
                COMPANY,
                &NewClockingLocation {
                    code: "POLE".into(),
                    name: "Beyond the pole".into(),
                    latitude: decimal("91"),
                    longitude: decimal("0"),
                    radius_meters: 0,
                },
            )
            .await,
    );
    attendance_step(
        "manager adds HQ again",
        attendance
            .create_clocking_location(
                &manager,
                COMPANY,
                &NewClockingLocation {
                    code: "HQ".into(),
                    name: "Another head office".into(),
                    latitude: decimal("3.1"),
                    longitude: decimal("101.6"),
                    radius_meters: 50,
                },
            )
            .await,
    );

    // Clock facts and the day projection, in Asia/Kuala_Lumpur (UTC+8).
    let clock = |key: &str, event_type: EventType, instant: &str| {
        ClockInput::new(key, event_type, "device", at(instant))
    };
    attendance_step(
        "device clocks staff in at 2026-10-21T01:00:00Z (09:00 local), key k1",
        attendance
            .record_clock(
                &manager,
                COMPANY,
                staff_employee,
                &clock("k1", EventType::In, "2026-10-21T01:00:00Z"),
            )
            .await,
    );
    attendance_step(
        "device replays k1 with the same facts",
        attendance
            .record_clock(
                &manager,
                COMPANY,
                staff_employee,
                &clock("k1", EventType::In, "2026-10-21T01:00:00Z"),
            )
            .await,
    );
    attendance_step(
        "device replays k1 as a clock-out",
        attendance
            .record_clock(
                &manager,
                COMPANY,
                staff_employee,
                &clock("k1", EventType::Out, "2026-10-21T01:00:00Z"),
            )
            .await,
    );
    attendance_step(
        "device clocks staff out at 2026-10-21T10:00:00Z (18:00 local), key k2",
        attendance
            .record_clock(
                &manager,
                COMPANY,
                staff_employee,
                &clock("k2", EventType::Out, "2026-10-21T10:00:00Z"),
            )
            .await,
    );
    attendance_step(
        "device clocks staff in at 2026-10-22T14:00:00Z (22:00 local), key k3",
        attendance
            .record_clock(
                &manager,
                COMPANY,
                staff_employee,
                &clock("k3", EventType::In, "2026-10-22T14:00:00Z"),
            )
            .await,
    );
    attendance_step(
        "device clocks staff out at 2026-10-22T22:00:00Z (06:00 local next day), key k4",
        attendance
            .record_clock(
                &manager,
                COMPANY,
                staff_employee,
                &clock("k4", EventType::Out, "2026-10-22T22:00:00Z"),
            )
            .await,
    );
    attendance_step(
        "device clocks staff in at 2026-10-24T00:00:00Z (08:00 local), key k5",
        attendance
            .record_clock(
                &manager,
                COMPANY,
                staff_employee,
                &clock("k5", EventType::In, "2026-10-24T00:00:00Z"),
            )
            .await,
    );
    attendance_step(
        "device clocks staff out 13 hours later at 2026-10-24T13:00:00Z (21:00 local), key k6",
        attendance
            .record_clock(
                &manager,
                COMPANY,
                staff_employee,
                &clock("k6", EventType::Out, "2026-10-24T13:00:00Z"),
            )
            .await,
    );
    attendance_step(
        "device clocks staff out with no clock-in at 2026-10-25T05:00:00Z, key k7",
        attendance
            .record_clock(
                &manager,
                COMPANY,
                staff_employee,
                &clock("k7", EventType::Out, "2026-10-25T05:00:00Z"),
            )
            .await,
    );
    attendance_step(
        "device clocks staff in with a 161-byte key",
        attendance
            .record_clock(
                &manager,
                COMPANY,
                staff_employee,
                &clock(&"k".repeat(161), EventType::In, "2026-10-26T01:00:00Z"),
            )
            .await,
    );
    attendance_step(
        "device clocks staff in with a latitude and no longitude",
        attendance
            .record_clock(
                &manager,
                COMPANY,
                staff_employee,
                &ClockInput {
                    latitude: Some(decimal("3.139")),
                    ..clock("k8", EventType::In, "2026-10-26T01:00:00Z")
                },
            )
            .await,
    );
    attendance_step(
        &format!("device clocks employee #{manager_employee} in company {OTHER_COMPANY}"),
        attendance
            .record_clock(
                &manager,
                OTHER_COMPANY,
                manager_employee,
                &clock("k9", EventType::In, "2026-10-26T01:00:00Z"),
            )
            .await,
    );

    // The location rule: coordinates against the active geofences.
    attendance_step(
        "manager requires a clocking location",
        attendance
            .put_rules(
                &manager,
                COMPANY,
                &RuleChanges {
                    location_required: Some(true),
                    ..RuleChanges::default()
                },
            )
            .await,
    );
    attendance_step(
        "device clocks staff in without coordinates while a location is required",
        attendance
            .record_clock(
                &manager,
                COMPANY,
                staff_employee,
                &clock("k10", EventType::In, "2026-10-27T01:00:00Z"),
            )
            .await,
    );
    attendance_step(
        "device clocks staff in from (1.35, 103.82), outside every location",
        attendance
            .record_clock(
                &manager,
                COMPANY,
                staff_employee,
                &ClockInput {
                    latitude: Some(decimal("1.35")),
                    longitude: Some(decimal("103.82")),
                    ..clock("k11", EventType::In, "2026-10-27T01:00:00Z")
                },
            )
            .await,
    );
    attendance_step(
        "device clocks staff in from (3.1391, 101.6869), 60 m from HQ",
        attendance
            .record_clock(
                &manager,
                COMPANY,
                staff_employee,
                &ClockInput {
                    latitude: Some(decimal("3.1391")),
                    longitude: Some(decimal("101.6869")),
                    ..clock("k12", EventType::In, "2026-10-27T01:00:00Z")
                },
            )
            .await,
    );
    if let Some(hq) = &hq {
        attendance_step(
            "manager retires HQ",
            attendance
                .set_clocking_location_status(&manager, COMPANY, hq.id, "retired")
                .await,
        );
    }
    attendance_step(
        "device clocks staff out from the same point once HQ is retired",
        attendance
            .record_clock(
                &manager,
                COMPANY,
                staff_employee,
                &ClockInput {
                    latitude: Some(decimal("3.1391")),
                    longitude: Some(decimal("101.6869")),
                    ..clock("k13", EventType::Out, "2026-10-27T10:00:00Z")
                },
            )
            .await,
    );
    attendance_step(
        "manager stops requiring a clocking location",
        attendance
            .put_rules(
                &manager,
                COMPANY,
                &RuleChanges {
                    location_required: Some(false),
                    ..RuleChanges::default()
                },
            )
            .await,
    );
    attendance_step(
        "device clocks staff out from the same point, no location required",
        attendance
            .record_clock(
                &manager,
                COMPANY,
                staff_employee,
                &ClockInput {
                    latitude: Some(decimal("3.1391")),
                    longitude: Some(decimal("101.6869")),
                    ..clock("k13", EventType::Out, "2026-10-27T10:00:00Z")
                },
            )
            .await,
    );
    attendance_step(
        "manager lists the clocking locations",
        attendance.list_clocking_locations(&manager, COMPANY).await,
    );

    // The days as the operator and the employee see them.
    if let Some(days) = attendance_step(
        &format!("manager lists employee #{staff_employee}'s days"),
        attendance
            .list_days(&manager, COMPANY, staff_employee)
            .await,
    ) {
        print_days("Sam Staff's days (newest first)", &days);
    }
    if let Some(days) = attendance_step(
        "staff@example.test (linked to Sam Staff) reads their own days",
        attendance.self_days(&staff, COMPANY).await,
    ) {
        print_days("own days", &days);
    }
    attendance_step(
        "viewer (no self view grant) reads their own days",
        attendance.self_days(&viewer, COMPANY).await,
    );
    attendance_step(
        "admin (every grant, no affiliation) reads their own days",
        attendance.self_days(&admin, COMPANY).await,
    );
    attendance_step(
        &format!("staff reads their own days in company {OTHER_COMPANY}"),
        attendance.self_days(&staff, OTHER_COMPANY).await,
    );
    attendance_step(
        "manager (an employee, no self view grant) reads their own days",
        attendance.self_days(&manager, COMPANY).await,
    );

    // PostgreSQL itself refuses what the facade never attempts.
    for (step, sql) in [
        (
            "SQL sets a day's status to 'done'",
            "UPDATE people_attendance_daily_summaries SET status='done' WHERE employee_id=$1",
        ),
        (
            "SQL records a 'lunch' clock fact",
            "UPDATE people_attendance_clock_facts SET event_type='lunch' WHERE employee_id=$1",
        ),
        (
            "SQL drops a fact's longitude but keeps its latitude",
            "UPDATE people_attendance_clock_facts SET longitude=NULL WHERE employee_id=$1 AND latitude IS NOT NULL",
        ),
        (
            "SQL inserts a second fact with key k1 from the device",
            "INSERT INTO people_attendance_clock_facts(tenant_id,company_id,employee_id,day_id,event_key,event_type,source,occurred_at,timezone) SELECT tenant_id,company_id,employee_id,day_id,'k1','in','device',occurred_at,timezone FROM people_attendance_clock_facts WHERE employee_id=$1 LIMIT 1",
        ),
        (
            "SQL makes a shift end when it starts",
            "UPDATE people_attendance_shift_definitions SET end_minute=start_minute WHERE company_id=$1",
        ),
        (
            "SQL gives a clocking location a radius of 0",
            "UPDATE people_attendance_clocking_locations SET radius_meters=0 WHERE company_id=$1",
        ),
    ] {
        let id = if sql.contains("company_id=$1") {
            COMPANY
        } else {
            staff_employee
        };
        refused(&client, step, sql, id).await;
    }
    println!(
        "The attendance core keeps no page yet: the rules page, My attendance, rosters and approvals land with their rows (PA3 to PA5)."
    );
    Ok(())
}
