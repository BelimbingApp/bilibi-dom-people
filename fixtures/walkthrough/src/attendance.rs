//! Attendance core proof on the database prepared by the default walkthrough.
use super::*;
use rust_decimal::Decimal as AttendanceDecimal;

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

pub(super) fn decimal(text: &str) -> AttendanceDecimal {
    text.parse().expect("a literal decimal")
}

/// The attendance core walkthrough; `url` is the database `run` prepared.
async fn run(url: &str) -> Result<()> {
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

    // PW1 deliberately leaves the account unlinked. Restore its Person
    // and affiliation through Core User before proving Attendance reads.
    let employment = workforce
        .employment(&manager, COMPANY, staff_employee)
        .await
        .map_err(|_| Error::Internal)?
        .require_current()
        .map_err(|_| Error::Internal)?;
    let users = services.get::<UserService>("core/user.accounts")?;
    users
        .link_person(
            &admin,
            STAFF,
            PersonId::new(employment.person_id).map_err(|_| Error::Invalid)?,
        )
        .await
        .map_err(|_| Error::Internal)?;
    users
        .link_affiliation(
            &admin,
            STAFF,
            bilibi_affiliation::AffiliationId::new(employment.affiliation_id)
                .map_err(|_| Error::Invalid)?,
        )
        .await
        .map_err(|_| Error::Internal)?;
    let staff = authentication
        .authenticate(&**client, TENANT, STAFF, "walkthrough".into())
        .await?;

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

    // An open shift older than the maximum shift length reads as an
    // exception while its row stays in progress.
    let thirteen_hours_ago = Timestamp::now()
        .checked_sub(jiff::Span::new().hours(13))
        .map_err(|_| Error::Internal)?;
    attendance_step(
        "device clocks the manager in 13 hours ago, key m1",
        attendance
            .record_clock(
                &manager,
                COMPANY,
                manager_employee,
                &ClockInput::new("m1", EventType::In, "device", thirteen_hours_ago),
            )
            .await,
    );
    if let Some(days) = attendance_step(
        &format!("manager lists employee #{manager_employee}'s days"),
        attendance
            .list_days(&manager, COMPANY, manager_employee)
            .await,
    ) {
        print_days("the manager's days", &days);
    }

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

    attendance_locks::run(&db, &services, &admin, &manager, &staff, staff_employee).await?;

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

walkthrough_command!("attendance", |url, _argument| async move { run(url).await });
