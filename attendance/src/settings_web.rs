//! Bilimbi Attendance.Web.{Rules,Shifts,Locations,Allowances}Live and Components.
//! Company selection and every write recheck the current company capability.
use crate::{
    Attendance, AttendanceError, NewAllowanceRule, NewClockingLocation, NewShiftTemplate,
    RuleChanges, Rules, ALLOWANCES_MANAGE, RULES_MANAGE,
};
use askama::Template;
use bilibi_database::{Db, Error, Result};
use bilibi_people_workforce::{authorization, CompanySummary};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::{
        LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState, MountFuture, Page, PageBody,
        PageFuture, PageRequest,
    },
    params, Badge, BadgeKind, BuiltinColumn, Button, ButtonVariant, Choice, ConfirmDialog,
    EmptyState, Field, FlashMap, FlexRow, FlexTable, Form, Html, InputKind, ListColumns, ListSpec,
    ListState, StatusKind, Tab, TableEmpty, Tabs, NO_FLASH,
};
use jiff::civil::{Date, Time};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

pub(crate) const PAGES: &str = "people/attendance.settings_pages";
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
#[derive(Clone, Copy, PartialEq, Eq)]
enum Area {
    Rules,
    Shifts,
    Locations,
    Allowances,
}
impl Area {
    fn parse(operation: &str) -> Self {
        match operation {
            "shifts" => Self::Shifts,
            "locations" => Self::Locations,
            "allowances" => Self::Allowances,
            _ => Self::Rules,
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::Rules => "rules",
            Self::Shifts => "shifts",
            Self::Locations => "locations",
            Self::Allowances => "allowances",
        }
    }
    fn title(self) -> &'static str {
        match self {
            Self::Rules => "Attendance rules",
            Self::Shifts => "Shift templates",
            Self::Locations => "Clocking locations",
            Self::Allowances => "Allowance rules",
        }
    }
    fn noun(self) -> &'static str {
        match self {
            Self::Rules => "attendance rules",
            Self::Shifts => "shift templates",
            Self::Locations => "clocking locations",
            Self::Allowances => "allowance rules",
        }
    }
    fn capability(self) -> &'static str {
        if self == Self::Allowances {
            ALLOWANCES_MANAGE
        } else {
            RULES_MANAGE
        }
    }
    fn path(self) -> String {
        if self == Self::Rules {
            "/people/attendance/rules".into()
        } else {
            format!("/people/attendance/rules/{}", self.key())
        }
    }
    fn columns(self) -> Vec<BuiltinColumn> {
        let pairs: &[(&str, &str)] = match self {
            Self::Shifts => &[
                ("name", "Name"),
                ("code", "Code"),
                ("hours", "Hours"),
                ("minutes", "Minutes"),
                ("status", "Status"),
                ("actions", "Actions"),
            ],
            Self::Locations => &[
                ("name", "Name"),
                ("code", "Code"),
                ("coordinates", "Coordinates"),
                ("radius", "Radius (m)"),
                ("status", "Status"),
                ("actions", "Actions"),
            ],
            Self::Allowances => &[
                ("code", "Code"),
                ("name", "Name"),
                ("value", "Value"),
                ("effective", "Effective"),
                ("status", "Status"),
                ("actions", "Action"),
            ],
            Self::Rules => &[("name", "Name")],
        };
        pairs
            .iter()
            .map(|(k, l)| BuiltinColumn::new(k, l))
            .collect()
    }
}
impl Page<Scope, Error> for Pages {
    fn title(&self, operation: &str) -> Option<&'static str> {
        Some(Area::parse(operation).title())
    }
    fn render<'a>(&'a self, r: PageRequest<'a, Scope>) -> PageFuture<'a, Error> {
        Box::pin(async move {
            let mut state = State::new(self.0.clone(), Area::parse(r.operation), r.query)?;
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
            Ok(Some(Box::new(State::new(
                self.0.clone(),
                Area::parse(r.operation),
                r.query,
            )?) as Box<dyn LiveState<Scope, Error>>))
        })
    }
}
#[derive(Clone)]
enum Pending {
    Status(i64, String),
    Retire(i64),
    End(i64, Date),
}
struct State {
    core: Arc<Core>,
    area: Area,
    companies: Vec<CompanySummary>,
    company: Option<i64>,
    rules: Option<Rules>,
    rows: Vec<FlexRow>,
    available: bool,
    allowances: bool,
    instance: Option<bilibi_system::Instance>,
    columns: ListColumns,
    list: ListState,
    form: HashMap<String, String>,
    pending: Option<Pending>,
    flash: Option<(StatusKind, String)>,
}
fn html(t: &impl Template) -> Result<Html> {
    Html::render(t).map_err(|_| Error::Internal)
}
fn button(event: &str, label: &str, id: i64) -> Result<Html> {
    html(&Button {
        event,
        name: "id",
        value: &id.to_string(),
        body: Html::text(label),
        ..Default::default()
    })
}
fn spec() -> Result<Arc<ListSpec>> {
    ListSpec {
        omit_defaults: true,
        ..Default::default()
    }
    .shared()
    .map_err(|_| Error::Internal)
}
impl State {
    fn new(core: Arc<Core>, area: Area, query: &HashMap<String, String>) -> Result<Self> {
        Ok(Self {
            core,
            area,
            companies: vec![],
            company: params::positive_id(query.get("company_id").map(String::as_str)),
            rules: None,
            rows: vec![],
            available: false,
            allowances: false,
            instance: None,
            columns: ListColumns::mount(&format!("attendance-{}", area.key()), area.columns())
                .map_err(|_| Error::Internal)?,
            list: ListState::parse(query, spec()?).map_err(|_| Error::Internal)?,
            form: HashMap::new(),
            pending: None,
            flash: None,
        })
    }
    async fn load(&mut self, scope: &Scope, changed: bool) -> Result<()> {
        let c = self.core.db.get().await?;
        self.companies =
            match authorization::selectable_companies(&**c, scope, self.area.capability()).await {
                Ok(values) => values.into_iter().filter(|v| !v.archived).collect(),
                Err(bilibi_people_workforce::WorkforceError::Failed(e)) => return Err(e),
                _ => vec![],
            };
        let old = self.company;
        self.company = self
            .company
            .filter(|id| self.companies.iter().any(|c| c.id == *id))
            .or_else(|| self.companies.first().map(|c| c.id));
        if old != self.company {
            self.pending = None;
            self.form.clear();
        }
        self.rules = None;
        self.rows.clear();
        self.available = false;
        self.allowances = if let Some(company) = self.company {
            authorization::allowed(&**c, scope, company, ALLOWANCES_MANAGE).await
        } else {
            false
        };
        self.instance =
            Some(bilibi_system::Instance::resolve(&self.core.attendance.settings, &**c).await?);
        drop(c);
        if let Some(company) = self.company {
            match self.read(scope, company).await {
                Ok(()) => self.available = true,
                Err(AttendanceError::Failed(e)) => return Err(e),
                Err(_) => {
                    self.pending = None;
                }
            }
        }
        if changed || old != self.company {
            self.columns.load(&self.rows, |r| r.key.clone());
        }
        Ok(())
    }
    async fn read(&mut self, scope: &Scope, company: i64) -> crate::Result<()> {
        let a = &self.core.attendance;
        match self.area {
            Area::Rules => {
                self.rules = Some(a.rules(scope, company).await?);
            }
            Area::Shifts => {
                for s in a.list_shift_templates(scope, company).await? {
                    self.rows.push(
                        FlexRow::new(s.id.to_string())
                            .slot("name", Html::text(&s.name))
                            .slot("code", Html::text(&s.code))
                            .slot(
                                "hours",
                                Html::text(&format!("{}–{}", s.starts_at(), s.ends_at())),
                            )
                            .slot(
                                "minutes",
                                Html::text(&format!(
                                    "{} min, break {} min",
                                    s.span_minutes(),
                                    s.break_minutes
                                )),
                            )
                            .slot(
                                "status",
                                html(&Badge::status(
                                    if s.status == "active" {
                                        BadgeKind::Success
                                    } else {
                                        BadgeKind::Neutral
                                    },
                                    &s.status,
                                ))?,
                            )
                            .slot(
                                "actions",
                                button(
                                    "set_status",
                                    if s.status == "active" {
                                        "Retire"
                                    } else {
                                        "Reactivate"
                                    },
                                    s.id,
                                )?,
                            ),
                    );
                }
            }
            Area::Locations => {
                self.rules = Some(a.rules(scope, company).await?);
                for l in a.list_clocking_locations(scope, company).await? {
                    self.rows.push(
                        FlexRow::new(l.id.to_string())
                            .slot("name", Html::text(&l.name))
                            .slot("code", Html::text(&l.code))
                            .slot(
                                "coordinates",
                                Html::text(&format!("{}, {}", l.latitude, l.longitude)),
                            )
                            .slot("radius", Html::text(&l.radius_meters.to_string()))
                            .slot(
                                "status",
                                html(&Badge::status(
                                    if l.status == "active" {
                                        BadgeKind::Success
                                    } else {
                                        BadgeKind::Neutral
                                    },
                                    &l.status,
                                ))?,
                            )
                            .slot(
                                "actions",
                                button(
                                    "set_status",
                                    if l.status == "active" {
                                        "Retire"
                                    } else {
                                        "Reactivate"
                                    },
                                    l.id,
                                )?,
                            ),
                    );
                }
            }
            Area::Allowances => {
                for r in a.list_allowance_rules(scope, company).await? {
                    let mut actions = Html::EMPTY;
                    if r.status == "active" {
                        let body = html(
                            &Field::named("rule_id")
                                .id(format!("end-rule-{}", r.id))
                                .kind(InputKind::Hidden)
                                .value(r.id.to_string()),
                        )?;
                        let mut fields = body;
                        let mut date = Field::named("effective_until")
                            .id(format!("end-date-{}", r.id))
                            .label(format!("End date for {}", r.code))
                            .wrapper_class("mb-0")
                            .kind(InputKind::Date)
                            .attr("required", "")
                            .attr("min", &r.effective_from);
                        if let Some(until) = &r.effective_until {
                            date = date.attr("max", until);
                        }
                        fields.push(&html(&date)?);
                        actions.push(&html(&Form {
                            id: &format!("allowance-end-{}", r.id),
                            event: "end_date",
                            class: "flex items-center gap-2 ui-compact-controls",
                            body: fields,
                            actions: html(&Button {
                                button_type: "submit",
                                body: Html::text("End"),
                                ..Default::default()
                            })?,
                            ..Default::default()
                        })?);
                        actions.push(&button("retire", "Retire", r.id)?);
                    }
                    self.rows.push(
                        FlexRow::new(r.id.to_string())
                            .slot("code", Html::text(&r.code))
                            .slot("name", Html::text(&r.name))
                            .slot(
                                "value",
                                Html::text(&format!("{} {} / {}", r.value, r.currency, r.unit)),
                            )
                            .slot(
                                "effective",
                                Html::text(&format!(
                                    "{} – {}",
                                    r.effective_from,
                                    r.effective_until.as_deref().unwrap_or("Open")
                                )),
                            )
                            .slot("status", Html::text(&r.status))
                            .slot("actions", actions),
                    );
                }
            }
        }
        for row in &mut self.rows {
            row.dom_id = Some(format!(
                "{}-{}",
                match self.area {
                    Area::Shifts => "shift-template",
                    Area::Locations => "clocking-location",
                    Area::Allowances => "allowance-rule",
                    Area::Rules => "attendance-rule",
                },
                row.key
            ));
        }
        Ok(())
    }
    fn message(&mut self, kind: StatusKind, text: impl Into<String>) {
        self.flash = Some((kind, text.into()));
    }
    fn refusal(&self) -> String {
        format!(
            "You no longer have permission to change this company's {}.",
            self.area.noun()
        )
    }
    fn invalid(&self) -> &'static str {
        match self.area {
        Area::Rules => "Enter a valid time zone, a shift length of 1 to 24 hours, and an adjustment window of 1 to 366 days.",
        Area::Shifts => "Enter a code, a name, and start and end times.",
        Area::Locations => "Enter a code, a name, a latitude from -90 to 90, a longitude from -180 to 180, and a radius from 1 to 100000 meters.",
        Area::Allowances => "Check the allowance rule fields and effective dates.",
    }
    }
    fn refused(&mut self, error: AttendanceError, creating: bool) -> Result<()> {
        let text = match error {
            AttendanceError::Failed(e) => return Err(e),
            AttendanceError::Unauthorized => self.refusal(),
            AttendanceError::CompanyArchived => error.to_string(),
            AttendanceError::EffectivePeriodOverlap => error.to_string(),
            AttendanceError::Invalid(ref fields) if creating => {
                if fields.iter().any(|f| f.message == "has already been taken") {
                    format!(
                        "That code is already used by another {}.",
                        if self.area == Area::Shifts {
                            "shift template"
                        } else {
                            "clocking location"
                        }
                    )
                } else if self.area == Area::Shifts && fields.iter().any(|f| f.field == "ends_at") {
                    "The shift must end at a different time than it starts.".into()
                } else if self.area == Area::Shifts
                    && fields
                        .iter()
                        .any(|f| f.message == "must be shorter than the shift")
                {
                    "The break must be shorter than the shift.".into()
                } else if self.area == Area::Shifts
                    && fields.iter().any(|f| f.field == "break_minutes")
                {
                    "Enter break minutes of 0 or more.".into()
                } else {
                    self.invalid().into()
                }
            }
            _ if creating => self.invalid().into(),
            _ => format!(
                "{} could not be updated.",
                match self.area {
                    Area::Shifts => "Shift template",
                    Area::Locations => "Clocking location",
                    _ => "Allowance rule",
                }
            ),
        };
        self.message(StatusKind::Error, text);
        Ok(())
    }
    async fn permitted(&mut self, scope: &Scope) -> Result<bool> {
        let c = self.core.db.get().await?;
        if let Some(company) = self.company {
            match authorization::authorize(&**c, scope, company, self.area.capability()).await {
                Ok(_) => return Ok(true),
                Err(bilibi_people_workforce::WorkforceError::Failed(e)) => return Err(e),
                _ => {}
            }
        }
        self.pending = None;
        self.message(StatusKind::Error, self.refusal());
        Ok(false)
    }
    async fn event(&mut self, scope: &Scope, payload: &Value) -> Result<bool> {
        let event = payload.get("type").and_then(Value::as_str).unwrap_or("");
        match event {
            "clear-flash" => {
                self.flash = None;
                return Ok(false);
            }
            "cancel_confirmation" => {
                self.pending = None;
                return Ok(false);
            }
            "grid" => {
                self.columns.handle(&params::string_values(payload));
                return Ok(false);
            }
            "select_company" => {
                let values = params::form_values(payload, "company");
                self.company = params::positive_id(values.get("company_id").map(String::as_str));
                self.pending = None;
                self.form.clear();
                return Ok(true);
            }
            _ => {}
        }
        if !self.permitted(scope).await? {
            return Ok(false);
        }
        let company = self.company.ok_or(Error::Invalid)?;
        match event {
            "save_rules" | "create" => {
                self.form = params::form_values(payload, "form");
                let result = self.create(scope, company).await;
                match result {
                    Ok(()) => {
                        self.form.clear();
                        self.message(
                            StatusKind::Success,
                            match self.area {
                                Area::Rules => "Attendance rules saved.",
                                Area::Shifts => "Shift template added.",
                                Area::Locations => "Clocking location added.",
                                Area::Allowances => "Allowance rule added.",
                            },
                        );
                    }
                    Err(e) => self.refused(e, true)?,
                }
            }
            "set_status" | "retire" => {
                let id = payload
                    .get("id")
                    .and_then(Value::as_str)
                    .and_then(|v| v.parse::<i64>().ok());
                if let Some(id) = id.filter(|id| self.rows.iter().any(|r| r.key == id.to_string()))
                {
                    self.pending = Some(if event == "retire" {
                        Pending::Retire(id)
                    } else {
                        // Derive the target status from current facade records, never a submitted status.
                        let status = if self.area == Area::Shifts {
                            self.core
                                .attendance
                                .list_shift_templates(scope, company)
                                .await
                                .map(|rows| rows.into_iter().find(|r| r.id == id).map(|r| r.status))
                        } else {
                            self.core
                                .attendance
                                .list_clocking_locations(scope, company)
                                .await
                                .map(|rows| rows.into_iter().find(|r| r.id == id).map(|r| r.status))
                        };
                        match status {
                            Ok(Some(status)) => Pending::Status(
                                id,
                                if status == "active" {
                                    "retired".into()
                                } else {
                                    "active".into()
                                },
                            ),
                            Err(e) => {
                                self.refused(e, false)?;
                                return Ok(true);
                            }
                            _ => return Ok(true),
                        }
                    });
                }
            }
            "end_date" => {
                let values = params::string_values(payload);
                if let (Some(id), Some(date)) = (
                    values.get("rule_id").and_then(|s| s.parse::<i64>().ok()),
                    values
                        .get("effective_until")
                        .and_then(|s| s.parse::<Date>().ok()),
                ) {
                    if self.rows.iter().any(|r| r.key == id.to_string()) {
                        self.pending = Some(Pending::End(id, date));
                    }
                } else {
                    self.message(StatusKind::Error, "Allowance rule could not be updated.");
                }
            }
            "confirm_change" => {
                if let Some(pending) = self.pending.take() {
                    let a = &self.core.attendance;
                    let result = match pending {
                        Pending::Status(id, status) if self.area == Area::Shifts => a
                            .set_shift_template_status(scope, company, id, &status)
                            .await
                            .map(|_| ()),
                        Pending::Status(id, status) if self.area == Area::Locations => a
                            .set_clocking_location_status(scope, company, id, &status)
                            .await
                            .map(|_| ()),
                        Pending::Retire(id) if self.area == Area::Allowances => a
                            .retire_allowance_rule(scope, company, id)
                            .await
                            .map(|_| ()),
                        Pending::End(id, date) if self.area == Area::Allowances => a
                            .end_allowance_rule(scope, company, id, date)
                            .await
                            .map(|_| ()),
                        _ => return Ok(false),
                    };
                    match result {
                        Ok(()) => self.message(
                            StatusKind::Success,
                            match self.area {
                                Area::Shifts => "Shift template updated.",
                                Area::Locations => "Clocking location updated.",
                                _ => "Allowance rule updated.",
                            },
                        ),
                        Err(e) => self.refused(e, false)?,
                    }
                }
            }
            _ => {}
        }
        Ok(true)
    }
    async fn create(&self, scope: &Scope, company: i64) -> crate::Result<()> {
        let v = |k: &str| self.form.get(k).cloned().unwrap_or_default();
        let bad = || AttendanceError::Invalid(vec![]);
        let a = &self.core.attendance;
        match self.area {
            Area::Rules => a
                .put_rules(
                    scope,
                    company,
                    &RuleChanges {
                        timezone: Some(v("timezone")),
                        self_clock_enabled: Some(v("enabled") == "true"),
                        max_shift_hours: Some(
                            v("max_shift_hours")
                                .trim()
                                .parse()
                                .map_err(|_| AttendanceError::InvalidRules)?,
                        ),
                        location_required: Some(v("location_required") == "true"),
                        adjustment_window_days: Some(
                            v("adjustment_window_days")
                                .trim()
                                .parse()
                                .map_err(|_| AttendanceError::InvalidRules)?,
                        ),
                    },
                )
                .await
                .map(|_| ()),
            Area::Shifts => a
                .create_shift_template(
                    scope,
                    company,
                    &NewShiftTemplate {
                        code: v("code"),
                        name: v("name"),
                        starts_at: v("starts_at").parse::<Time>().map_err(|_| bad())?,
                        ends_at: v("ends_at").parse::<Time>().map_err(|_| bad())?,
                        break_minutes: v("break_minutes").parse().map_err(|_| bad())?,
                    },
                )
                .await
                .map(|_| ()),
            Area::Locations => a
                .create_clocking_location(
                    scope,
                    company,
                    &NewClockingLocation {
                        code: v("code"),
                        name: v("name"),
                        latitude: v("latitude").parse().map_err(|_| bad())?,
                        longitude: v("longitude").parse().map_err(|_| bad())?,
                        radius_meters: v("radius_meters").parse().map_err(|_| bad())?,
                    },
                )
                .await
                .map(|_| ()),
            Area::Allowances => a
                .create_allowance_rule(
                    scope,
                    company,
                    &NewAllowanceRule {
                        code: v("code"),
                        name: v("name"),
                        unit: v("unit"),
                        value: v("value").parse().map_err(|_| bad())?,
                        currency: v("currency"),
                        effective_from: v("effective_from").parse().map_err(|_| bad())?,
                        effective_until: if v("effective_until").is_empty() {
                            None
                        } else {
                            Some(v("effective_until").parse().map_err(|_| bad())?)
                        },
                    },
                )
                .await
                .map(|_| ()),
        }
    }
    fn fields(&self) -> Vec<Field> {
        let field = |key: &str, label: &str, kind: InputKind, default: String| {
            Field::in_form("form", key)
                .id(match key {
                    "timezone" => "attendance-timezone".into(),
                    "max_shift_hours" => "attendance-max-shift".into(),
                    "adjustment_window_days" => "attendance-adjustment-window".into(),
                    _ => format!("attendance-{}-{key}", self.area.key()),
                })
                .label(label)
                .wrapper_class("mb-0")
                .kind(kind)
                .value(self.form.get(key).cloned().unwrap_or(default))
                .attr("required", "")
        };
        if self.area == Area::Rules {
            let Some(r) = &self.rules else { return vec![] };
            return vec![
                field(
                    "timezone",
                    "Attendance time zone",
                    InputKind::Text,
                    r.timezone.clone(),
                ),
                Field::in_form("form", "enabled")
                    .label("Allow employee clocking")
                    .kind(InputKind::Checkbox)
                    .checked(
                        self.form
                            .get("enabled")
                            .map_or(r.self_clock_enabled, |v| v == "true"),
                    ),
                field(
                    "max_shift_hours",
                    "Maximum shift length (hours)",
                    InputKind::Number,
                    r.max_shift_hours.to_string(),
                )
                .attr("min", "1")
                .attr("max", "24"),
                Field::in_form("form", "location_required")
                    .label("Require an active clocking location for clock events")
                    .kind(InputKind::Checkbox)
                    .checked(
                        self.form
                            .get("location_required")
                            .map_or(r.location_required, |v| v == "true"),
                    ),
                field(
                    "adjustment_window_days",
                    "Adjustment request window (days, counting today)",
                    InputKind::Number,
                    r.adjustment_window_days.to_string(),
                )
                .attr("min", "1")
                .attr("max", "366"),
            ];
        }
        let mut fields = vec![
            field("code", "Code", InputKind::Text, String::new()).attr("maxlength", "40"),
            field("name", "Name", InputKind::Text, String::new()).attr("maxlength", "120"),
        ];
        match self.area {
            Area::Shifts => fields.extend([
                field("starts_at", "Starts", InputKind::Time, String::new()),
                field("ends_at", "Ends", InputKind::Time, String::new()),
                field(
                    "break_minutes",
                    "Break minutes",
                    InputKind::Number,
                    "0".into(),
                )
                .attr("min", "0"),
            ]),
            Area::Locations => fields.extend([
                field("latitude", "Latitude", InputKind::Text, String::new())
                    .attr("inputmode", "decimal"),
                field("longitude", "Longitude", InputKind::Text, String::new())
                    .attr("inputmode", "decimal"),
                field(
                    "radius_meters",
                    "Radius (m)",
                    InputKind::Number,
                    String::new(),
                )
                .attr("min", "1")
                .attr("max", "100000"),
            ]),
            Area::Allowances => {
                fields.extend([
                    field("unit", "Unit", InputKind::Text, String::new())
                        .attr("maxlength", "32")
                        .attr("placeholder", "hour, shift, event"),
                    field("value", "Value", InputKind::Number, String::new())
                        .attr("min", "0.0001")
                        .attr("step", "0.0001"),
                    field("currency", "Currency", InputKind::Text, String::new())
                        .attr("minlength", "3")
                        .attr("maxlength", "3"),
                    field(
                        "effective_from",
                        "Effective from",
                        InputKind::Date,
                        String::new(),
                    ),
                    Field::in_form("form", "effective_until")
                        .label("Effective until")
                        .kind(InputKind::Date)
                        .value(
                            self.form
                                .get("effective_until")
                                .cloned()
                                .unwrap_or_default(),
                        ),
                ]);
            }
            _ => {}
        }
        fields
    }
    fn render(&self) -> Result<String> {
        let mut flash = NO_FLASH;
        if let Some((kind, text)) = &self.flash {
            if *kind == StatusKind::Success {
                flash.success = text
            } else {
                flash.error = text
            }
        }
        let query = self
            .company
            .map(|id| format!("?company_id={id}"))
            .unwrap_or_default();
        let areas = [Area::Rules, Area::Allowances, Area::Shifts, Area::Locations];
        let links: Vec<_> = areas
            .iter()
            .map(|a| format!("{}{query}", a.path()))
            .collect();
        let mut tabs = vec![];
        for (i, area) in areas.iter().enumerate() {
            if *area == Area::Allowances && !self.allowances {
                continue;
            }
            let mut tab = Tab::patch(
                &links[i],
                if *area == Area::Rules {
                    "Rules"
                } else {
                    area.title()
                },
                *area == self.area,
            );
            tab.id = match area {
                Area::Rules => "attendance-rules-tab",
                Area::Shifts => "attendance-shifts-tab",
                Area::Locations => "attendance-locations-tab",
                Area::Allowances => "attendance-allowances-tab",
            };
            tabs.push(tab);
        }
        let tabs = html(&Tabs {
            id: "attendance-rules-tabs",
            label: "Attendance rules",
            tabs,
            ..Default::default()
        })?;
        let company_field = Field::in_form("company", "company_id")
            .label("Company")
            .kind(InputKind::Select)
            .value(self.company.map(|v| v.to_string()).unwrap_or_default())
            .options(
                self.companies
                    .iter()
                    .map(|c| Choice::new(&c.name, c.id.to_string()))
                    .collect(),
            );
        let company_form = html(&Form {
            id: match self.area {
                Area::Rules => "attendance-company-form",
                Area::Shifts => "attendance-shifts-company",
                Area::Locations => "attendance-locations-company",
                Area::Allowances => "allowance-company-form",
            },
            event: "select_company",
            change: true,
            body: html(&company_field)?,
            ..Default::default()
        })?;
        let empty_id = match (self.area, self.company.is_none()) {
            (Area::Rules, true) => "attendance-rules-empty",
            (Area::Rules, false) => "attendance-rules-unavailable",
            (Area::Shifts, true) => "attendance-shifts-no-company",
            (Area::Shifts, false) => "attendance-shifts-unavailable",
            (Area::Locations, true) => "attendance-locations-no-company",
            (Area::Locations, false) => "attendance-locations-unavailable",
            (Area::Allowances, true) => "allowances-no-company",
            (Area::Allowances, false) => "allowances-unavailable",
        };
        let empty = html(
            &EmptyState::missing(
                &if self.company.is_none() {
                    format!("No active company is available for {}.", self.area.noun())
                } else {
                    format!("{} are unavailable for this company.", self.area.title())
                },
                if self.company.is_some() {
                    "The company's workforce is not current."
                } else {
                    ""
                },
            )
            .id(empty_id),
        )?;
        let mut table = FlexTable::new(
            "attendance-settings-table",
            self.columns.flex_columns(),
            self.rows.clone(),
        );
        table.event = "grid";
        table.zoom = self.columns.zoom;
        table.suggestions = self.columns.flex_suggestions();
        table.add_query = &self.columns.add_query;
        table.caption = self.area.title();
        if self.rows.is_empty() {
            table.empty = Some(TableEmpty::State(EmptyState::missing(
                match self.area {
                    Area::Shifts => "No shift templates have been added for this company.",
                    Area::Locations => "No clocking locations have been added for this company.",
                    _ => "No allowance rules are configured.",
                },
                "",
            )));
        }
        let confirmation = if let Some(pending) = &self.pending {
            let detail = match pending {
                Pending::End(_, date) => {
                    format!("The effective end date will be {date}. This cannot be undone.")
                }
                Pending::Retire(_) => "This cannot be undone.".into(),
                _ => String::new(),
            };
            // ConfirmDialog has no banner slot; use System's authoritative notice in its detail.
            let detail = self.instance.as_ref().map_or(detail.clone(), |instance| {
                format!("{} {detail}", instance.notice())
            });
            let (consequence, confirm) = match pending {
                Pending::Status(_, s) if s == "active" => ("Reactivate this record?", "Reactivate"),
                Pending::End(_, _) => ("End this allowance rule on the selected date?", "End"),
                _ => ("Retire this record?", "Retire"),
            };
            html(&ConfirmDialog {
                id: "attendance-settings-confirm",
                consequence,
                detail: &detail,
                confirm,
                working: "Updating…",
                confirm_event: "confirm_change",
                cancel_event: "cancel_confirmation",
            })?
        } else {
            Html::EMPTY
        };
        View {
            flash,
            id: format!("attendance-{}-page", self.area.key()),
            title: if self.area == Area::Allowances { "Allowance rules" } else { "Attendance rules" },
            subtitle: match self.area {
                Area::Shifts => "Shift templates planners assign on the roster.",
                Area::Locations => "Places where clock events count as on site.",
                _ => "",
            },
            tabs,
            company_form,
            has_company: self.company.is_some(),
            available: self.available,
            empty,
            fields: self.fields(),
            is_rules: self.area == Area::Rules,
            is_allowances: self.area == Area::Allowances,
            section: self.area.title(),
            table: html(&table)?,
            submit: html(&Button {
                button_type: "submit",
                variant: if matches!(self.area, Area::Rules | Area::Allowances) { ButtonVariant::Primary } else { ButtonVariant::Secondary },
                body: Html::text(match self.area {
                    Area::Rules => "Save rules",
                    Area::Shifts => "Add shift template",
                    Area::Locations => "Add clocking location",
                    Area::Allowances => "Add rule",
                }),
                ..Default::default()
            })?,
            form_event: if self.area == Area::Rules { "save_rules" } else { "create" },
            form_id: match self.area {
                Area::Rules => "attendance-rules-form",
                Area::Shifts => "attendance-shift-form",
                Area::Locations => "attendance-location-form",
                Area::Allowances => "allowance-rule-form",
            },
            hint: match self.area {
                Area::Shifts => "An end at or before the start crosses midnight.",
                Area::Locations => if self.rules.as_ref().is_some_and(|r| r.location_required) {
                    "This company requires clock events to carry coordinates inside an active location."
                } else {
                    "Locations are recorded on clock events; the company does not require them."
                },
                Area::Allowances => "Use a company code and explicit currency. A later version ends the code's open version the day before it starts; active versions cannot overlap.",
                _ => "",
            },
            confirmation,
        }.render().map_err(|_| Error::Internal)
    }
}
impl LiveState<Scope, Error> for State {
    fn update<'a>(&'a mut self, r: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            let changed = match r.input {
                LiveInput::Params(query) => {
                    self.company = params::positive_id(query.get("company_id").map(String::as_str));
                    self.list = ListState::parse(query, spec()?).map_err(|_| Error::Internal)?;
                    self.pending = None;
                    self.form.clear();
                    true
                }
                LiveInput::Event(payload) => self.event(r.scope, payload).await?,
                LiveInput::Refresh => true,
            };
            self.load(r.scope, changed).await?;
            let mut query = self.list.to_params();
            if let Some(company) = self.company {
                query.insert("company_id".into(), company.to_string());
            }
            Ok(LiveBody {
                html: self.render()?,
                query,
                rows: self.rows.len(),
                ..Default::default()
            })
        })
    }
}
#[derive(Template)]
#[template(path = "settings.html")]
struct View<'a> {
    flash: FlashMap<'a>,
    id: String,
    title: &'static str,
    subtitle: &'static str,
    tabs: Html,
    company_form: Html,
    has_company: bool,
    available: bool,
    empty: Html,
    fields: Vec<Field>,
    is_rules: bool,
    is_allowances: bool,
    section: &'static str,
    table: Html,
    submit: Html,
    form_id: &'static str,
    form_event: &'static str,
    hint: &'static str,
    confirmation: Html,
}
