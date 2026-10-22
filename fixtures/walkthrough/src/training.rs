//! The Training catalog block of the walkthrough (slice PT1): the manager,
//! granted the four catalog capabilities directly by the admin, builds a
//! course catalog, an event and sessions through the Training facade, with
//! the refusals beside each write (a viewer without the grant, a taken or
//! malformed code, an inactive course, a capacity above the event's, wall
//! clocks inside a DST gap and overlap, an unknown zone, an end before the
//! start), reads the calendar in the company's own zone (set here to
//! `Asia/Kuala_Lumpur`) with its bounds and its window refusals, shows how
//! `day_start` resolves a midnight gap and a midnight overlap, and ends with
//! PostgreSQL refusing what the facade never attempts.
use crate::{describe, refused, COMPANY, MANAGER, OTHER_COMPANY, OTHER_TENANT_COMPANY};
use bilibi_authz::grants::Grants;
use bilibi_database::{Error, MutationBoundary, Result};
use bilibi_module_registry::Services;
use bilibi_people_training::{
    day_start, CourseUpdate, NewCourse, NewEvent, NewSession, Session, Training, TrainingError,
    CAPABILITIES, COURSES_MANAGE, SESSIONS_MANAGE,
};
use bilibi_settings::{Settings, Target};
use bilibi_tenancy::Scope;
use jiff::civil::date;
use serde_json::json;

/// The zone the walkthrough gives Company 001 (`localization.timezone`).
const ZONE: &str = "Asia/Kuala_Lumpur";

fn show<T: std::fmt::Debug>(
    step: &str,
    result: std::result::Result<T, TrainingError>,
) -> Option<T> {
    match result {
        Ok(value) => {
            println!("{step}: {value:?}");
            Some(value)
        }
        Err(TrainingError::Invalid(errors)) => {
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

fn print_sessions(heading: &str, sessions: &[Session]) {
    println!("{heading}: {} session(s)", sessions.len());
    for session in sessions {
        println!(
            "  #{} {} (event #{}) {} to {} in {}, capacity {}",
            session.id,
            session.name,
            session.event_id,
            session.starts_at,
            session.ends_at,
            session.time_zone,
            session.capacity
        );
    }
}

fn new_session(event_id: i64, name: &str, zone: &str, starts: &str, ends: &str) -> NewSession {
    NewSession {
        event_id,
        name: name.into(),
        capacity: 10,
        time_zone: zone.into(),
        starts_local: starts.into(),
        ends_local: ends.into(),
    }
}

/// Runs the block; `admin` holds every capability, `manager` is the Tenant
/// Owner affiliated with Company 001, `viewer` holds nothing.
pub(crate) async fn run(
    services: &Services,
    client: &tokio_postgres::Client,
    admin: &Scope,
    manager: &Scope,
    viewer: &Scope,
) -> Result<()> {
    println!("\n== Training catalog (PT1) ==");
    let training = services.get::<Training>(bilibi_people_training::SERVICE)?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    let settings = services.get::<Settings>("base/settings.store")?;
    let mutations = services.get::<MutationBoundary>("base/database.mutations")?;
    let db = services.get::<bilibi_database::Db>("base/database.pool")?;

    // Bilimbi grants no Training key to a system role: an administrator does.
    for key in CAPABILITIES {
        grants.set_direct(admin, MANAGER, key, Some(true)).await?;
    }
    println!("admin grants manager@example.test {CAPABILITIES:?}");
    println!(
        "manager may manage sessions in company {COMPANY}: {}; viewer: {}",
        training.allowed(manager, COMPANY, SESSIONS_MANAGE).await,
        training.allowed(viewer, COMPANY, SESSIONS_MANAGE).await
    );

    // Courses, and the refusals a form shows.
    let safety = show(
        "manager adds course safety-101",
        training
            .create_course(
                manager,
                COMPANY,
                &NewCourse {
                    code: "safety-101".into(),
                    name: "  Workplace safety  ".into(),
                    description: Some("Hazards, reporting and first response.".into()),
                },
            )
            .await,
    )
    .ok_or(Error::Internal)?;
    let first_aid = show(
        "manager adds course first-aid without a description",
        training
            .create_course(
                manager,
                COMPANY,
                &NewCourse {
                    code: "first-aid".into(),
                    name: "First aid".into(),
                    description: Some(String::new()),
                },
            )
            .await,
    )
    .ok_or(Error::Internal)?;
    show(
        "manager adds safety-101 again",
        training
            .create_course(
                manager,
                COMPANY,
                &NewCourse {
                    code: "safety-101".into(),
                    name: "Safety again".into(),
                    description: None,
                },
            )
            .await,
    );
    show(
        "manager adds a course with the code 'Bad Code!' and a blank name",
        training
            .create_course(
                manager,
                COMPANY,
                &NewCourse {
                    code: "Bad Code!".into(),
                    name: "   ".into(),
                    description: None,
                },
            )
            .await,
    );
    show(
        "viewer adds a course",
        training
            .create_course(
                viewer,
                COMPANY,
                &NewCourse {
                    code: "viewer".into(),
                    name: "Viewer's course".into(),
                    description: None,
                },
            )
            .await,
    );
    show(
        &format!("manager adds a course in company {OTHER_TENANT_COMPANY} (another tenant's)"),
        training
            .create_course(
                manager,
                OTHER_TENANT_COMPANY,
                &NewCourse {
                    code: "elsewhere".into(),
                    name: "Elsewhere".into(),
                    description: None,
                },
            )
            .await,
    );
    let onboarding = show(
        &format!("manager adds course onboarding in company {OTHER_COMPANY} (tenant-wide reach)"),
        training
            .create_course(
                manager,
                OTHER_COMPANY,
                &NewCourse {
                    code: "onboarding".into(),
                    name: "Onboarding".into(),
                    description: None,
                },
            )
            .await,
    )
    .ok_or(Error::Internal)?;
    show(
        "manager renames safety-101 and sets its description",
        training
            .update_course(
                manager,
                COMPANY,
                safety.id,
                &CourseUpdate {
                    name: Some("Workplace safety essentials".into()),
                    description: Some("Hazards, reporting, first response and drills.".into()),
                    active: None,
                },
            )
            .await,
    );
    show(
        "manager clears safety-101's description with an empty text",
        training
            .update_course(
                manager,
                COMPANY,
                safety.id,
                &CourseUpdate {
                    description: Some(String::new()),
                    ..CourseUpdate::default()
                },
            )
            .await,
    );
    show(
        "manager renames safety-101 to a 161-character name",
        training
            .update_course(
                manager,
                COMPANY,
                safety.id,
                &CourseUpdate {
                    name: Some("x".repeat(161)),
                    ..CourseUpdate::default()
                },
            )
            .await,
    );
    show(
        "manager deactivates first-aid",
        training
            .set_course_active(manager, COMPANY, first_aid.id, false)
            .await,
    );
    show(
        "manager updates course #999",
        training
            .set_course_active(manager, COMPANY, 999, false)
            .await,
    );
    show(
        &format!("manager updates the company {OTHER_COMPANY} course from company {COMPANY}"),
        training
            .set_course_active(manager, COMPANY, onboarding.id, false)
            .await,
    );
    show(
        "viewer deactivates safety-101",
        training
            .set_course_active(viewer, COMPANY, safety.id, false)
            .await,
    );
    if let Some(courses) = show(
        &format!("manager lists company {COMPANY}'s courses"),
        training.courses(manager, COMPANY).await,
    ) {
        for course in &courses {
            println!(
                "  #{} {} {} {} ({})",
                course.id,
                course.code,
                course.name,
                if course.active { "active" } else { "inactive" },
                course.description.as_deref().unwrap_or("no description")
            );
        }
    }
    show(
        "viewer lists courses",
        training.courses(viewer, COMPANY).await,
    );
    show(
        "manager lists the active courses an event may deliver",
        training.event_courses(manager, COMPANY).await,
    );

    // Events.
    let event = show(
        "manager adds event 'Safety cohort A' for safety-101, capacity 12",
        training
            .create_event(
                manager,
                COMPANY,
                &NewEvent {
                    course_id: safety.id,
                    name: "Safety cohort A".into(),
                    capacity: 12,
                },
            )
            .await,
    )
    .ok_or(Error::Internal)?;
    show(
        "manager adds an event for the inactive first-aid course",
        training
            .create_event(
                manager,
                COMPANY,
                &NewEvent {
                    course_id: first_aid.id,
                    name: "First aid cohort".into(),
                    capacity: 8,
                },
            )
            .await,
    );
    show(
        &format!(
            "manager adds an event in company {COMPANY} for the company {OTHER_COMPANY} course"
        ),
        training
            .create_event(
                manager,
                COMPANY,
                &NewEvent {
                    course_id: onboarding.id,
                    name: "Onboarding cohort".into(),
                    capacity: 8,
                },
            )
            .await,
    );
    show(
        "manager adds an event with capacity 0",
        training
            .create_event(
                manager,
                COMPANY,
                &NewEvent {
                    course_id: safety.id,
                    name: "Empty cohort".into(),
                    capacity: 0,
                },
            )
            .await,
    );
    show(
        &format!("manager lists company {COMPANY}'s events"),
        training.events(manager, COMPANY).await,
    );

    // The company's own zone, then sessions entered in it and elsewhere.
    {
        let mut client = db.get().await?;
        let mut tx = mutations
            .begin(
                &mut client,
                admin.mutation_context("people.training.walkthrough.company_zone"),
            )
            .await?;
        settings
            .put(
                &mut tx,
                admin,
                Target::Company(
                    bilibi_party_ids::OrganisationId::new(COMPANY).map_err(|_| Error::Invalid)?,
                ),
                bilibi_datetime::TIMEZONE_KEY,
                json!(ZONE),
            )
            .await?;
        tx.commit().await?;
    }
    show(
        &format!("company {COMPANY}'s zone after the admin sets {ZONE}"),
        training.company_zone(manager, COMPANY).await,
    );
    let morning = show(
        "manager adds session 'Morning run' 2026-11-03 09:00 to 12:00 Asia/Kuala_Lumpur",
        training
            .create_session(
                manager,
                COMPANY,
                &new_session(
                    event.id,
                    "Morning run",
                    ZONE,
                    "2026-11-03T09:00",
                    "2026-11-03T12:00",
                ),
            )
            .await,
    )
    .ok_or(Error::Internal)?;
    show(
        "manager adds session 'Night run' 2026-11-30 22:00 to 2026-12-01 02:00 Asia/Kuala_Lumpur (crosses the month)",
        training
            .create_session(
                manager,
                COMPANY,
                &new_session(
                    event.id,
                    "Night run",
                    ZONE,
                    "2026-11-30T22:00",
                    "2026-12-01T02:00",
                ),
            )
            .await,
    );
    show(
        "manager adds session 'London run' 2026-11-10 09:00:00 to 17:00:00 Europe/London (seconds given)",
        training
            .create_session(
                manager,
                COMPANY,
                &new_session(
                    event.id,
                    "London run",
                    "Europe/London",
                    "2026-11-10T09:00:00",
                    "2026-11-10T17:00:00",
                ),
            )
            .await,
    );
    show(
        "manager adds a session with capacity 20 on an event of 12",
        training
            .create_session(
                manager,
                COMPANY,
                &NewSession {
                    capacity: 20,
                    ..new_session(
                        event.id,
                        "Too many",
                        ZONE,
                        "2026-11-04T09:00",
                        "2026-11-04T12:00",
                    )
                },
            )
            .await,
    );
    show(
        "manager adds a session ending before it starts",
        training
            .create_session(
                manager,
                COMPANY,
                &new_session(
                    event.id,
                    "Backwards",
                    ZONE,
                    "2026-11-04T12:00",
                    "2026-11-04T09:00",
                ),
            )
            .await,
    );
    show(
        "manager adds a session starting 2026-03-29 01:30 Europe/London (inside the spring gap)",
        training
            .create_session(
                manager,
                COMPANY,
                &new_session(
                    event.id,
                    "Gap",
                    "Europe/London",
                    "2026-03-29T01:30",
                    "2026-03-29T04:00",
                ),
            )
            .await,
    );
    show(
        "manager adds a session starting 2026-10-25 01:30 Europe/London (inside the autumn overlap)",
        training
            .create_session(
                manager,
                COMPANY,
                &new_session(
                    event.id,
                    "Overlap",
                    "Europe/London",
                    "2026-10-25T01:30",
                    "2026-10-25T04:00",
                ),
            )
            .await,
    );
    show(
        "manager adds a session in the zone 'Mars/Olympus'",
        training
            .create_session(
                manager,
                COMPANY,
                &new_session(
                    event.id,
                    "Mars",
                    "Mars/Olympus",
                    "2026-11-04T09:00",
                    "2026-11-04T12:00",
                ),
            )
            .await,
    );
    show(
        "manager adds a session with the clock '2026-11-04 nine'",
        training
            .create_session(
                manager,
                COMPANY,
                &new_session(
                    event.id,
                    "Nine",
                    ZONE,
                    "2026-11-04 nine",
                    "2026-11-04T12:00",
                ),
            )
            .await,
    );
    show(
        "manager adds a session for event #999",
        training
            .create_session(
                manager,
                COMPANY,
                &new_session(999, "Nowhere", ZONE, "2026-11-04T09:00", "2026-11-04T12:00"),
            )
            .await,
    );
    show(
        "viewer adds a session",
        training
            .create_session(
                viewer,
                COMPANY,
                &new_session(
                    event.id,
                    "Viewer's",
                    ZONE,
                    "2026-11-04T09:00",
                    "2026-11-04T12:00",
                ),
            )
            .await,
    );

    // The calendar: November 2026 bounded in the company's zone, then December,
    // which still shows the session that crosses midnight, then the refusals.
    let november = date(2026, 11, 1);
    let december = date(2026, 12, 1);
    let january = date(2027, 1, 1);
    if let Some((from, until, zone)) = show(
        "manager asks the company-zone bounds of November 2026",
        training
            .company_day_bounds(manager, COMPANY, november, december)
            .await,
    ) {
        println!("  window [{from}, {until}) in {zone}");
        if let Some(sessions) = show(
            "manager reads the November 2026 calendar",
            training.calendar(manager, COMPANY, from, until).await,
        ) {
            print_sessions("November 2026", &sessions);
        }
        show(
            "manager reads a calendar whose end is its start",
            training.calendar(manager, COMPANY, from, from).await,
        );
        show(
            "manager reads a calendar of 367 days",
            training
                .calendar(
                    manager,
                    COMPANY,
                    from,
                    from.checked_add(jiff::Span::new().hours(367 * 24))
                        .map_err(|_| Error::Internal)?,
                )
                .await,
        );
        if let Some(sessions) = show(
            "manager reads a calendar of exactly 366 days",
            training
                .calendar(
                    manager,
                    COMPANY,
                    from,
                    from.checked_add(jiff::Span::new().hours(366 * 24))
                        .map_err(|_| Error::Internal)?,
                )
                .await,
        ) {
            println!("  {} session(s)", sessions.len());
        }
        show(
            "viewer reads the November 2026 calendar",
            training.calendar(viewer, COMPANY, from, until).await,
        );
    }
    if let Some((from, until, _)) = show(
        "manager asks the company-zone bounds of December 2026",
        training
            .company_day_bounds(manager, COMPANY, december, january)
            .await,
    ) {
        if let Some(sessions) = show(
            "manager reads the December 2026 calendar",
            training.calendar(manager, COMPANY, from, until).await,
        ) {
            print_sessions("December 2026 (the night run crosses into it)", &sessions);
        }
    }

    // Exercise the half-open window with a bound between stored whole seconds.
    let before_start = morning
        .starts_at
        .checked_sub(jiff::Span::new().seconds(1))
        .map_err(|_| Error::Internal)?;
    let after_start = morning
        .starts_at
        .checked_add(jiff::Span::new().nanoseconds(1))
        .map_err(|_| Error::Internal)?;
    let at_start = training
        .calendar(manager, COMPANY, before_start, morning.starts_at)
        .await
        .map_err(|_| Error::Internal)?;
    let just_after_start = training
        .calendar(manager, COMPANY, before_start, after_start)
        .await
        .map_err(|_| Error::Internal)?;
    let at_end = training
        .calendar(
            manager,
            COMPANY,
            morning.ends_at,
            morning
                .ends_at
                .checked_add(jiff::Span::new().seconds(1))
                .map_err(|_| Error::Internal)?,
        )
        .await
        .map_err(|_| Error::Internal)?;
    if !at_start.is_empty()
        || just_after_start.len() != 1
        || just_after_start[0].id != morning.id
        || !at_end.is_empty()
    {
        return Err(Error::Internal);
    }
    println!("Calendar half-open edges: excludes a session at until=start and from=end; includes it at until=start+1ns.");
    let over_limit = morning
        .starts_at
        .checked_add(jiff::Span::new().hours(366 * 24).nanoseconds(1))
        .map_err(|_| Error::Internal)?;
    if !matches!(
        training
            .calendar(manager, COMPANY, morning.starts_at, over_limit)
            .await,
        Err(TrainingError::InvalidCalendarRange)
    ) {
        return Err(Error::Internal);
    }
    println!("Calendar window of 366 days + 1ns: refused.");

    // The same sealed scope loses authority as soon as its grant is revoked.
    grants
        .set_direct(admin, MANAGER, COURSES_MANAGE, Some(false))
        .await?;
    let revoked = training
        .set_course_active(manager, COMPANY, safety.id, false)
        .await;
    grants
        .set_direct(admin, MANAGER, COURSES_MANAGE, Some(true))
        .await?;
    if !matches!(revoked, Err(TrainingError::Unauthorized)) {
        return Err(Error::Internal);
    }
    println!("Course write after revoking the manager's grant: refused; grant restored.");

    // Day bounds across a clock change at local midnight.
    describe(
        "day_start 2026-09-06 America/Santiago (clocks jump 00:00 to 01:00: the first instant after the gap)",
        day_start(date(2026, 9, 6), "America/Santiago"),
    );
    describe(
        "day_start 2026-11-01 America/Havana (clocks fall back 01:00 to 00:00: the earlier midnight)",
        day_start(date(2026, 11, 1), "America/Havana"),
    );
    describe(
        "day_start 2026-11-01 Asia/Kuala_Lumpur",
        day_start(date(2026, 11, 1), ZONE),
    );
    describe(
        "day_start 2026-11-01 Mars/Olympus",
        day_start(date(2026, 11, 1), "Mars/Olympus"),
    );

    // PostgreSQL itself refuses what the facade never attempts.
    for (step, sql) in [
        (
            "SQL shrinks the event's capacity below its sessions'",
            "UPDATE people_training_events SET capacity = 5 WHERE id = $1",
        ),
        (
            "SQL inserts a session with capacity 50 on the event of 12",
            "INSERT INTO people_training_session_runs (tenant_id, company_id, event_id, name, capacity, time_zone, starts_at, ends_at, actor_user_id) \
             SELECT tenant_id, company_id, id, 'Oversized', 50, 'UTC', '2026-11-05 01:00', '2026-11-05 04:00', actor_user_id FROM people_training_events WHERE id = $1",
        ),
        (
            "SQL stores the zone 'Mars/Olympus' on a session",
            "UPDATE people_training_session_runs SET time_zone = 'Mars/Olympus' WHERE event_id = $1",
        ),
        (
            "SQL ends a session when it starts",
            "UPDATE people_training_session_runs SET ends_at = starts_at WHERE event_id = $1",
        ),
        (
            "SQL sets a session's capacity to 0",
            "UPDATE people_training_session_runs SET capacity = 0 WHERE event_id = $1",
        ),
        (
            "SQL inserts a company 001 event for the company 002 course",
            "INSERT INTO people_training_events (tenant_id, company_id, course_id, name, capacity, actor_user_id) \
             SELECT tenant_id, company_id, (SELECT id FROM people_training_courses WHERE code = 'onboarding'), 'Cross-company', 5, actor_user_id FROM people_training_events WHERE id = $1",
        ),
        (
            "SQL gives first-aid the code safety-101",
            "UPDATE people_training_courses SET code = 'safety-101' WHERE code = 'first-aid' AND company_id = (SELECT company_id FROM people_training_events WHERE id = $1)",
        ),
    ] {
        refused(client, step, sql, event.id).await;
    }
    Ok(())
}
