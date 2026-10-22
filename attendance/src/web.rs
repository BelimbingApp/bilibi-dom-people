//! My attendance, mirroring Bilimbi Attendance.Web.MyLive. The page never
//! stores an employee identity: every facade read/write resolves it afresh.
use crate::{
    AdjustmentRequest, Attendance, AttendanceError, Day, EventType, NewAdjustment, PublishedShift,
    Rules, SELF_ROSTER_DAYS, SELF_VIEW,
};
use askama::Template;
use bilibi_database::{Db, Error, Result};
use bilibi_people_workforce::authorization;
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::{
        LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState, MountFuture, Page, PageBody,
        PageFuture, PageRequest,
    },
    params, Badge, BadgeKind, BuiltinColumn, Button, ButtonVariant, Choice, ColumnsOutcome,
    ConfirmDialog, EmptyState, Field, FlashMap, FlexRow, FlexTable, Html, InputKind, ListColumns,
    ListSpec, ListState, LocationAction, LocationReport, StatusKind, TableEmpty, NO_FLASH,
};
use jiff::{tz::TimeZone, Timestamp};
use rust_decimal::Decimal;
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

pub(crate) const PAGES: &str = "people/attendance.my_pages";
const PERMISSION: &str = "You no longer have permission to use attendance self-service here.";
struct Core {
    db: Db,
    attendance: Arc<Attendance>,
}
pub(crate) struct Pages(Arc<Core>);
impl Pages {
    pub(crate) fn new(db: Db, attendance: Arc<Attendance>) -> Self {
        Self(Arc::new(Core { db, attendance }))
    }
}
impl Page<Scope, Error> for Pages {
    fn title(&self, _: &str) -> Option<&'static str> {
        Some("My attendance")
    }
    fn render<'a>(&'a self, r: PageRequest<'a, Scope>) -> PageFuture<'a, Error> {
        Box::pin(async move {
            let mut state = MyAttendance::new(self.0.clone(), r.query)?;
            state
                .load(r.scope.ok_or(Error::Unauthorized)?, true)
                .await?;
            Ok(PageBody {
                html: state.render()?,
                live: true,
                title: None,
            })
        })
    }
    fn mount<'a>(&'a self, r: PageRequest<'a, Scope>) -> MountFuture<'a, Scope, Error> {
        Box::pin(async move {
            Ok(Some(Box::new(MyAttendance::new(self.0.clone(), r.query)?)
                as Box<dyn LiveState<Scope, Error>>))
        })
    }
}
struct Records {
    rules: Rules,
    days: Vec<Day>,
    shifts: Vec<PublishedShift>,
    requests: Vec<AdjustmentRequest>,
    can_clock: bool,
}
struct MyAttendance {
    core: Arc<Core>,
    records: Option<Records>,
    company: Option<i64>,
    columns: [ListColumns; 3],
    lists: [ListState; 3],
    clock_key: String,
    request_key: String,
    pending_cancel: Option<i64>,
    form: HashMap<String, String>,
    flash: Option<(StatusKind, String)>,
}
fn render(t: &impl Template) -> Result<Html> {
    Html::render(t).map_err(|_| Error::Internal)
}
impl MyAttendance {
    fn new(core: Arc<Core>, query: &HashMap<String, String>) -> Result<Self> {
        let cols = |id, pairs: &[(&str, &str)]| {
            ListColumns::mount(
                id,
                pairs
                    .iter()
                    .map(|(key, label)| BuiltinColumn::new(key, label))
                    .collect(),
            )
            .map_err(|_| Error::Internal)
        };
        let list = |prefix: &str, bound| {
            ListState::parse(
                query,
                ListSpec {
                    page_sizes: vec![bound],
                    default_page_size: bound,
                    param_prefix: prefix.into(),
                    omit_defaults: true,
                    ..ListSpec::default()
                }
                .shared()
                .map_err(|_| Error::Internal)?,
            )
            .map_err(|_| Error::Internal)
        };
        Ok(Self {
            core,
            records: None,
            company: None,
            columns: [
                cols(
                    "my-attendance-days",
                    &[
                        ("date", "Date"),
                        ("status", "Status"),
                        ("minutes", "Minutes"),
                    ],
                )?,
                cols(
                    "my-attendance-shifts",
                    &[("date", "Date"), ("shift", "Shift")],
                )?,
                cols(
                    "my-attendance-requests",
                    &[
                        ("event", "Clock event"),
                        ("time", "Time"),
                        ("status", "Status"),
                        ("note", "Decision note"),
                        ("actions", "Actions"),
                    ],
                )?,
            ],
            lists: [
                list("days_", 31)?,
                list("shifts_", 14)?,
                list("requests_", 20)?,
            ],
            clock_key: uuid::Uuid::new_v4().to_string(),
            request_key: uuid::Uuid::new_v4().to_string(),
            pending_cancel: None,
            form: HashMap::new(),
            flash: None,
        })
    }
    async fn read(&self, scope: &Scope, company: i64) -> crate::Result<Records> {
        let a = &self.core.attendance;
        let days = a.self_days(scope, company).await?;
        let rules = a.rules(scope, company).await?;
        let zone = TimeZone::get(&rules.timezone).map_err(|_| AttendanceError::InvalidTime)?;
        let today = Timestamp::now().to_zoned(zone).date();
        let shifts = a
            .self_roster(scope, company, today, SELF_ROSTER_DAYS)
            .await?;
        let requests = a.self_adjustments(scope, company).await?;
        Ok(Records {
            rules,
            days,
            shifts,
            requests,
            can_clock: scope.impersonator().is_none() && scope.agent().is_none(),
        })
    }
    async fn load(&mut self, scope: &Scope, rows_changed: bool) -> Result<()> {
        self.company = scope.actor().organisation().map(|id| id.get());
        self.records = match self.company {
            Some(company) => match self.read(scope, company).await {
                Ok(records) => Some(records),
                Err(AttendanceError::Failed(error)) => return Err(error),
                Err(_) => None,
            },
            None => None,
        };
        if let Some(r) = &self.records {
            if rows_changed {
                self.columns[0].load(&r.days, |d| d.on_date.to_string());
                self.columns[1].load(&r.shifts, |s| s.on_date.clone());
                self.columns[2].load(&r.requests, |r| r.id.to_string());
            }
        } else {
            self.pending_cancel = None;
        }
        Ok(())
    }
    fn error(&mut self, message: impl Into<String>) {
        self.flash = Some((StatusKind::Error, message.into()));
    }
    fn success(&mut self, message: &str) {
        self.flash = Some((StatusKind::Success, message.into()));
    }
    async fn permitted(&mut self, scope: &Scope) -> Result<bool> {
        let Some(company) = self.company else {
            self.error(PERMISSION);
            return Ok(false);
        };
        let c = self.core.db.get().await?;
        match authorization::authorize(&**c, scope, company, SELF_VIEW).await {
            Ok(_) => Ok(true),
            Err(bilibi_people_workforce::WorkforceError::Failed(e)) => Err(e),
            Err(_) => {
                self.error(PERMISSION);
                self.records = None;
                self.pending_cancel = None;
                Ok(false)
            }
        }
    }
    async fn event(&mut self, scope: &Scope, payload: &Value) -> Result<bool> {
        let kind = payload.get("type").and_then(Value::as_str).unwrap_or("");
        if kind == "clear-flash" {
            if self
                .flash
                .as_ref()
                .is_some_and(|(k, _)| Some(k.name()) == payload.get("key").and_then(Value::as_str))
            {
                self.flash = None;
            }
            return Ok(false);
        }
        if kind == "cancel_confirmation" {
            self.pending_cancel = None;
            return Ok(false);
        }
        if let Some(index) = ["days_grid", "shifts_grid", "requests_grid"]
            .iter()
            .position(|event| *event == kind)
        {
            let values = params::string_values(payload);
            let _: ColumnsOutcome = self.columns[index].handle(&values);
            return Ok(false);
        }
        if !self.permitted(scope).await? {
            return Ok(true);
        }
        let Some(company) = self.company else {
            return Ok(true);
        };
        match kind {
            "clock" => {
                let Some(r) = &self.records else {
                    self.error("Self clocking is unavailable for this account.");
                    return Ok(true);
                };
                if !r.can_clock || !r.rules.self_clock_enabled {
                    self.error("Self clocking is unavailable for this account.");
                    return Ok(true);
                }
                let mut coordinates = (None, None);
                if r.rules.location_required {
                    match LocationReport::from_event(payload) {
                        Some(LocationReport::Position {
                            latitude,
                            longitude,
                            ..
                        }) => {
                            coordinates = (
                                Decimal::from_f64_retain(latitude),
                                Decimal::from_f64_retain(longitude),
                            )
                        }
                        Some(report) => {
                            self.error(match report {
                                LocationReport::PermissionDenied => "Location permission was refused. Allow location access in your browser and try again, or request an adjustment below.".into(),
                                LocationReport::InsecureContext => format!("{} Or request an adjustment below.", report.refusal().unwrap_or_default()),
                                _ => "Your location is unavailable. Check location access and try again, or request an adjustment below.".into(),
                            });
                            return Ok(true);
                        }
                        None => {
                            self.error("A valid location is required to clock here. Try again, or request an adjustment below.");
                            return Ok(true);
                        }
                    }
                }
                let Some(event_type) = payload
                    .get("event_type")
                    .and_then(Value::as_str)
                    .and_then(EventType::parse)
                    .filter(|e| matches!(e, EventType::In | EventType::Out))
                else {
                    self.error("Clock event could not be recorded.");
                    return Ok(true);
                };
                match self.core.attendance.self_clock(scope, company, event_type, &self.clock_key, coordinates.0, coordinates.1).await {
                    Ok(_) => { self.clock_key = uuid::Uuid::new_v4().to_string(); self.success("Clock event recorded."); }
                    Err(AttendanceError::LocationRequired | AttendanceError::InvalidEvent) => self.error("A valid location is required to clock here. Try again, or request an adjustment below."),
                    Err(AttendanceError::OutsideClockingLocation) => self.error("You are outside this company's approved clocking locations. Move to an approved location and try again, or request an adjustment below."),
                    Err(AttendanceError::SelfClockUnavailable | AttendanceError::Unavailable(_)) => self.error("Clocking is unavailable for this account."),
                    Err(e) => self.refused(e, "Clock event could not be recorded.")?,
                }
            }
            "request_adjustment" => {
                self.form = params::form_values(payload, "adjustment");
                let Some(event_type) = self
                    .form
                    .get("event_type")
                    .and_then(|s| EventType::parse(s))
                    .filter(|e| matches!(e, EventType::In | EventType::Out))
                else {
                    self.error("Choose clock in or out, a time, and give a reason.");
                    return Ok(true);
                };
                let new = NewAdjustment {
                    request_key: self.request_key.clone(),
                    event_type,
                    local_at: self.form.get("local_at").cloned().unwrap_or_default(),
                    reason: self.form.get("reason").cloned().unwrap_or_default(),
                };
                match self
                    .core
                    .attendance
                    .submit_adjustment(scope, company, &new)
                    .await
                {
                    Ok(_) => {
                        self.request_key = uuid::Uuid::new_v4().to_string();
                        self.form.clear();
                        self.success("Adjustment request submitted.");
                    }
                    Err(e) => {
                        self.refused(e, "Choose clock in or out, a time, and give a reason.")?
                    }
                }
            }
            "cancel_adjustment" => {
                let id = payload
                    .get("id")
                    .and_then(Value::as_str)
                    .and_then(|id| id.parse::<i64>().ok());
                self.pending_cancel = id.filter(|id| {
                    self.records.as_ref().is_some_and(|r| {
                        r.requests
                            .iter()
                            .any(|r| r.id == *id && r.status == "pending")
                    })
                });
                if self.pending_cancel.is_none() {
                    self.error("The request can no longer be cancelled.");
                }
            }
            "confirm_cancel" => {
                if let Some(id) = self.pending_cancel.take() {
                    match self
                        .core
                        .attendance
                        .cancel_adjustment(scope, company, id)
                        .await
                    {
                        Ok(_) => self.success("Adjustment request cancelled."),
                        Err(e) => self.refused(e, "The request can no longer be cancelled.")?,
                    }
                }
            }
            _ => self.error("That action is unavailable."),
        }
        Ok(true)
    }
    fn refused(&mut self, e: AttendanceError, fallback: &str) -> Result<()> {
        let message = match e {
            AttendanceError::Failed(e) => return Err(e),
            AttendanceError::Unauthorized => PERMISSION.into(),
            AttendanceError::NotLinked => e.to_string(),
            AttendanceError::InvalidTime
            | AttendanceError::FutureTime
            | AttendanceError::OutsideWindow
            | AttendanceError::DuplicateRequest
            | AttendanceError::CompanyArchived => e.to_string(),
            AttendanceError::Unavailable(_) => {
                "Adjustment requests are unavailable for this account.".into()
            }
            _ => fallback.into(),
        };
        self.error(message);
        Ok(())
    }
    fn table(
        &self,
        index: usize,
        id: &str,
        caption: &str,
        event: &str,
        rows: Vec<FlexRow>,
        empty: EmptyState,
    ) -> Result<Html> {
        let c = &self.columns[index];
        let none = rows.is_empty();
        let mut table = FlexTable::new(id, c.flex_columns(), rows)
            .sorted(self.lists[index].sort_by.as_deref(), None);
        table.event = event;
        table.zoom = c.zoom;
        table.suggestions = c.flex_suggestions();
        table.add_query = &c.add_query;
        table.caption = caption;
        if none {
            table.empty = Some(TableEmpty::State(empty));
        }
        render(&table)
    }
    fn render(&self) -> Result<String> {
        let mut flash = NO_FLASH;
        if let Some((kind, text)) = &self.flash {
            match kind {
                StatusKind::Success => flash.success = text,
                _ => flash.error = text,
            }
        }
        let mut view = View {
            flash,
            available: self.records.is_some(),
            clock: Html::EMPTY,
            clock_hint: String::new(),
            days: Html::EMPTY,
            shifts: Html::EMPTY,
            requests: Html::EMPTY,
            fields: vec![],
            submit: Html::EMPTY,
            adjustment_hint: String::new(),
            confirmation: Html::EMPTY,
            unavailable: render(
                &EmptyState::missing(
                    "Attendance is unavailable for this account.",
                    "Ask an operator to link your account to an active employee in this company.",
                )
                .id("my-attendance-unavailable"),
            )?,
        };
        if let Some(r) = &self.records {
            if r.rules.self_clock_enabled {
                if r.rules.location_required {
                    view.clock_hint = "This company requires your location inside an approved clocking location. Your browser will ask for location access when you clock. If it is unavailable, request an adjustment below.".into();
                }
                for (event, label) in [("in", "Clock in"), ("out", "Clock out")] {
                    let id = format!("my-attendance-clock-{event}");
                    let action = if r.rules.location_required {
                        let mut button = LocationAction::new(&id, "clock", label)
                            .value("event_type", event)
                            .sending("Recording clock event…");
                        if !r.can_clock {
                            button = button.disabled();
                        }
                        render(&button)?
                    } else {
                        render(&Button {
                            id: &id,
                            event: "clock",
                            name: "event_type",
                            value: event,
                            disabled: !r.can_clock,
                            body: Html::text(label),
                            ..Default::default()
                        })?
                    };
                    view.clock.push(&action);
                }
                if !r.can_clock {
                    view.clock.push(&render(&EmptyState::missing(
                        "Self clocking is unavailable for this account.",
                        "",
                    ))?);
                }
            } else {
                view.clock = render(
                    &EmptyState::missing(
                        "Self clocking is off.",
                        "An operator can enable it in Attendance rules.",
                    )
                    .id("my-attendance-clocking-off"),
                )?;
            }
            let days = r
                .days
                .iter()
                .map(|d| {
                    FlexRow::new(d.on_date.to_string())
                        .slot("date", Html::text(&d.on_date.to_string()))
                        .slot("status", Html::text(&d.status.as_str().replace('_', " ")))
                        .slot("minutes", Html::text(&d.worked_minutes.to_string()))
                })
                .collect();
            view.days = self.table(
                0,
                "my-attendance-days",
                "Recent days",
                "days_grid",
                days,
                EmptyState::missing(
                    "No clock events yet.",
                    "Your recorded days will appear here.",
                )
                .id("my-attendance-empty"),
            )?;
            let shifts = r
                .shifts
                .iter()
                .map(|s| {
                    let hours = |minute: Option<i32>| {
                        minute
                            .map(|m| format!("{:02}:{:02}", m / 60, m % 60))
                            .unwrap_or_default()
                    };
                    let label = if s.kind == "rest" {
                        "Rest day".into()
                    } else {
                        format!(
                            "{} ({}) {}–{}",
                            s.shift_name.as_deref().unwrap_or(""),
                            s.shift_code.as_deref().unwrap_or(""),
                            hours(s.start_minute),
                            hours(s.end_minute)
                        )
                    };
                    FlexRow::new(&s.on_date)
                        .slot("date", Html::text(&s.on_date))
                        .slot("shift", Html::text(&label))
                })
                .collect();
            view.shifts = self.table(
                1,
                "my-attendance-shifts",
                "Upcoming shifts",
                "shifts_grid",
                shifts,
                EmptyState::missing("No published shifts for the next 14 days.", "")
                    .id("my-attendance-shifts-empty"),
            )?;
            let mut requests = vec![];
            for a in &r.requests {
                // An adjustment's wall time is pinned to the zone in which it was requested.
                let time = a
                    .proposed_at
                    .parse::<Timestamp>()
                    .ok()
                    .map(|at| {
                        bilibi_datetime::DisplayContext {
                            mode: bilibi_datetime::Mode::Company,
                            timezone: a.timezone.clone(),
                            ..bilibi_datetime::DisplayContext::anonymous()
                        }
                        .render(
                            at,
                            bilibi_datetime::Format::DateTime,
                            bilibi_datetime::Precision::Minute,
                        )
                        .text
                    })
                    .unwrap_or_default();
                let action = if a.status == "pending" {
                    let id = a.id.to_string();
                    render(&Button {
                        event: "cancel_adjustment",
                        name: "id",
                        value: &id,
                        body: Html::text("Cancel"),
                        ..Default::default()
                    })?
                } else {
                    Html::EMPTY
                };
                requests.push(
                    FlexRow::new(format!("my-adjustment-{}", a.id))
                        .slot(
                            "event",
                            Html::text(if a.event_type == "in" {
                                "Clock in"
                            } else {
                                "Clock out"
                            }),
                        )
                        .slot("time", Html::text(&time))
                        .slot(
                            "status",
                            render(&Badge::status(
                                match a.status.as_str() {
                                    "approved" => BadgeKind::Success,
                                    "rejected" => BadgeKind::Danger,
                                    "pending" => BadgeKind::Warning,
                                    _ => BadgeKind::Neutral,
                                },
                                &a.status,
                            ))?,
                        )
                        .slot("note", Html::text(a.decision_note.as_deref().unwrap_or("")))
                        .slot("actions", action),
                );
            }
            view.requests = self.table(
                2,
                "my-attendance-requests",
                "Adjustment requests",
                "requests_grid",
                requests,
                EmptyState::missing("You have no adjustment requests.", "")
                    .id("my-attendance-requests-empty"),
            )?;
            let value = |key: &str| self.form.get(key).cloned().unwrap_or_default();
            view.fields = vec![
                Field::in_form("adjustment", "event_type")
                    .wrapper_class("mb-0")
                    .label("Clock event")
                    .kind(InputKind::Select)
                    .options(vec![
                        Choice::new("Clock in", "in"),
                        Choice::new("Clock out", "out"),
                    ])
                    .value(
                        self.form
                            .get("event_type")
                            .map(String::as_str)
                            .unwrap_or("in"),
                    ),
                Field::in_form("adjustment", "local_at")
                    .wrapper_class("mb-0")
                    .label("Time")
                    .kind(InputKind::DatetimeLocal)
                    .required()
                    .value(value("local_at")),
                Field::in_form("adjustment", "reason")
                    .wrapper_class("mb-0 sm:col-span-2")
                    .label("Reason")
                    .required()
                    .attr("maxlength", "500")
                    .value(value("reason")),
            ];
            view.submit = render(&Button {
                button_type: "submit",
                variant: ButtonVariant::Primary,
                body: Html::text("Submit request"),
                ..Default::default()
            })?;
            view.adjustment_hint = format!("For a missed clock event in the last {} {}, in {} time. An approver adds it once accepted.",r.rules.adjustment_window_days, if r.rules.adjustment_window_days == 1 { "day" } else { "days" },r.rules.timezone);
        }
        if self.pending_cancel.is_some() {
            view.confirmation = render(&ConfirmDialog {
                id: "my-attendance-cancel-confirm",
                consequence: "Cancel this adjustment request?",
                detail: "This cannot be undone.",
                confirm: "Cancel request",
                working: "Cancelling…",
                confirm_event: "confirm_cancel",
                cancel_event: "cancel_confirmation",
            })?;
        }
        view.render().map_err(|_| Error::Internal)
    }
}
impl LiveState<Scope, Error> for MyAttendance {
    fn update<'a>(&'a mut self, r: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            // Re-read authority, link and rules before every event, including delayed location reports.
            self.load(r.scope, false).await?;
            let changed = match r.input {
                LiveInput::Event(payload) => self.event(r.scope, payload).await?,
                _ => true,
            };
            if changed {
                self.load(r.scope, true).await?;
            }
            Ok(LiveBody {
                html: self.render()?,
                rows: self
                    .records
                    .as_ref()
                    .map_or(0, |r| r.days.len() + r.shifts.len() + r.requests.len()),
                ..Default::default()
            })
        })
    }
}
#[derive(Template)]
#[template(path = "my.html")]
struct View<'a> {
    flash: FlashMap<'a>,
    available: bool,
    clock: Html,
    clock_hint: String,
    days: Html,
    shifts: Html,
    requests: Html,
    fields: Vec<Field>,
    submit: Html,
    adjustment_hint: String,
    confirmation: Html,
    unavailable: Html,
}
