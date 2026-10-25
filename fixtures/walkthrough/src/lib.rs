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
//! A `bilibi-web run` against the same database lets the manager open
//! People references under People > Settings and the Training catalog pages.
//! See the fixture README for adding independent proof files.
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
use bilibi_module_registry::Services;
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

use bilibi_people_attendance::{
    Attendance, AttendanceError, ClockInput, Day, EventType, NewClockingLocation, NewShiftTemplate,
    RuleChanges, RULES_MANAGE, SELF_VIEW,
};
use jiff::{civil::time, Timestamp};

#[macro_use]
mod registry;
use registry::Context;

include!(concat!(env!("OUT_DIR"), "/proofs.rs"));

/// Runs the automatically collected proofs after the shared Workforce setup.
pub async fn run(url: &str) -> Result<()> {
    let steps = registry::steps()?;
    let mut context = workforce::prepare(url).await?;
    for step in steps {
        println!("walkthrough step: {}", step.id);
        (step.run)(&mut context).await?;
    }
    println!(
        "Run bilibi-web against this database and sign in as manager@example.test (password {}): People > Settings > People references is available; Courses and Sessions & calendar are available. staff@example.test is linked to Sam Staff's affiliation with Company 001 for the Learning pages.",
        bilibi_walkthrough_fixture::PASSWORD
    );
    Ok(())
}

/// Runs one registered standalone proof. Existing binaries keep their commands.
pub async fn command(name: &str, url: &str, argument: Option<&str>) -> Result<()> {
    registry::command(name, url, argument).await
}

/// Prints the validated default order and standalone commands without a database.
pub fn list() -> Result<()> {
    registry::list()
}
