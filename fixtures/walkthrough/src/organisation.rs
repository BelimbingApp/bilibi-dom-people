//! The Organisation block of the People walkthrough (port plan row PO1):
//! positions, title versions and assignments in the fixture's Company 001,
//! run after the Workforce block has employed the manager, Sam Staff and the
//! contractor. The admin grants the manager the two Organisation
//! capabilities directly (Bilimbi grants them to no system role), then the
//! manager builds a small structure and the walkthrough shows every refusal
//! beside the write: the viewer without the grant, a duplicate or blank
//! code, a parent of another company, a cycle, an overlapping or repeated
//! version, an overlapping substantive assignment, an employee who is not
//! working in the company, an end that is not earlier. A substantive
//! assignment over a holder who has left the working statuses releases the
//! seat and records `people.organisation.assignment_released`; ending one
//! records `assignment_ended`. It reads the explorer's offset pages, walks
//! the keyset scan to completion and resumes it with tampered cursors, calls
//! the `people/organisation.positions` port as Workforce will, and ends with
//! PostgreSQL refusing what the facade never attempts.
use bilibi_authz::grants::Grants;
use bilibi_database::{Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_organisation::{
    Assignment, NewAssignment, NewPosition, NewVersion, Organisation, OrganisationError,
    PositionProjection, MANAGE, POSITIONS_PORT, SERVICE, VIEW,
};
use bilibi_people_workforce::Workforce;
use bilibi_ports::{PortError, Ports};
use bilibi_tenancy::Scope;
use jiff::civil::{date, Date};
use serde_json::{json, Value};
use tokio_postgres::Client;

fn show<T: std::fmt::Debug>(
    step: &str,
    result: std::result::Result<T, OrganisationError>,
) -> Option<T> {
    match result {
        Ok(value) => {
            println!("{step}: {value:?}");
            Some(value)
        }
        Err(OrganisationError::Invalid(errors)) => {
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

/// `show` for a read whose value `print_positions` lists line by line.
fn read<T>(step: &str, result: std::result::Result<T, OrganisationError>) -> Option<T> {
    match result {
        Ok(value) => {
            println!("{step}: ok");
            Some(value)
        }
        Err(error) => {
            show::<()>(step, Err(error));
            None
        }
    }
}

fn saved<T>(record: Option<T>) -> Result<T> {
    record.ok_or(Error::Internal)
}

fn print_positions(heading: &str, positions: &[PositionProjection]) {
    println!("{heading}: {} position(s)", positions.len());
    for position in positions {
        let holders = position
            .assignments
            .iter()
            .map(|holder| {
                format!(
                    "#{} employee #{} {}",
                    holder.reference.stable_id, holder.employee_reference.stable_id, holder.kind
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "  #{} {} \"{}\" v{} parent {}; {}; holders [{}]{}",
            position.reference.stable_id,
            position.code,
            position.title.as_deref().unwrap_or("(no title in force)"),
            position
                .version
                .map_or("-".to_owned(), |version| version.to_string()),
            position
                .parent_reference
                .as_ref()
                .map_or("none".to_owned(), |parent| format!("#{}", parent.stable_id)),
            if position.vacant {
                "vacant"
            } else {
                "occupied"
            },
            holders,
            if position.assignments_incomplete {
                " (incomplete)"
            } else {
                ""
            }
        );
    }
}

fn print_assignment(step: &str, assignment: &Assignment) {
    println!(
        "{step}: assignment #{} of employee #{} to position #{} ({}) from {} to {}",
        assignment.id,
        assignment.employee_id,
        assignment.position_id,
        assignment.kind,
        assignment.effective_from,
        assignment
            .effective_to
            .map_or("open".to_owned(), |date| date.to_string())
    );
}

async fn refused(client: &Client, step: &str, sql: &str, id: i64) {
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

fn utc_today() -> Date {
    jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date()
}

/// The fixture's actors and employments, as the Workforce block left them.
pub(crate) struct Cast<'a> {
    pub admin: &'a Scope,
    pub manager: &'a Scope,
    pub viewer: &'a Scope,
    pub company: i64,
    pub other_company: i64,
    pub manager_user: i64,
    pub manager_employment: i64,
    pub staff_employment: i64,
    pub contractor_employment: i64,
}

pub(crate) async fn run(services: &Services, client: &Client, cast: Cast<'_>) -> Result<()> {
    let Cast {
        admin,
        manager,
        viewer,
        company,
        other_company,
        manager_user,
        manager_employment,
        staff_employment,
        contractor_employment,
    } = cast;
    let organisation = services.get::<Organisation>(SERVICE)?;
    let workforce = services.get::<Workforce>(bilibi_people_workforce::SERVICE)?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    let ports = services.get::<Ports>(bilibi_ports::SERVICE)?;
    let today = utc_today();

    println!("\n== Organisation (PO1)");
    // Bilimbi grants the Organisation keys to no system role: the admin
    // grants them to the manager directly.
    show(
        "viewer creates the position CEO before any grant",
        organisation
            .create_position(viewer, company, &NewPosition::new("CEO"))
            .await,
    );
    show(
        "manager creates the position CEO before any grant",
        organisation
            .create_position(manager, company, &NewPosition::new("CEO"))
            .await,
    );
    for key in [VIEW, MANAGE] {
        grants
            .set_direct(admin, manager_user, key, Some(true))
            .await?;
        println!("admin grants manager@example.test {key}");
    }

    // The structure: CEO, COO under CEO, OPS-1 under COO; a GM at Company 002.
    let ceo = saved(show(
        "manager creates the position CEO",
        organisation
            .create_position(manager, company, &NewPosition::new("CEO"))
            .await,
    ))?;
    let coo = saved(show(
        "manager creates the position COO under CEO",
        organisation
            .create_position(
                manager,
                company,
                &NewPosition {
                    parent_id: Some(ceo.id),
                    ..NewPosition::new("COO")
                },
            )
            .await,
    ))?;
    let ops = saved(show(
        "manager creates the position OPS-1 under COO",
        organisation
            .create_position(
                manager,
                company,
                &NewPosition {
                    parent_id: Some(coo.id),
                    ..NewPosition::new(" OPS-1 ")
                },
            )
            .await,
    ))?;
    show(
        "manager creates the position CEO again",
        organisation
            .create_position(manager, company, &NewPosition::new("CEO"))
            .await,
    );
    show(
        "manager creates a position with a blank code",
        organisation
            .create_position(manager, company, &NewPosition::new("   "))
            .await,
    );
    show(
        "manager creates a position under a parent that does not exist",
        organisation
            .create_position(
                manager,
                company,
                &NewPosition {
                    parent_id: Some(i64::MAX),
                    ..NewPosition::new("CFO")
                },
            )
            .await,
    );
    let gm = saved(show(
        &format!(
            "manager, with tenant-wide reach, creates the position GM at company {other_company}"
        ),
        organisation
            .create_position(manager, other_company, &NewPosition::new("GM"))
            .await,
    ))?;
    show(
        &format!("manager creates a position at company {other_company} under Company 001's CEO"),
        organisation
            .create_position(
                manager,
                other_company,
                &NewPosition {
                    parent_id: Some(ceo.id),
                    ..NewPosition::new("GM-2")
                },
            )
            .await,
    );

    // Parents: company-bound and cycle-checked under the company lock.
    show(
        "manager moves CEO under OPS-1 (its own descendant)",
        organisation
            .set_parent(manager, company, ceo.id, Some(ops.id))
            .await,
    );
    show(
        "manager moves OPS-1 under itself",
        organisation
            .set_parent(manager, company, ops.id, Some(ops.id))
            .await,
    );
    show(
        &format!("manager moves OPS-1 under company {other_company}'s GM"),
        organisation
            .set_parent(manager, company, ops.id, Some(gm.id))
            .await,
    );
    show(
        "viewer moves OPS-1 to the root",
        organisation.set_parent(viewer, company, ops.id, None).await,
    );
    show(
        "manager moves OPS-1 under CEO",
        organisation
            .set_parent(manager, company, ops.id, Some(ceo.id))
            .await,
    );

    // Title versions: immutable, numbered, non-overlapping.
    show(
        "manager records CEO v1 \"Chief Executive Officer\" from 2024-01-01 to 2025-12-31",
        organisation
            .record_version(
                manager,
                company,
                ceo.id,
                &NewVersion {
                    version: 1,
                    title: "Chief Executive Officer".into(),
                    effective_from: date(2024, 1, 1),
                    effective_to: Some(date(2025, 12, 31)),
                },
            )
            .await,
    );
    show(
        "manager records CEO v1 again",
        organisation
            .record_version(
                manager,
                company,
                ceo.id,
                &NewVersion {
                    version: 1,
                    title: "Chief Executive".into(),
                    effective_from: date(2026, 1, 1),
                    effective_to: None,
                },
            )
            .await,
    );
    show(
        "manager records CEO v2 from 2025-06-01 (overlapping v1)",
        organisation
            .record_version(
                manager,
                company,
                ceo.id,
                &NewVersion {
                    version: 2,
                    title: "Chief Executive".into(),
                    effective_from: date(2025, 6, 1),
                    effective_to: None,
                },
            )
            .await,
    );
    show(
        "manager records CEO v0 with a blank title ending before it starts",
        organisation
            .record_version(
                manager,
                company,
                ceo.id,
                &NewVersion {
                    version: 0,
                    title: "  ".into(),
                    effective_from: date(2026, 1, 2),
                    effective_to: Some(date(2026, 1, 1)),
                },
            )
            .await,
    );
    show(
        "manager records CEO v2 \"Chief Executive\" from 2026-01-01",
        organisation
            .record_version(
                manager,
                company,
                ceo.id,
                &NewVersion {
                    version: 2,
                    title: "Chief Executive".into(),
                    effective_from: date(2026, 1, 1),
                    effective_to: None,
                },
            )
            .await,
    );
    show(
        "manager records OPS-1 v1 \"Operations Officer\" from 2026-01-01",
        organisation
            .record_version(
                manager,
                company,
                ops.id,
                &NewVersion {
                    version: 1,
                    title: "Operations Officer".into(),
                    effective_from: date(2026, 1, 1),
                    effective_to: None,
                },
            )
            .await,
    );
    show(
        &format!(
            "manager records a version for company {other_company}'s GM as a Company 001 position"
        ),
        organisation
            .record_version(
                manager,
                company,
                gm.id,
                &NewVersion {
                    version: 1,
                    title: "General Manager".into(),
                    effective_from: date(2026, 1, 1),
                    effective_to: None,
                },
            )
            .await,
    );

    // Assignments: only a working employee of the company; one substantive
    // holder per day; acting and concurrent cover may overlap.
    let substantive = |employee_id: i64, from: Date| NewAssignment {
        employee_id,
        kind: "substantive".into(),
        effective_from: from,
        effective_to: None,
    };
    let manager_ceo = saved(show(
        "manager assigns themselves substantively to CEO from 2024-01-01",
        organisation
            .assign(
                manager,
                company,
                ceo.id,
                &substantive(manager_employment, date(2024, 1, 1)),
            )
            .await,
    ))?;
    print_assignment("saved", &manager_ceo);
    show(
        "manager assigns Sam Staff substantively to CEO from 2026-10-01 (the manager still holds it)",
        organisation
            .assign(
                manager,
                company,
                ceo.id,
                &substantive(staff_employment, date(2026, 10, 1)),
            )
            .await,
    );
    let staff_acting = saved(show(
        "manager assigns Sam Staff as acting CEO from 2026-10-01",
        organisation
            .assign(
                manager,
                company,
                ceo.id,
                &NewAssignment {
                    kind: "acting".into(),
                    ..substantive(staff_employment, date(2026, 10, 1))
                },
            )
            .await,
    ))?;
    print_assignment("saved", &staff_acting);
    show(
        &format!("manager assigns the contractor (company {other_company}) to OPS-1"),
        organisation
            .assign(
                manager,
                company,
                ops.id,
                &substantive(contractor_employment, date(2026, 10, 1)),
            )
            .await,
    );
    show(
        "manager assigns Sam Staff to OPS-1 as 'temporary', ending before the start",
        organisation
            .assign(
                manager,
                company,
                ops.id,
                &NewAssignment {
                    kind: "temporary".into(),
                    effective_to: Some(date(2026, 9, 30)),
                    ..substantive(staff_employment, date(2026, 10, 1))
                },
            )
            .await,
    );
    show(
        "viewer assigns Sam Staff to OPS-1",
        organisation
            .assign(
                viewer,
                company,
                ops.id,
                &substantive(staff_employment, date(2026, 10, 1)),
            )
            .await,
    );
    show(
        "manager assigns Sam Staff to a position that does not exist",
        organisation
            .assign(
                manager,
                company,
                i64::MAX,
                &substantive(staff_employment, date(2026, 10, 1)),
            )
            .await,
    );
    let staff_ops = saved(show(
        "manager assigns Sam Staff substantively to OPS-1 from 2026-10-01",
        organisation
            .assign(
                manager,
                company,
                ops.id,
                &substantive(staff_employment, date(2026, 10, 1)),
            )
            .await,
    ))?;
    print_assignment("saved", &staff_ops);

    // The release: Sam Staff (on probation) leaves the working statuses, so a
    // new substantive OPS-1 holder ends Sam's seat the day before and records
    // the release; once probation counts again, nothing is released.
    show(
        "admin narrows the working statuses to active (Sam Staff, on probation, stops working)",
        workforce
            .put_working_statuses(admin, company, &["active".into()])
            .await
            .map_err(OrganisationError::from),
    );
    if let Some(read) = read(
        "manager reads positions as of 2026-10-15 while Sam Staff is not working",
        organisation
            .positions(manager, company, date(2026, 10, 15), 1, 50)
            .await,
    ) {
        print_positions("as of 2026-10-15, probation not working", &read);
    }
    let manager_ops = saved(show(
        "manager assigns themselves substantively to OPS-1 from 2026-11-01 (releases Sam Staff's seat)",
        organisation
            .assign(
                manager,
                company,
                ops.id,
                &substantive(manager_employment, date(2026, 11, 1)),
            )
            .await,
    ))?;
    print_assignment("saved", &manager_ops);
    show(
        "admin restores the working statuses to probation and active",
        workforce
            .put_working_statuses(admin, company, &["probation".into(), "active".into()])
            .await
            .map_err(OrganisationError::from),
    );
    show(
        "manager assigns Sam Staff substantively to CEO from 2027-01-01 (the manager, working, still holds it)",
        organisation
            .assign(
                manager,
                company,
                ceo.id,
                &substantive(staff_employment, date(2027, 1, 1)),
            )
            .await,
    );

    // Ending: only earlier, recorded as the actor's action.
    show(
        "viewer ends Sam Staff's acting CEO assignment",
        organisation
            .end_assignment(viewer, company, staff_acting.id, date(2026, 12, 31))
            .await,
    );
    show(
        "manager ends Sam Staff's acting CEO assignment before it started",
        organisation
            .end_assignment(manager, company, staff_acting.id, date(2026, 9, 30))
            .await,
    );
    show(
        "manager ends Sam Staff's acting CEO assignment on 2026-12-31",
        organisation
            .end_assignment(manager, company, staff_acting.id, date(2026, 12, 31))
            .await,
    );
    show(
        "manager ends it again on 2026-12-31 (not earlier)",
        organisation
            .end_assignment(manager, company, staff_acting.id, date(2026, 12, 31))
            .await,
    );
    show(
        "manager ends an assignment that does not exist",
        organisation
            .end_assignment(manager, company, i64::MAX, date(2026, 12, 31))
            .await,
    );
    show(
        &format!(
            "manager ends Sam Staff's OPS-1 assignment as a company {other_company} assignment"
        ),
        organisation
            .end_assignment(manager, other_company, staff_ops.id, date(2026, 10, 15))
            .await,
    );
    let actions = bilibi_audit::recent_actions(client, manager, 50).await?;
    for action in actions
        .iter()
        .filter(|action| action.event.starts_with("people.organisation."))
        .rev()
    {
        println!(
            "audit: {} by {} {}: {}",
            action.event, action.actor_type, action.actor_id, action.payload
        );
    }

    // The explorer's reads: offset pages as of a day, vacancies included.
    for (heading, day) in [
        ("as of 2024-06-01", date(2024, 6, 1)),
        ("as of 2026-10-15", date(2026, 10, 15)),
        ("as of 2026-12-01", date(2026, 12, 1)),
        ("as of today", today),
    ] {
        if let Some(read) = read(
            &format!("manager reads company {company}'s positions {heading}"),
            organisation.positions(manager, company, day, 1, 50).await,
        ) {
            print_positions(heading, &read);
        }
    }
    show(
        &format!("manager counts company {company}'s positions"),
        organisation.count_positions(manager, company).await,
    );
    if let Some(read) = read(
        &format!("viewer, without any grant, reads company {company}'s positions"),
        organisation.positions(viewer, company, today, 1, 50).await,
    ) {
        print_positions("as the viewer sees them", &read);
    }
    show(
        "manager reads page 2 of 2 positions",
        organisation
            .positions(manager, company, today, 2, 2)
            .await
            .map(|positions| positions.len()),
    );
    show(
        "manager reads 101 positions per page",
        organisation
            .positions(manager, company, today, 1, 101)
            .await,
    );
    show(
        "manager reads page 0",
        organisation.positions(manager, company, today, 0, 50).await,
    );
    show(
        "manager reads a company that does not exist",
        organisation
            .positions(manager, i64::MAX, today, 1, 50)
            .await,
    );

    // The keyset scan: a high-water mark on the first step, two per page.
    let mut cursor: Option<String> = None;
    let mut step = 1;
    let mut seen = Vec::new();
    loop {
        let Some(scan) = read(
            &format!("manager scans company {company}'s positions, step {step}, 2 per page"),
            organisation
                .scan_positions(manager, company, today, 2, cursor.as_deref())
                .await,
        ) else {
            break;
        };
        println!(
            "  step {step}: {} position(s), high-water id {}, next cursor {}",
            scan.positions.len(),
            scan.high_water_id,
            scan.next_cursor.as_deref().unwrap_or("none (complete)")
        );
        seen.extend(scan.positions.iter().map(|p| p.reference.stable_id.clone()));
        if step == 1 {
            if let Some(next) = scan.next_cursor.as_deref() {
                let other_day = utc_today().yesterday().map_err(|_| Error::Internal)?;
                show(
                    "manager resumes the scan with the same cursor for another day",
                    organisation
                        .scan_positions(manager, company, other_day, 2, Some(next))
                        .await,
                );
                show(
                    &format!(
                        "manager resumes the scan with the same cursor for company {other_company}"
                    ),
                    organisation
                        .scan_positions(manager, other_company, today, 2, Some(next))
                        .await,
                );
                show(
                    "manager resumes the scan with a tampered cursor",
                    organisation
                        .scan_positions(manager, company, today, 2, Some("dGFtcGVyZWQ"))
                        .await,
                );
            }
        }
        match scan.next_cursor {
            Some(next) => {
                cursor = Some(next);
                step += 1;
            }
            None => break,
        }
    }
    println!("scan saw positions {seen:?} in order");

    // PW1 consumes this exact port shape through Workforce's public facade.
    // Require successful decoding, including assignment references.
    let native = workforce
        .positions(
            manager,
            company,
            &bilibi_people_workforce::PositionScan::from_start(today),
        )
        .await
        .map_err(|_| Error::Internal)?
        .require_current()
        .map_err(|_| Error::Internal)?;
    if native.positions.len() != seen.len() {
        return Err(Error::Internal);
    }
    println!(
        "Workforce positions port integration: decoded {} positions and their assignment references, high-water id {}",
        native.positions.len(), native.high_water_id
    );

    // The port, as Workforce's `positions` scan calls it: the caller's
    // client and scope, JSON in and out.
    println!(
        "port {POSITIONS_PORT} available: {}",
        ports.available(POSITIONS_PORT)
    );
    let call = |request: Value| {
        let ports = ports.clone();
        async move {
            ports
                .call::<_, Value>(client, manager, POSITIONS_PORT, &request)
                .await
        }
    };
    match call(json!({ "company_id": company, "as_of": today.to_string(), "page_size": 2, "cursor": null })).await {
        Ok(answer) => println!(
            "port scan step 1: {}",
            serde_json::to_string(&answer).unwrap_or_default()
        ),
        Err(error) => println!("port scan step 1: refused ({error})"),
    }
    match call(json!({ "company_id": company, "as_of": "2026-10-15", "page": 1 })).await {
        Ok(answer) => println!(
            "port offset page as of 2026-10-15: {}",
            serde_json::to_string(&answer).unwrap_or_default()
        ),
        Err(error) => println!("port offset page: refused ({error})"),
    }
    for (step, request) in [
        (
            "port request with both page and cursor",
            json!({ "company_id": company, "as_of": today.to_string(), "page": 1, "cursor": null }),
        ),
        (
            "port request with an unknown key",
            json!({ "company_id": company, "as_of": today.to_string(), "page": 1, "limit": 5 }),
        ),
        (
            "port request with a malformed day",
            json!({ "company_id": company, "as_of": "yesterday", "page": 1 }),
        ),
        (
            "port request for a company that does not exist",
            json!({ "company_id": i64::MAX, "as_of": today.to_string(), "page": 1 }),
        ),
    ] {
        match call(request).await {
            Ok(answer) => println!("{step}: {answer}"),
            Err(PortError::Refused(error)) => println!("{step}: refused ({error})"),
            Err(error) => println!("{step}: {error}"),
        }
    }

    // PostgreSQL itself refuses what the facade never attempts.
    for (step, sql) in [
        (
            "SQL inserts a second substantive CEO holder over the manager's",
            "INSERT INTO people_position_placements(position_id, employee_id, kind, effective_from) \
             SELECT position_id, employee_id, 'substantive', '2025-01-01' FROM people_position_placements WHERE id=$1",
        ),
        (
            "SQL inserts a CEO title version overlapping v2",
            "INSERT INTO people_position_revisions(position_id, version, title, effective_from) \
             SELECT position_id, 9, 'Chief', '2026-06-01' FROM people_position_placements WHERE id=$1",
        ),
        (
            "SQL makes CEO its own parent",
            "UPDATE people_positions SET parent_id = id WHERE id = (SELECT position_id FROM people_position_placements WHERE id=$1)",
        ),
        (
            "SQL gives CEO a parent from Company 002",
            "UPDATE people_positions SET parent_id = (SELECT id FROM people_positions WHERE company_id = 2 LIMIT 1) \
             WHERE id = (SELECT position_id FROM people_position_placements WHERE id=$1)",
        ),
        (
            "SQL sets an assignment's kind to 'temporary'",
            "UPDATE people_position_placements SET kind = 'temporary' WHERE id=$1",
        ),
        (
            "SQL ends an assignment before it started",
            "UPDATE people_position_placements SET effective_to = effective_from - 1 WHERE id=$1",
        ),
        (
            "SQL assigns an employment that does not exist",
            "UPDATE people_position_placements SET employee_id = 9223372036854775807 WHERE id=$1",
        ),
    ] {
        refused(client, step, sql, manager_ceo.id).await;
    }

    let db = services.get::<bilibi_database::Db>("base/database.pool")?;
    // Leave exactly one pool connection free. A position read must use it
    // throughout, including company and holder membership reads in Workforce.
    let status = db.status();
    let borrowed = status.size - status.available;
    let reserve = status.max_size.saturating_sub(borrowed + 1);
    let mut held = Vec::with_capacity(reserve);
    for _ in 0..reserve {
        held.push(db.get().await?);
    }
    let page = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        organisation.positions(manager, company, today, 1, 100),
    )
    .await
    .map_err(|_| Error::Internal)?
    .map_err(|_| Error::Internal)?;
    if page.len() != seen.len() {
        return Err(Error::Internal);
    }
    drop(held);
    println!("position read with only one pool connection free: passed (company and holders use the caller's client)");

    // Hold only a read lock in a separate connection. Revoke the manager's
    // grant after the write queues on that lock, before allowing it to run.
    let mut locker = db.get().await?;
    let lock = locker.transaction().await?;
    let blocker: i32 = lock.query_one("SELECT pg_backend_pid()", &[]).await?.get(0);
    lock.query_one(
        "SELECT id FROM companies WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
        &[&manager.tenant(), &company],
    )
    .await?;
    let facade = organisation.clone();
    let queued_scope = manager.clone();
    let pending = tokio::spawn(async move {
        facade
            .create_position(&queued_scope, company, &NewPosition::new("QUEUED"))
            .await
    });
    let queued = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let waiting: bool = client
                .query_one(
                    "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid)))",
                    &[&blocker],
                )
                .await?
                .get(0);
            if waiting {
                return Ok::<(), Error>(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await;
    if !matches!(queued, Ok(Ok(()))) {
        pending.abort();
        return Err(Error::Internal);
    }
    grants
        .set_direct(admin, manager_user, MANAGE, Some(false))
        .await?;
    lock.rollback().await?;
    let result = pending.await.map_err(|_| Error::Internal)?;
    grants
        .set_direct(admin, manager_user, MANAGE, Some(true))
        .await?;
    if !matches!(result, Err(OrganisationError::Unauthorized)) {
        return Err(Error::Internal);
    }
    println!("manager's queued position write after grant revocation: refused (Access denied); no position created");
    Ok(())
}
