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

    // Leave (slice PL1): its own block, in `leave.rs`.
    leave::run(
        &services,
        &client,
        &admin,
        &manager,
        &viewer,
        MANAGER,
        COMPANY,
        OTHER_COMPANY,
        manager_employment.id,
        staff_employment.id,
        contractor_employment.id,
    )
    .await?;

    // Claims (slice PC1): currencies, catalog, policies and assignments, on
    // the employments created above.
    claims::run(
        &services,
        &client,
        claims::Context {
            admin: &admin,
            manager: &manager,
            viewer: &viewer,
            contractor: &contractor,
            staff_user: STAFF,
            tenant: TENANT,
            company: COMPANY,
            other_company: OTHER_COMPANY,
            manager_employment: manager_employment.id,
            staff_employment: staff_employment.id,
        },
    )
    .await?;

    // PW1: employment records and self-service.
    pw1(
        &services,
        &client,
        &admin,
        &manager,
        &viewer,
        &contractor,
        &manager_employment,
        &staff_employment,
        &contractor_employment,
    )
    .await?;

    // Claims assignment pickers include the pending employment PW1 created.
    claims::all_status_members(&services, &manager, COMPANY).await?;

    println!(
        "Run bilibi-web against this database and sign in as manager@example.test (password {}): the sidebar shows no People entry, because the People root and its groups have no leaf yet and Base Menu hides an empty container. staff@example.test was linked to Sam Staff's affiliation with Company 001 and unlinked again at the end of the self-service steps.",
        bilibi_walkthrough_fixture::PASSWORD
    );
    Ok(())
}

// Slice blocks, each its own module, added at the end so parallel slices
// do not clash on merge.
mod claims;
// ---------------------------------------------------------------------------
// PW1: employment records and self-service. Reads the private employment
// facts and the summary, changes and ends employments through the
// version-fenced `update_employment` (type, status, dates, supervisor,
// department) and shows its refusals (a stale version, a reporting cycle, a
// supervisor who is not working, an unknown department, another company's
// employee, an actor without the grant), walks the supervisor chain and the
// reach, resolves each account's own employee and proves it under the
// employment lock, shows the refusals the slice names (an affiliation of
// another company, a duplicate number, an unlinked account, a sibling
// company without reach), and asks for positions while Organisation is not
// mounted.
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
async fn pw1(
    services: &Services,
    client: &tokio_postgres::Client,
    admin: &Scope,
    manager: &Scope,
    viewer: &Scope,
    contractor: &Scope,
    manager_employment: &Employment,
    staff_employment: &Employment,
    contractor_employment: &Employment,
) -> Result<()> {
    use bilibi_company::{departments, reference_types, CompanyService};
    use bilibi_people_workforce::{EmploymentChange, PositionScan};

    let workforce = services.get::<Workforce>(SERVICE)?;
    let persons = services.get::<PersonService>(bilibi_person::SERVICE)?;
    let affiliations = services.get::<AffiliationService>(bilibi_affiliation::SERVICE)?;
    let users = services.get::<UserService>("core/user.accounts")?;
    let companies = services.get::<CompanyService>(bilibi_company::SERVICE)?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    let authentication = services.get::<Authentication>("base/tenancy.authentication")?;
    let staff = authentication
        .authenticate(client, TENANT, STAFF, "walkthrough".into())
        .await?;
    println!("\n== PW1: employment records");

    // The private facts, under the view capability, at any status.
    if let Some(read) = show(
        &format!(
            "manager reads employee #{}'s employment",
            staff_employment.id
        ),
        workforce
            .employment(manager, COMPANY, staff_employment.id)
            .await,
    ) {
        if let Some(employment) = read.value {
            print_employment("read", &employment);
        }
    }
    show(
        &format!(
            "viewer reads employee #{}'s employment",
            staff_employment.id
        ),
        workforce
            .employment(viewer, COMPANY, staff_employment.id)
            .await,
    );
    show(
        &format!(
            "manager reads employee #{}'s employment in company {COMPANY}",
            contractor_employment.id
        ),
        workforce
            .employment(manager, COMPANY, contractor_employment.id)
            .await,
    );
    show(
        &format!("manager reads company {COMPANY}'s employment summary"),
        workforce.employment_summary(manager, COMPANY).await,
    );
    show(
        &format!("viewer reads company {COMPANY}'s employment summary"),
        workforce.employment_summary(viewer, COMPANY).await,
    );

    // A department of Company 001, through Core Company, for the change below.
    let department = match companies
        .create_department_type(
            admin,
            &reference_types::NewType {
                code: "PEOPLE-WALK".into(),
                name: "Walkthrough operations".into(),
                category: Some("operational".into()),
                description: None,
                is_active: None,
            },
        )
        .await?
    {
        reference_types::Outcome::Saved(department_type) => {
            match companies
                .create_department(admin, COMPANY, department_type.id, "active")
                .await
            {
                Ok(departments::Outcome::Saved(department)) => Some(department),
                Ok(departments::Outcome::Refused(errors)) => {
                    println!("admin adds the Walkthrough operations department to Company 001: refused ({errors:?})");
                    None
                }
                Err(error) => {
                    println!("admin adds the Walkthrough operations department to Company 001: refused ({error})");
                    None
                }
            }
        }
        reference_types::Outcome::Refused(errors) => {
            println!("admin creates the department type PEOPLE-WALK: refused ({errors:?})");
            None
        }
    };
    if let Some(department) = &department {
        println!(
            "admin adds the Walkthrough operations department to Company 001: department {}",
            department.id
        );
    }

    // Changes: type, status, dates, department and metadata, fenced on the
    // version the manager read.
    let changed = saved(show(
        &format!(
            "manager confirms employee #{} (version {}): permanent, active, department Walkthrough operations",
            staff_employment.id, staff_employment.version
        ),
        workforce
            .update_employment(
                manager,
                COMPANY,
                staff_employment.id,
                &EmploymentChange {
                    employment_type: Some("permanent".into()),
                    status: Some("active".into()),
                    employment_start: Some(Some(date(2026, 10, 1))),
                    department_id: Some(department.as_ref().map(|department| department.id)),
                    metadata: Some(Some(serde_json::json!({"confirmed": "2026-10-22"}))),
                    ..EmploymentChange::at(staff_employment.version)
                },
            )
            .await,
    ))?;
    print_employment("changed", &changed);
    show(
        &format!(
            "manager changes employee #{} again with the version already used ({})",
            staff_employment.id, staff_employment.version
        ),
        workforce
            .update_employment(
                manager,
                COMPANY,
                staff_employment.id,
                &EmploymentChange {
                    status: Some("inactive".into()),
                    ..EmploymentChange::at(staff_employment.version)
                },
            )
            .await,
    );
    show(
        &format!(
            "manager makes employee #{} report to their own report #{}",
            manager_employment.id, changed.id
        ),
        workforce
            .update_employment(
                manager,
                COMPANY,
                manager_employment.id,
                &EmploymentChange {
                    supervisor_employment_id: Some(Some(changed.id)),
                    ..EmploymentChange::at(manager_employment.version)
                },
            )
            .await,
    );
    show(
        &format!(
            "manager gives employee #{} the status 'fired' and an end before the start",
            changed.id
        ),
        workforce
            .update_employment(
                manager,
                COMPANY,
                changed.id,
                &EmploymentChange {
                    status: Some("fired".into()),
                    employment_end: Some(Some(date(2026, 9, 30))),
                    ..EmploymentChange::at(changed.version)
                },
            )
            .await,
    );
    show(
        &format!(
            "manager moves employee #{} to a department that does not exist",
            changed.id
        ),
        workforce
            .update_employment(
                manager,
                COMPANY,
                changed.id,
                &EmploymentChange {
                    department_id: Some(Some(i64::MAX)),
                    ..EmploymentChange::at(changed.version)
                },
            )
            .await,
    );
    show(
        &format!("viewer changes employee #{}", changed.id),
        workforce
            .update_employment(
                viewer,
                COMPANY,
                changed.id,
                &EmploymentChange {
                    status: Some("inactive".into()),
                    ..EmploymentChange::at(changed.version)
                },
            )
            .await,
    );
    show(
        &format!(
            "manager changes employee #{} in company {COMPANY}",
            contractor_employment.id
        ),
        workforce
            .update_employment(
                manager,
                COMPANY,
                contractor_employment.id,
                &EmploymentChange {
                    status: Some("inactive".into()),
                    ..EmploymentChange::at(contractor_employment.version)
                },
            )
            .await,
    );

    // A pending (not working) employee cannot supervise.
    let pending_person = persons
        .create(
            manager,
            &PersonInput {
                name: "Pat Pending".into(),
                kind: PersonKind::Human,
                email: None,
                phone: None,
            },
        )
        .await
        .map_err(|error| {
            println!("manager creates the Person Pat Pending: refused ({error})");
            Error::Internal
        })?;
    let pending_affiliation = affiliations
        .create(
            manager,
            &AffiliationInput {
                person_id: pending_person.id,
                organisation_id: OrganisationId::new(COMPANY).map_err(|_| Error::Invalid)?,
                title: Some("Trainee".into()),
                work_email: None,
                work_phone: None,
                dates: EffectiveDates::new(None, None).map_err(|_| Error::Invalid)?,
            },
        )
        .await?;
    let pending_employment = saved(show(
        "manager employs Pat Pending as pending, starting next year",
        workforce
            .create_employment(
                manager,
                COMPANY,
                &NewEmployment {
                    employment_start: Some(date(2027, 1, 4)),
                    supervisor_employment_id: Some(manager_employment.id),
                    ..NewEmployment::new(
                        pending_person.id.get(),
                        pending_affiliation.id.get(),
                        "E-003",
                        "full_time",
                        "pending",
                    )
                },
            )
            .await,
    ))?;
    print_employment("saved", &pending_employment);
    show(
        &format!(
            "manager makes employee #{} report to the pending employee #{}",
            changed.id, pending_employment.id
        ),
        workforce
            .update_employment(
                manager,
                COMPANY,
                changed.id,
                &EmploymentChange {
                    supervisor_employment_id: Some(Some(pending_employment.id)),
                    ..EmploymentChange::at(changed.version)
                },
            )
            .await,
    );

    // The supervisor chain and the reach.
    show(
        &format!("supervisor chain of employee #{}", changed.id),
        workforce
            .supervisor_chain(manager, COMPANY, changed.id)
            .await,
    );
    show(
        &format!("supervisor chain of employee #{}", manager_employment.id),
        workforce
            .supervisor_chain(manager, COMPANY, manager_employment.id)
            .await,
    );
    show(
        &format!(
            "supervisor chain of the pending employee #{}",
            pending_employment.id
        ),
        workforce
            .supervisor_chain(manager, COMPANY, pending_employment.id)
            .await,
    );
    show(
        &format!(
            "employee #{} is in employee #{}'s reach",
            changed.id, manager_employment.id
        ),
        workforce
            .in_reach(viewer, COMPANY, manager_employment.id, changed.id)
            .await,
    );
    show(
        &format!(
            "employee #{} is in employee #{}'s reach",
            manager_employment.id, changed.id
        ),
        workforce
            .in_reach(viewer, COMPANY, changed.id, manager_employment.id)
            .await,
    );
    show(
        &format!(
            "the pending employee #{} is in employee #{}'s reach",
            pending_employment.id, manager_employment.id
        ),
        workforce
            .in_reach(
                viewer,
                COMPANY,
                manager_employment.id,
                pending_employment.id,
            )
            .await,
    );

    // Self-service: each account's own employee, resolved now.
    println!("\n== PW1: self-service");
    show(
        &format!("manager's own employee in company {COMPANY}"),
        workforce.self_employee(manager, COMPANY).await,
    );
    show(
        &format!("staff's own employee in company {COMPANY}"),
        workforce.self_employee(&staff, COMPANY).await,
    );
    show(
        &format!("viewer's (unlinked account) own employee in company {COMPANY}"),
        workforce.self_employee(viewer, COMPANY).await,
    );
    show(
        &format!("contractor's own employee in company {COMPANY} (affiliated with Company 002)"),
        workforce.self_employee(contractor, COMPANY).await,
    );
    show(
        &format!("contractor's own employee in company {OTHER_COMPANY}"),
        workforce.self_employee(contractor, OTHER_COMPANY).await,
    );
    show(
        &format!("staff authorizes employment.view for themselves in company {COMPANY}"),
        workforce
            .authorize_self(&staff, COMPANY, EMPLOYMENT_VIEW)
            .await,
    );
    show(
        &format!("manager authorizes employment.view for themselves in company {COMPANY}"),
        workforce
            .authorize_self(manager, COMPANY, EMPLOYMENT_VIEW)
            .await,
    );
    show(
        "manager runs a self-service write under their own employment lock",
        workforce
            .with_self_employee_lock(
                manager,
                COMPANY,
                EMPLOYMENT_VIEW,
                "people.walkthrough.self_service",
                |tx, me| {
                    Box::pin(async move {
                        // The row is locked for this transaction; a read on it
                        // shows which employee the write would act for.
                        let row = tx
                            .sql()
                            .query_one(
                                "SELECT employee_number FROM people_workforce_employments WHERE id = $1",
                                &[&me.employee_id],
                            )
                            .await
                            .map_err(Error::from)?;
                        Ok(format!(
                            "acting for employee #{} ({})",
                            me.employee_id,
                            row.get::<_, String>(0)
                        ))
                    })
                },
            )
            .await,
    );
    show(
        "viewer runs a self-service write",
        workforce
            .with_self_employee_lock(
                viewer,
                COMPANY,
                EMPLOYMENT_VIEW,
                "people.walkthrough.self_service",
                |_tx, me| {
                    let id = me.employee_id;
                    Box::pin(async move { Ok(id) })
                },
            )
            .await,
    );
    // Ending the contractor's employment: terminated, with the last day.
    let ended = saved(show(
        &format!(
            "manager ends the contractor's employment #{} at Company 002",
            contractor_employment.id
        ),
        workforce
            .update_employment(
                manager,
                OTHER_COMPANY,
                contractor_employment.id,
                &EmploymentChange {
                    status: Some("terminated".into()),
                    employment_end: Some(Some(date(2026, 10, 31))),
                    ..EmploymentChange::at(contractor_employment.version)
                },
            )
            .await,
    ))?;
    print_employment("ended", &ended);
    if let Some(read) = show(
        &format!("manager lists company {OTHER_COMPANY}'s employees"),
        workforce.employees(manager, OTHER_COMPANY).await,
    ) {
        print_employees(
            "working employees of Company 002 after the end",
            read.value.as_deref().unwrap_or(&[]),
        );
    }
    show(
        &format!("contractor's own employee in company {OTHER_COMPANY} after the end"),
        workforce.self_employee(contractor, OTHER_COMPANY).await,
    );
    // The account unlinked while the employment stands: the next call refuses.
    match users.unlink(manager, STAFF).await {
        Ok(links) => println!(
            "manager unlinks staff@example.test from Sam Staff's affiliation: affiliation {:?}",
            links.affiliation
        ),
        Err(error) => println!("manager unlinks staff@example.test: refused ({error})"),
    }
    show(
        &format!("staff's own employee in company {COMPANY} after the unlink"),
        workforce.self_employee(&staff, COMPANY).await,
    );
    show(
        &format!(
            "manager reads employee #{}'s employment after the unlink",
            changed.id
        ),
        workforce.employment(manager, COMPANY, changed.id).await,
    );

    // A sibling company without reach: the contractor holds the employment
    // grants directly but no tenant-wide reach, so Company 002 is theirs to
    // write and Company 001 is not.
    println!("\n== PW1: reach over companies");
    for key in [EMPLOYMENT_MANAGE, bilibi_affiliation::contributions::PROVE] {
        match grants.set_direct(admin, CONTRACTOR, key, Some(true)).await {
            Ok(()) => println!("admin grants {key} to contractor@example.test directly"),
            Err(error) => {
                println!("admin grants {key} to contractor@example.test: refused ({error})")
            }
        }
    }
    let lead_person = persons
        .create(
            manager,
            &PersonInput {
                name: "Lee Lead".into(),
                kind: PersonKind::Human,
                email: None,
                phone: None,
            },
        )
        .await
        .map_err(|error| {
            println!("manager creates the Person Lee Lead: refused ({error})");
            Error::Internal
        })?;
    let lead_affiliation = affiliations
        .create(
            manager,
            &AffiliationInput {
                person_id: lead_person.id,
                organisation_id: OrganisationId::new(OTHER_COMPANY).map_err(|_| Error::Invalid)?,
                title: Some("Site Lead".into()),
                work_email: None,
                work_phone: None,
                dates: EffectiveDates::new(None, None).map_err(|_| Error::Invalid)?,
            },
        )
        .await?;
    show(
        "contractor employs Lee Lead at Company 001 (a sibling company without reach)",
        workforce
            .create_employment(
                contractor,
                COMPANY,
                &NewEmployment::new(
                    lead_person.id.get(),
                    lead_affiliation.id.get(),
                    "E-004",
                    "full_time",
                    "active",
                ),
            )
            .await,
    );
    show(
        "contractor employs Lee Lead at Company 002 with the contractor's own number",
        workforce
            .create_employment(
                contractor,
                OTHER_COMPANY,
                &NewEmployment::new(
                    lead_person.id.get(),
                    lead_affiliation.id.get(),
                    "C-001",
                    "full_time",
                    "active",
                ),
            )
            .await,
    );
    if let Some(lead) = show(
        "contractor employs Lee Lead at Company 002",
        workforce
            .create_employment(
                contractor,
                OTHER_COMPANY,
                &NewEmployment {
                    employment_start: Some(date(2026, 11, 2)),
                    ..NewEmployment::new(
                        lead_person.id.get(),
                        lead_affiliation.id.get(),
                        "C-002",
                        "full_time",
                        "active",
                    )
                },
            )
            .await,
    ) {
        print_employment("saved", &lead);
    }
    show(
        "manager employs Lee Lead's Company 002 affiliation at Company 001",
        workforce
            .create_employment(
                manager,
                COMPANY,
                &NewEmployment::new(
                    lead_person.id.get(),
                    lead_affiliation.id.get(),
                    "E-005",
                    "full_time",
                    "active",
                ),
            )
            .await,
    );
    show(
        &format!("manager reads company {OTHER_COMPANY}'s employment summary"),
        workforce.employment_summary(manager, OTHER_COMPANY).await,
    );

    // Positions come from Organisation, which is not mounted here.
    println!("\n== PW1: positions");
    println!(
        "a mounted module answers the positions port: {}",
        workforce.positions_available()
    );
    show(
        &format!("manager scans company {COMPANY}'s positions as of 2026-10-22"),
        workforce
            .positions(
                manager,
                COMPANY,
                &PositionScan::from_start(date(2026, 10, 22)),
            )
            .await,
    );
    show(
        "manager scans positions with a page size of 0",
        workforce
            .positions(
                manager,
                COMPANY,
                &PositionScan {
                    page_size: 0,
                    ..PositionScan::from_start(date(2026, 10, 22))
                },
            )
            .await,
    );
    Ok(())
}

// Leave (slice PL1).
mod leave;
