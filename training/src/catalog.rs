//! The catalog tables (`people_training_courses`, `people_training_events`,
//! `people_training_session_runs`): their rows, the SQL that reads them, the
//! validation in Bilimbi's changeset words and the writes behind the facade.
//! Private to the facade.
use crate::{
    clock, Course, CourseOption, CourseUpdate, Event, FieldError, NewCourse, NewEvent, NewSession,
    Result, Session, TrainingError, CALENDAR_LIMIT,
};
use bilibi_database::{Change, Error, MutationTransaction};
use jiff::Timestamp;
use serde_json::{json, Value};
use tokio_postgres::{GenericClient, Row};

/// Who writes: the sealed scope's user and, while impersonating, the operator.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Actor {
    pub user: i64,
    pub impersonator: Option<i64>,
}

/// The widest capacity the `integer` columns hold (Bilimbi's upper bound).
const CAPACITY_MAX: i64 = 2_147_483_647;

const COURSE_COLUMNS: &str =
    "c.id, c.company_id, c.code, c.name, c.description, c.active, c.actor_user_id, c.impersonator_id";
const EVENT_COLUMNS: &str =
    "e.id, e.company_id, e.course_id, c.name AS course_name, e.name, e.capacity, \
    e.actor_user_id, e.impersonator_id";
const EVENT_FROM: &str = "FROM people_training_events e \
    JOIN people_training_courses c ON c.id = e.course_id AND c.tenant_id = e.tenant_id AND c.company_id = e.company_id";
const SESSION_COLUMNS: &str = "s.id, s.company_id, s.event_id, s.name, s.capacity, s.time_zone, \
    to_char(s.starts_at, 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS starts_at, \
    to_char(s.ends_at, 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS ends_at, \
    s.actor_user_id, s.impersonator_id";

fn course(row: &Row) -> Course {
    Course {
        id: row.get("id"),
        platform_company_id: row.get("company_id"),
        code: row.get("code"),
        name: row.get("name"),
        description: row.get("description"),
        active: row.get("active"),
        actor_user_id: row.get("actor_user_id"),
        impersonator_id: row.get("impersonator_id"),
    }
}

fn event(row: &Row) -> Event {
    Event {
        id: row.get("id"),
        platform_company_id: row.get("company_id"),
        course_id: row.get("course_id"),
        course_name: row.get("course_name"),
        name: row.get("name"),
        capacity: row.get("capacity"),
        actor_user_id: row.get("actor_user_id"),
        impersonator_id: row.get("impersonator_id"),
    }
}

fn instant(row: &Row, column: &str) -> std::result::Result<Timestamp, Error> {
    clock::from_sql_text(row.get::<_, String>(column).as_str()).ok_or(Error::Internal)
}

fn session(row: &Row) -> std::result::Result<Session, Error> {
    Ok(Session {
        id: row.get("id"),
        platform_company_id: row.get("company_id"),
        event_id: row.get("event_id"),
        name: row.get("name"),
        capacity: row.get("capacity"),
        time_zone: row.get("time_zone"),
        starts_at: instant(row, "starts_at")?,
        ends_at: instant(row, "ends_at")?,
        actor_user_id: row.get("actor_user_id"),
        impersonator_id: row.get("impersonator_id"),
    })
}

// ---- reads -----------------------------------------------------------------

/// The company's courses, active or not, by name then id.
pub(crate) async fn courses(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
) -> Result<Vec<Course>> {
    Ok(client
        .query(
            &format!("SELECT {COURSE_COLUMNS} FROM people_training_courses c WHERE c.tenant_id = $1 AND c.company_id = $2 ORDER BY c.name, c.id"),
            &[&tenant, &company_id],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(course)
        .collect())
}

/// The company's active courses as `(id, name)` choices, by name then id.
pub(crate) async fn active_courses(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
) -> Result<Vec<CourseOption>> {
    Ok(client
        .query(
            "SELECT id, name FROM people_training_courses WHERE tenant_id = $1 AND company_id = $2 AND active ORDER BY name, id",
            &[&tenant, &company_id],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(|row| CourseOption {
            id: row.get("id"),
            name: row.get("name"),
        })
        .collect())
}

/// One course of the company, or `None`; `lock` takes the row `FOR UPDATE`.
async fn find_course(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    id: i64,
    lock: bool,
) -> Result<Option<Course>> {
    let lock = if lock { " FOR UPDATE" } else { "" };
    Ok(client
        .query_opt(
            &format!("SELECT {COURSE_COLUMNS} FROM people_training_courses c WHERE c.tenant_id = $1 AND c.company_id = $2 AND c.id = $3{lock}"),
            &[&tenant, &company_id, &id],
        )
        .await
        .map_err(Error::from)?
        .as_ref()
        .map(course))
}

/// The company's events with their course names, by event name then id.
pub(crate) async fn events(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
) -> Result<Vec<Event>> {
    Ok(client
        .query(
            &format!("SELECT {EVENT_COLUMNS} {EVENT_FROM} WHERE e.tenant_id = $1 AND e.company_id = $2 ORDER BY e.name, e.id"),
            &[&tenant, &company_id],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(event)
        .collect())
}

/// One event of the company, or `None`; `lock` takes the row `FOR UPDATE`
/// so a capacity judged holds until the writing transaction commits.
async fn find_event(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    id: i64,
    lock: bool,
) -> Result<Option<Event>> {
    let lock = if lock { " FOR UPDATE OF e" } else { "" };
    Ok(client
        .query_opt(
            &format!("SELECT {EVENT_COLUMNS} {EVENT_FROM} WHERE e.tenant_id = $1 AND e.company_id = $2 AND e.id = $3{lock}"),
            &[&tenant, &company_id, &id],
        )
        .await
        .map_err(Error::from)?
        .as_ref()
        .map(event))
}

/// The sessions overlapping the half-open UTC window (`starts_at < until`
/// and `ends_at > from`), by instant then id, at most [`CALENDAR_LIMIT`].
pub(crate) async fn calendar(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    from: Timestamp,
    until: Timestamp,
) -> Result<Vec<Session>> {
    let limit = i64::try_from(CALENDAR_LIMIT).map_err(|_| Error::Internal)?;
    // Stored sessions have whole seconds. Floor the lower bound and ceil the
    // upper bound without letting PostgreSQL round nanoseconds to microseconds.
    // The ceiling happens in SQL so Timestamp::MAX remains a valid window end.
    let until_fractional = until.as_nanosecond().rem_euclid(1_000_000_000) != 0;
    client
        .query(
            &format!(
                "SELECT {SESSION_COLUMNS} FROM people_training_session_runs s \
                WHERE s.tenant_id = $1 AND s.company_id = $2 \
                  AND s.starts_at < ($4::text::timestamp + CASE WHEN $6 THEN interval '1 second' ELSE interval '0 seconds' END) \
                  AND s.ends_at > $3::text::timestamp \
                ORDER BY s.starts_at, s.id LIMIT $5"
            ),
            &[
                &tenant,
                &company_id,
                &clock::sql_text(clock::whole_second(from)),
                &clock::sql_text(clock::whole_second(until)),
                &limit,
                &until_fractional,
            ],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(|row| session(row).map_err(TrainingError::Failed))
        .collect()
}

// ---- validation (Bilimbi changeset words) ----------------------------------

pub(crate) struct ValidCourse {
    pub code: String,
    pub name: String,
    pub description: Option<String>,
}

/// `^[a-z0-9][a-z0-9_-]*$`, at most 80 characters.
fn code_valid(code: &str) -> bool {
    let mut chars = code.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn name_errors(name: &str, errors: &mut Vec<FieldError>) -> String {
    let name = name.trim().to_owned();
    if name.is_empty() {
        errors.push(FieldError::new("name", "can't be blank"));
    } else if name.chars().count() > 160 {
        errors.push(FieldError::new(
            "name",
            "should be at most 160 character(s)",
        ));
    }
    name
}

/// Ecto casts an empty text to `nil`; a description longer than 4,000
/// characters is refused.
fn description_errors(description: Option<&str>, errors: &mut Vec<FieldError>) -> Option<String> {
    let description = description.filter(|text| !text.is_empty())?;
    if description.chars().count() > 4000 {
        errors.push(FieldError::new(
            "description",
            "should be at most 4000 character(s)",
        ));
    }
    Some(description.to_owned())
}

fn capacity_errors(capacity: i64, errors: &mut Vec<FieldError>) -> i32 {
    if capacity <= 0 {
        errors.push(FieldError::new("capacity", "must be greater than 0"));
    } else if capacity > CAPACITY_MAX {
        errors.push(FieldError::new(
            "capacity",
            "must be less than or equal to 2147483647",
        ));
    }
    i32::try_from(capacity).unwrap_or(0)
}

pub(crate) fn validate_course(new: &NewCourse) -> Result<ValidCourse> {
    let mut errors = Vec::new();
    let code = new.code.clone();
    if code.is_empty() {
        errors.push(FieldError::new("code", "can't be blank"));
    } else {
        if !code_valid(&code) {
            errors.push(FieldError::new("code", "has invalid format"));
        }
        if code.chars().count() > 80 {
            errors.push(FieldError::new("code", "should be at most 80 character(s)"));
        }
    }
    let name = name_errors(&new.name, &mut errors);
    let description = description_errors(new.description.as_deref(), &mut errors);
    if errors.is_empty() {
        Ok(ValidCourse {
            code,
            name,
            description,
        })
    } else {
        Err(TrainingError::Invalid(errors))
    }
}

/// The course as it would be after the change, validated as a whole.
pub(crate) fn validate_course_update(
    current: &Course,
    update: &CourseUpdate,
) -> Result<ValidCourse> {
    let mut errors = Vec::new();
    let name = match &update.name {
        Some(name) => name_errors(name, &mut errors),
        None => current.name.clone(),
    };
    let description = match &update.description {
        Some(description) => description_errors(Some(description), &mut errors),
        None => current.description.clone(),
    };
    if errors.is_empty() {
        Ok(ValidCourse {
            code: current.code.clone(),
            name,
            description,
        })
    } else {
        Err(TrainingError::Invalid(errors))
    }
}

pub(crate) struct ValidEvent {
    pub course_id: i64,
    pub name: String,
    pub capacity: i32,
}

pub(crate) fn validate_event(new: &NewEvent) -> Result<ValidEvent> {
    let mut errors = Vec::new();
    let name = name_errors(&new.name, &mut errors);
    let capacity = capacity_errors(new.capacity, &mut errors);
    if errors.is_empty() {
        Ok(ValidEvent {
            course_id: new.course_id,
            name,
            capacity,
        })
    } else {
        Err(TrainingError::Invalid(errors))
    }
}

pub(crate) struct ValidSession {
    pub event_id: i64,
    pub name: String,
    pub capacity: i32,
    pub time_zone: String,
    pub starts_at: Timestamp,
    pub ends_at: Timestamp,
}

/// Bilimbi resolves the instants first (zone and clock refusals), then the
/// time order, then the changeset.
pub(crate) fn validate_session(new: &NewSession) -> Result<ValidSession> {
    let starts_at = clock::local_instant(&new.starts_local, &new.time_zone)?;
    let ends_at = clock::local_instant(&new.ends_local, &new.time_zone)?;
    if ends_at <= starts_at {
        return Err(TrainingError::InvalidTimeRange);
    }
    let mut errors = Vec::new();
    let name = name_errors(&new.name, &mut errors);
    let capacity = capacity_errors(new.capacity, &mut errors);
    if errors.is_empty() {
        Ok(ValidSession {
            event_id: new.event_id,
            name,
            capacity,
            time_zone: new.time_zone.clone(),
            starts_at,
            ends_at,
        })
    } else {
        Err(TrainingError::Invalid(errors))
    }
}

// ---- writes ----------------------------------------------------------------

/// A code already used by another course of the company (`unique_constraint`).
async fn code_taken(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    code: &str,
    except: Option<i64>,
) -> Result<bool> {
    let except = except.unwrap_or(0);
    Ok(client
        .query_opt(
            "SELECT 1 FROM people_training_courses WHERE company_id = $1 AND code = $2 AND id <> $3 AND tenant_id = $4",
            &[&company_id, &code, &except, &tenant],
        )
        .await
        .map_err(Error::from)?
        .is_some())
}

fn course_values(course: &Course) -> Value {
    json!({
        "company_id": course.platform_company_id,
        "code": course.code,
        "name": course.name,
        "description": course.description,
        "active": course.active,
        "actor_user_id": course.actor_user_id,
        "impersonator_id": course.impersonator_id,
    })
}

pub(crate) async fn insert_course(
    tx: &mut MutationTransaction<'_>,
    tenant: i64,
    company_id: i64,
    actor: Actor,
    valid: ValidCourse,
) -> Result<Course> {
    if code_taken(tx.sql(), tenant, company_id, &valid.code, None).await? {
        return Err(TrainingError::Invalid(vec![FieldError::new(
            "code",
            "has already been taken",
        )]));
    }
    tx.mutate(move |sql| {
        Box::pin(async move {
            let row = sql
                .query_one(
                    &format!("INSERT INTO people_training_courses AS c \
                        (tenant_id, company_id, code, name, description, active, actor_user_id, impersonator_id) \
                        VALUES ($1, $2, $3, $4, $5, true, $6, $7) RETURNING {COURSE_COLUMNS}"),
                    &[
                        &tenant,
                        &company_id,
                        &valid.code,
                        &valid.name,
                        &valid.description,
                        &actor.user,
                        &actor.impersonator,
                    ],
                )
                .await?;
            let created = course(&row);
            // An empty object on the absent side is what names the audit
            // event `created`.
            let change = Change {
                kind: "people_training_course".into(),
                id: created.id,
                old: json!({}),
                new: course_values(&created),
            };
            Ok((created, vec![change]))
        })
    })
    .await
    .map_err(TrainingError::from)
}

/// Finds the course of the company for a change, locked; missing is `NotFound`.
pub(crate) async fn lock_course(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    id: i64,
) -> Result<Course> {
    if id <= 0 {
        return Err(TrainingError::NotFound);
    }
    find_course(client, tenant, company_id, id, true)
        .await?
        .ok_or(TrainingError::NotFound)
}

/// Writes the course's name, description and active flag, authored by the
/// actor, as an `updated` change carrying both sides.
pub(crate) async fn update_course(
    tx: &mut MutationTransaction<'_>,
    tenant: i64,
    current: Course,
    actor: Actor,
    valid: ValidCourse,
    active: bool,
) -> Result<Course> {
    tx.mutate(move |sql| {
        Box::pin(async move {
            let row = sql
                .query_one(
                    &format!("UPDATE people_training_courses AS c \
                        SET name = $2, description = $3, active = $4, actor_user_id = $5, impersonator_id = $6, \
                            updated_at = (now() AT TIME ZONE 'UTC') \
                        WHERE c.id = $1 AND c.tenant_id = $7 AND c.company_id = $8 RETURNING {COURSE_COLUMNS}"),
                    &[
                        &current.id,
                        &valid.name,
                        &valid.description,
                        &active,
                        &actor.user,
                        &actor.impersonator,
                        &tenant,
                        &current.platform_company_id,
                    ],
                )
                .await?;
            let updated = course(&row);
            let change = Change {
                kind: "people_training_course".into(),
                id: updated.id,
                old: course_values(&current),
                new: course_values(&updated),
            };
            Ok((updated, vec![change]))
        })
    })
    .await
    .map_err(TrainingError::from)
}

/// Inserts an event for an active course of the company, locked for the
/// transaction; a missing or inactive course is `CourseUnavailable`.
pub(crate) async fn insert_event(
    tx: &mut MutationTransaction<'_>,
    tenant: i64,
    company_id: i64,
    actor: Actor,
    valid: ValidEvent,
) -> Result<Event> {
    let course = if valid.course_id > 0 {
        find_course(tx.sql(), tenant, company_id, valid.course_id, true).await?
    } else {
        None
    };
    if !course.is_some_and(|course| course.active) {
        return Err(TrainingError::CourseUnavailable);
    }
    tx.mutate(move |sql| {
        Box::pin(async move {
            let row = sql
                .query_one(
                    "INSERT INTO people_training_events \
                        (tenant_id, company_id, course_id, name, capacity, actor_user_id, impersonator_id) \
                        VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id",
                    &[
                        &tenant,
                        &company_id,
                        &valid.course_id,
                        &valid.name,
                        &valid.capacity,
                        &actor.user,
                        &actor.impersonator,
                    ],
                )
                .await?;
            let id: i64 = row.get(0);
            let row = sql
                .query_one(
                    &format!("SELECT {EVENT_COLUMNS} {EVENT_FROM} WHERE e.id = $1 AND e.tenant_id = $2 AND e.company_id = $3"),
                    &[&id, &tenant, &company_id],
                )
                .await?;
            let created = event(&row);
            let change = Change {
                kind: "people_training_event".into(),
                id,
                old: json!({}),
                new: json!({
                    "company_id": created.platform_company_id,
                    "course_id": created.course_id,
                    "name": created.name,
                    "capacity": created.capacity,
                    "actor_user_id": created.actor_user_id,
                    "impersonator_id": created.impersonator_id,
                }),
            };
            Ok((created, vec![change]))
        })
    })
    .await
    .map_err(TrainingError::from)
}

/// Inserts a session of an event of the company, locked for the transaction
/// (`EventUnavailable` when missing); a capacity above the event's is
/// `CapacityExceeded` before the trigger would refuse it.
pub(crate) async fn insert_session(
    tx: &mut MutationTransaction<'_>,
    tenant: i64,
    company_id: i64,
    actor: Actor,
    valid: ValidSession,
) -> Result<Session> {
    let event = if valid.event_id > 0 {
        find_event(tx.sql(), tenant, company_id, valid.event_id, true).await?
    } else {
        None
    };
    let Some(event) = event else {
        return Err(TrainingError::EventUnavailable);
    };
    if valid.capacity > event.capacity {
        return Err(TrainingError::CapacityExceeded);
    }
    tx.mutate(move |sql| {
        Box::pin(async move {
            let row = sql
                .query_one(
                    &format!("INSERT INTO people_training_session_runs AS s \
                        (tenant_id, company_id, event_id, name, capacity, time_zone, starts_at, ends_at, actor_user_id, impersonator_id) \
                        VALUES ($1, $2, $3, $4, $5, $6, $7::text::timestamp, $8::text::timestamp, $9, $10) \
                        RETURNING {SESSION_COLUMNS}"),
                    &[
                        &tenant,
                        &company_id,
                        &valid.event_id,
                        &valid.name,
                        &valid.capacity,
                        &valid.time_zone,
                        &clock::sql_text(valid.starts_at),
                        &clock::sql_text(valid.ends_at),
                        &actor.user,
                        &actor.impersonator,
                    ],
                )
                .await?;
            let created = session(&row)?;
            let change = Change {
                kind: "people_training_session_run".into(),
                id: created.id,
                old: json!({}),
                new: json!({
                    "company_id": created.platform_company_id,
                    "event_id": created.event_id,
                    "name": created.name,
                    "capacity": created.capacity,
                    "time_zone": created.time_zone,
                    "starts_at": created.starts_at.to_string(),
                    "ends_at": created.ends_at.to_string(),
                    "actor_user_id": created.actor_user_id,
                    "impersonator_id": created.impersonator_id,
                }),
            };
            Ok((created, vec![change]))
        })
    })
    .await
    .map_err(TrainingError::from)
}
