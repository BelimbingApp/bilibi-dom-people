//! Clock facts and the day projection (Bilimbi `record_clock/4`,
//! `list_days/3` and the private `write_event`, `shift_day`, `project_day`
//! and `day_view` behind them): the `people_attendance_clock_facts` and
//! `people_attendance_daily_summaries` tables. Private to the facade.
//!
//! Instants are bound and read as RFC 3339 text (`$n::text::timestamptz`,
//! `to_char`) and dates as ISO text: the workspace's `tokio-postgres` has no
//! date or time feature, and a Domain must not turn one on.
use crate::{AttendanceError, Result};
use bilibi_database::{Change, Error, MutationTransaction};
use bilibi_tenancy::Scope;
use jiff::{civil::Date, tz::TimeZone, Span, Timestamp};
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::{json, Value};
use tokio_postgres::{GenericClient, Row};

/// How many recent days a self-service read shows (Bilimbi's `limit: 31`).
pub const DAYS_SHOWN: i64 = 31;
/// Bilimbi's bounds on an event key and source.
const KEY_BYTES: std::ops::RangeInclusive<usize> = 1..=160;
const SOURCE_BYTES: std::ops::RangeInclusive<usize> = 1..=32;

/// What a clock fact records.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    In,
    Out,
    BreakIn,
    BreakOut,
}
impl EventType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::In => "in",
            Self::Out => "out",
            Self::BreakIn => "break_in",
            Self::BreakOut => "break_out",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "in" => Some(Self::In),
            "out" => Some(Self::Out),
            "break_in" => Some(Self::BreakIn),
            "break_out" => Some(Self::BreakOut),
            _ => None,
        }
    }
}

/// Where a projected day stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DayStatus {
    /// A clock-in with no clock-out yet, within the maximum shift length.
    InProgress,
    /// No clock-in, a clock-out before the clock-in or past the maximum
    /// length, or an open shift older than that length: a supervisor's.
    ExceptionPending,
    /// A clock-in and a later clock-out within the maximum length.
    ReadyForReview,
}
impl DayStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InProgress => "in_progress",
            Self::ExceptionPending => "exception_pending",
            Self::ReadyForReview => "ready_for_review",
        }
    }
    fn parse(value: &str) -> Option<Self> {
        match value {
            "in_progress" => Some(Self::InProgress),
            "exception_pending" => Some(Self::ExceptionPending),
            "ready_for_review" => Some(Self::ReadyForReview),
            _ => None,
        }
    }
}

/// What records a clock fact: the source's idempotent key (1 to 160
/// bytes), the type, the source (1 to 32 bytes), the instant, and optional
/// coordinates (both or neither).
#[derive(Clone, Debug)]
pub struct ClockInput {
    pub event_key: String,
    pub event_type: EventType,
    pub source: String,
    pub occurred_at: Timestamp,
    pub latitude: Option<Decimal>,
    pub longitude: Option<Decimal>,
}
impl ClockInput {
    pub fn new(
        event_key: impl Into<String>,
        event_type: EventType,
        source: impl Into<String>,
        occurred_at: Timestamp,
    ) -> Self {
        Self {
            event_key: event_key.into(),
            event_type,
            source: source.into(),
            occurred_at,
            latitude: None,
            longitude: None,
        }
    }
}

/// A recorded clock fact, as Bilimbi's `event_view/1` shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ClockEvent {
    pub id: i64,
    pub event_type: EventType,
    #[serde(serialize_with = "as_text")]
    pub occurred_at: Timestamp,
    pub timezone: String,
    pub source: String,
}

/// One projected day of one employee (Bilimbi's `day_view/3`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Day {
    #[serde(serialize_with = "as_text")]
    pub on_date: Date,
    pub status: DayStatus,
    #[serde(serialize_with = "as_optional_text")]
    pub first_in_at: Option<Timestamp>,
    #[serde(serialize_with = "as_optional_text")]
    pub last_out_at: Option<Timestamp>,
    pub worked_minutes: i64,
}
/// Instants and dates serialize as their RFC 3339 and ISO text: the
/// workspace's `jiff` carries no serde feature, and a Domain must not turn
/// one on.
fn as_text<T: std::fmt::Display, S: serde::Serializer>(
    value: &T,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.collect_str(value)
}
fn as_optional_text<T: std::fmt::Display, S: serde::Serializer>(
    value: &Option<T>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    match value {
        Some(value) => serializer.collect_str(value),
        None => serializer.serialize_none(),
    }
}

/// A validated event, with its local date in the company's time zone.
pub(crate) struct Event {
    pub(crate) key: String,
    pub(crate) event_type: EventType,
    pub(crate) source: String,
    pub(crate) occurred_at: Timestamp,
    pub(crate) timezone: String,
    point: Option<(Decimal, Decimal)>,
    on_date: Date,
}
impl Event {
    pub(crate) fn point(&self) -> Option<(Decimal, Decimal)> {
        self.point
    }
}

/// Bilimbi `normalize_event/2`: bounded key and source, coordinates both
/// or neither within ±90 and ±180 kept to six decimals, the instant to the
/// second, and the local date from the company's time zone.
pub(crate) fn normalize(input: &ClockInput, timezone: &str) -> Result<Event> {
    if !KEY_BYTES.contains(&input.event_key.len()) || !SOURCE_BYTES.contains(&input.source.len()) {
        return Err(AttendanceError::InvalidEvent);
    }
    let point = match (input.latitude, input.longitude) {
        (None, None) => None,
        (Some(latitude), Some(longitude)) => {
            Some((coordinate(latitude, 90)?, coordinate(longitude, 180)?))
        }
        _ => return Err(AttendanceError::InvalidEvent),
    };
    let occurred_at = Timestamp::from_second(input.occurred_at.as_second())
        .map_err(|_| AttendanceError::InvalidEvent)?;
    Ok(Event {
        key: input.event_key.clone(),
        event_type: input.event_type,
        source: input.source.clone(),
        occurred_at,
        timezone: timezone.to_owned(),
        point,
        on_date: local_date(occurred_at, timezone)?,
    })
}

fn coordinate(value: Decimal, bound: i64) -> Result<Decimal> {
    if value.abs() > Decimal::from(bound) {
        return Err(AttendanceError::InvalidEvent);
    }
    Ok(value.round_dp(6))
}

/// The date of an instant in an IANA time zone; a zone the database does
/// not know is a bad rule, not a bad event.
pub(crate) fn local_date(at: Timestamp, timezone: &str) -> Result<Date> {
    let zone = TimeZone::get(timezone).map_err(|_| AttendanceError::InvalidRules)?;
    Ok(at.to_zoned(zone).date())
}

/// A day row.
#[derive(Clone, Debug)]
struct DayRow {
    id: i64,
    company_id: i64,
    employee_id: i64,
    on_date: Date,
    status: DayStatus,
    first_in_at: Option<Timestamp>,
    last_out_at: Option<Timestamp>,
    worked_minutes: i32,
}
impl DayRow {
    fn open_shift(&self) -> bool {
        self.status == DayStatus::InProgress && self.first_in_at.is_some()
    }
    fn audit_values(&self) -> Value {
        json!({
            "company_id": self.company_id,
            "employee_id": self.employee_id,
            "on_date": self.on_date.to_string(),
            "status": self.status.as_str(),
            "first_in_at": self.first_in_at.map(|at| at.to_string()),
            "last_out_at": self.last_out_at.map(|at| at.to_string()),
            "worked_minutes": self.worked_minutes,
        })
    }
    fn view(&self, max_shift_hours: i64, now: Timestamp) -> Day {
        let status = match self.first_in_at {
            Some(first_in)
                if self.open_shift() && seconds_between(first_in, now) > max_shift_hours * 3600 =>
            {
                DayStatus::ExceptionPending
            }
            _ => self.status,
        };
        Day {
            on_date: self.on_date,
            status,
            first_in_at: self.first_in_at,
            last_out_at: self.last_out_at,
            worked_minutes: i64::from(self.worked_minutes),
        }
    }
}

/// A clock fact row.
#[derive(Clone, Debug)]
struct EventRow {
    id: i64,
    company_id: i64,
    employee_id: i64,
    day_id: i64,
    event_key: String,
    event_type: EventType,
    source: String,
    occurred_at: Timestamp,
    timezone: String,
    actor_user_id: Option<i64>,
    latitude: Option<Decimal>,
    longitude: Option<Decimal>,
    clocking_location_id: Option<i64>,
}
impl EventRow {
    fn audit_values(&self) -> Value {
        json!({
            "company_id": self.company_id,
            "employee_id": self.employee_id,
            "day_id": self.day_id,
            "event_key": self.event_key,
            "event_type": self.event_type.as_str(),
            "source": self.source,
            "occurred_at": self.occurred_at.to_string(),
            "timezone": self.timezone,
            "actor_user_id": self.actor_user_id,
            "latitude": self.latitude.map(|value| value.to_string()),
            "longitude": self.longitude.map(|value| value.to_string()),
            "clocking_location_id": self.clocking_location_id,
        })
    }
    fn view(&self) -> ClockEvent {
        ClockEvent {
            id: self.id,
            event_type: self.event_type,
            occurred_at: self.occurred_at,
            timezone: self.timezone.clone(),
            source: self.source.clone(),
        }
    }
}

const DAY_COLUMNS: &str = "id, company_id, employee_id, to_char(on_date, 'YYYY-MM-DD') AS on_date, status, \
    to_char(first_in_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS first_in_at, \
    to_char(last_out_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS last_out_at, worked_minutes";
const EVENT_COLUMNS: &str = "id, company_id, employee_id, day_id, event_key, event_type, source, \
    to_char(occurred_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS occurred_at, \
    timezone, actor_user_id, latitude, longitude, clocking_location_id";

fn timestamp(row: &Row, column: &str) -> std::result::Result<Option<Timestamp>, Error> {
    row.get::<_, Option<String>>(column)
        .map(|text| text.parse::<Timestamp>().map_err(|_| Error::Internal))
        .transpose()
}

fn read_day(row: &Row) -> std::result::Result<DayRow, Error> {
    Ok(DayRow {
        id: row.get("id"),
        company_id: row.get("company_id"),
        employee_id: row.get("employee_id"),
        on_date: row
            .get::<_, String>("on_date")
            .parse()
            .map_err(|_| Error::Internal)?,
        status: DayStatus::parse(row.get("status")).ok_or(Error::Internal)?,
        first_in_at: timestamp(row, "first_in_at")?,
        last_out_at: timestamp(row, "last_out_at")?,
        worked_minutes: row.get("worked_minutes"),
    })
}

fn read_event(row: &Row) -> std::result::Result<EventRow, Error> {
    Ok(EventRow {
        id: row.get("id"),
        company_id: row.get("company_id"),
        employee_id: row.get("employee_id"),
        day_id: row.get("day_id"),
        event_key: row.get("event_key"),
        event_type: EventType::parse(row.get("event_type")).ok_or(Error::Internal)?,
        source: row.get("source"),
        occurred_at: timestamp(row, "occurred_at")?.ok_or(Error::Internal)?,
        timezone: row.get("timezone"),
        actor_user_id: row.get("actor_user_id"),
        latitude: row.get("latitude"),
        longitude: row.get("longitude"),
        clocking_location_id: row.get("clocking_location_id"),
    })
}

fn seconds_between(from: Timestamp, to: Timestamp) -> i64 {
    to.as_second() - from.as_second()
}

/// The recorded fact for the company, source and key, if any.
async fn find_event(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    source: &str,
    key: &str,
) -> Result<Option<EventRow>> {
    client
        .query_opt(
            &format!("SELECT {EVENT_COLUMNS} FROM people_attendance_clock_facts WHERE tenant_id = $1 AND company_id = $2 AND source = $3 AND event_key = $4"),
            &[&tenant, &company_id, &source, &key],
        )
        .await
        .map_err(Error::from)?
        .as_ref()
        .map(read_event)
        .transpose()
        .map_err(AttendanceError::Failed)
}

/// Bilimbi `replay_event/3`: the same facts answer the recorded event,
/// different facts are refused.
fn replay(
    existing: &EventRow,
    employee_id: i64,
    event: &Event,
    actor: Option<i64>,
) -> Result<ClockEvent> {
    if existing.employee_id == employee_id
        && existing.event_type == event.event_type
        && existing.occurred_at == event.occurred_at
        && existing.actor_user_id == actor
    {
        Ok(existing.view())
    } else {
        Err(AttendanceError::EventKeyConflict)
    }
}

/// The employee's day row for a date, locked for the transaction.
async fn locked_day(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    employee_id: i64,
    date: Date,
) -> Result<Option<DayRow>> {
    client
        .query_opt(
            &format!("SELECT {DAY_COLUMNS} FROM people_attendance_daily_summaries WHERE tenant_id = $1 AND company_id = $2 AND employee_id = $3 AND on_date = $4::text::date FOR UPDATE"),
            &[&tenant, &company_id, &employee_id, &date.to_string()],
        )
        .await
        .map_err(Error::from)?
        .as_ref()
        .map(read_day)
        .transpose()
        .map_err(AttendanceError::Failed)
}

/// Which day the event belongs to (Bilimbi `shift_day/5`): a clock-in's own
/// local date; any other event the previous day's open shift when it falls
/// within the maximum shift length of that clock-in, else its own date.
enum Target {
    Existing(DayRow),
    Create(Date),
}
async fn shift_day(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    employee_id: i64,
    event: &Event,
    max_shift_hours: i64,
) -> Result<Target> {
    let today = locked_day(client, tenant, company_id, employee_id, event.on_date).await?;
    if event.event_type == EventType::In {
        return Ok(today.map_or(Target::Create(event.on_date), Target::Existing));
    }
    if today.as_ref().is_some_and(DayRow::open_shift) {
        return Ok(today.map_or(Target::Create(event.on_date), Target::Existing));
    }
    let yesterday = event
        .on_date
        .checked_sub(Span::new().days(1))
        .map_err(|_| AttendanceError::InvalidEvent)?;
    let previous = locked_day(client, tenant, company_id, employee_id, yesterday).await?;
    if let Some(previous) = previous {
        if let Some(first_in) = previous.first_in_at {
            if previous.open_shift()
                && event.occurred_at >= first_in
                && seconds_between(first_in, event.occurred_at) <= max_shift_hours * 3600
            {
                return Ok(Target::Existing(previous));
            }
        }
    }
    Ok(today.map_or(Target::Create(event.on_date), Target::Existing))
}

/// Bilimbi `project_day/3`: the first clock-in and last clock-out of the
/// day's ordered events give the status and the raw worked span.
fn project(
    events: &[EventRow],
    max_shift_hours: i64,
) -> (DayStatus, Option<Timestamp>, Option<Timestamp>, i32) {
    let first_in = events
        .iter()
        .find(|event| event.event_type == EventType::In);
    let last_out = events
        .iter()
        .rev()
        .find(|event| event.event_type == EventType::Out);
    let complete = match (first_in, last_out) {
        (Some(first_in), Some(last_out)) => {
            last_out.occurred_at > first_in.occurred_at
                && seconds_between(first_in.occurred_at, last_out.occurred_at)
                    <= max_shift_hours * 3600
        }
        _ => false,
    };
    let minutes = match (complete, first_in, last_out) {
        (true, Some(first_in), Some(last_out)) => {
            i32::try_from(seconds_between(first_in.occurred_at, last_out.occurred_at) / 60)
                .unwrap_or(i32::MAX)
        }
        _ => 0,
    };
    let status = if complete {
        DayStatus::ReadyForReview
    } else if first_in.is_some() && last_out.is_none() {
        DayStatus::InProgress
    } else {
        DayStatus::ExceptionPending
    };
    (
        status,
        first_in.map(|event| event.occurred_at),
        last_out.map(|event| event.occurred_at),
        minutes,
    )
}

/// Bilimbi `write_event/5` inside the caller's transaction, which holds the
/// company's lock: answers a replay, else finds or creates the day, inserts
/// the fact and re-projects the day, every row a recorded change.
pub(crate) async fn write(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company_id: i64,
    employee_id: i64,
    event: Event,
    clocking_location_id: Option<i64>,
    max_shift_hours: i64,
) -> Result<ClockEvent> {
    let tenant = scope.tenant();
    let actor = Some(scope.user());
    if let Some(existing) =
        find_event(tx.sql(), tenant, company_id, &event.source, &event.key).await?
    {
        return replay(&existing, employee_id, &event, actor);
    }
    let target = shift_day(
        tx.sql(),
        tenant,
        company_id,
        employee_id,
        &event,
        max_shift_hours,
    )
    .await?;
    tx.mutate(move |sql| {
        Box::pin(async move {
            let mut changes = Vec::with_capacity(3);
            let day = match target {
                Target::Existing(day) => day,
                Target::Create(date) => {
                    let row = sql
                        .query_one(
                            &format!("INSERT INTO people_attendance_daily_summaries (tenant_id, company_id, employee_id, on_date, status) VALUES ($1, $2, $3, $4::text::date, 'in_progress') RETURNING {DAY_COLUMNS}"),
                            &[&tenant, &company_id, &employee_id, &date.to_string()],
                        )
                        .await?;
                    let day = read_day(&row)?;
                    changes.push(Change {
                        kind: "people_attendance_daily_summary".into(),
                        id: day.id,
                        old: json!({}),
                        new: day.audit_values(),
                    });
                    day
                }
            };
            let (latitude, longitude) = match event.point {
                Some((latitude, longitude)) => (Some(latitude), Some(longitude)),
                None => (None, None),
            };
            let row = sql
                .query_one(
                    &format!("INSERT INTO people_attendance_clock_facts (tenant_id, company_id, employee_id, day_id, event_key, event_type, source, occurred_at, timezone, actor_user_id, latitude, longitude, clocking_location_id) \
                              VALUES ($1, $2, $3, $4, $5, $6, $7, $8::text::timestamptz, $9, $10, $11, $12, $13) RETURNING {EVENT_COLUMNS}"),
                    &[
                        &tenant,
                        &company_id,
                        &employee_id,
                        &day.id,
                        &event.key,
                        &event.event_type.as_str(),
                        &event.source,
                        &event.occurred_at.to_string(),
                        &event.timezone,
                        &actor,
                        &latitude,
                        &longitude,
                        &clocking_location_id,
                    ],
                )
                .await?;
            let saved = read_event(&row)?;
            changes.push(Change {
                kind: "people_attendance_clock_fact".into(),
                id: saved.id,
                old: json!({}),
                new: saved.audit_values(),
            });
            let events = sql
                .query(
                    &format!("SELECT {EVENT_COLUMNS} FROM people_attendance_clock_facts WHERE day_id = $1 AND company_id = $2 AND tenant_id = $3 ORDER BY occurred_at, id"),
                    &[&day.id, &company_id, &tenant],
                )
                .await?
                .iter()
                .map(read_event)
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let (status, first_in_at, last_out_at, worked_minutes) = project(&events, max_shift_hours);
            let row = sql
                .query_one(
                    &format!("UPDATE people_attendance_daily_summaries SET status = $2, first_in_at = $3::text::timestamptz, last_out_at = $4::text::timestamptz, worked_minutes = $5, updated_at = now() WHERE id = $1 AND tenant_id = $6 AND company_id = $7 RETURNING {DAY_COLUMNS}"),
                    &[
                        &day.id,
                        &status.as_str(),
                        &first_in_at.map(|at| at.to_string()),
                        &last_out_at.map(|at| at.to_string()),
                        &worked_minutes,
                        &tenant,
                        &company_id,
                    ],
                )
                .await?;
            let projected = read_day(&row)?;
            changes.push(Change {
                kind: "people_attendance_daily_summary".into(),
                id: day.id,
                old: day.audit_values(),
                new: projected.audit_values(),
            });
            Ok((saved.view(), changes))
        })
    })
    .await
    .map_err(AttendanceError::from)
}

/// The employee's [`DAYS_SHOWN`] most recent days, newest first.
pub(crate) async fn days(
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company_id: i64,
    employee_id: i64,
    max_shift_hours: i64,
) -> Result<Vec<Day>> {
    let now = Timestamp::now();
    client
        .query(
            &format!("SELECT {DAY_COLUMNS} FROM people_attendance_daily_summaries WHERE tenant_id = $1 AND company_id = $2 AND employee_id = $3 ORDER BY on_date DESC LIMIT {DAYS_SHOWN}"),
            &[&scope.tenant(), &company_id, &employee_id],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(|row| read_day(row).map(|day| day.view(max_shift_hours, now)))
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(AttendanceError::Failed)
}
