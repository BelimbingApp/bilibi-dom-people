//! Bilimbi Attendance.Web.RostersLive: seven-day drafts and company-wide publication.
use super::{refusal, render, row, submit, text, Core, Flash, Table};
use crate::{AttendanceError, Roster, RosterValue, ROSTER_MANAGE};
use askama::Template;
use bilibi_database::{Error, Result};
use bilibi_people_workforce::{authorization, CompanySummary};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    choices,
    page::{LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState},
    params, BuiltinColumn, Button, ButtonVariant, Choice, ConfirmDialog, EmptyState, Field, Filter,
    FlashMap, Form, Html, InputKind, ListSpec, ListState, RowMode,
};
use jiff::civil::Date;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

const NO_PERMISSION: &str = "You no longer have permission to plan rosters for this company.";
fn spec() -> Result<Arc<ListSpec>> {
    let mut spec = ListSpec {
        filters: BTreeMap::from([
            ("company_id".into(), Filter::string("")),
            ("from".into(), Filter::string("")),
        ]),
        omit_defaults: true,
        ..ListSpec::default()
    };
    spec.param_names.search = "q".into();
    spec.shared().map_err(|_| Error::Internal)
}
#[derive(Template)]
#[template(path = "rosters.html")]
struct View<'a> {
    flash: FlashMap<'a>,
    company: Html,
    state: Html,
    filter: Html,
    available: bool,
    pending: String,
    publish: Html,
    no_shifts: bool,
    shifts_link: Html,
    truncated: bool,
    table: Html,
    confirm: Html,
}
struct Publication {
    company: i64,
    from: Date,
}
pub(super) struct Rosters {
    core: Arc<Core>,
    list: ListState,
    companies: Vec<CompanySummary>,
    company: Option<CompanySummary>,
    roster: Option<Roster>,
    dates: Vec<String>,
    table: Table,
    pending: Option<Publication>,
    flash: Flash,
}
impl Rosters {
    pub(super) fn new(core: Arc<Core>, query: &HashMap<String, String>) -> Result<Self> {
        Ok(Self {
            core,
            list: ListState::parse(query, spec()?).map_err(|_| Error::Internal)?,
            companies: vec![],
            company: None,
            roster: None,
            dates: vec![],
            table: Table::new(
                "attendance-roster-grid",
                "roster_grid",
                &[("employee", "Employee")],
            )?,
            pending: None,
            flash: Flash::default(),
        })
    }
    fn company_id(&self) -> Option<i64> {
        params::positive_id(self.list.filters.get("company_id").map(String::as_str))
    }
    fn from(&self) -> Option<Date> {
        self.list.filters.get("from").and_then(|s| s.parse().ok())
    }
    pub(super) async fn load(&mut self, scope: &Scope, changed: bool) -> Result<()> {
        let client = self.core.db.get().await?;
        self.companies =
            match authorization::selectable_companies(&**client, scope, ROSTER_MANAGE).await {
                Ok(companies) => companies.into_iter().filter(|c| !c.archived).collect(),
                Err(error) => {
                    refusal(error.into())?;
                    vec![]
                }
            };
        if self.company_id().is_none() {
            if let Some(company) = self.companies.first() {
                self.list
                    .filters
                    .insert("company_id".into(), company.id.to_string());
            }
        }
        self.company = self
            .companies
            .iter()
            .find(|c| Some(c.id) == self.company_id())
            .cloned();
        self.roster = None;
        if let Some(company) = &self.company {
            let result = async {
                let rules = self.core.attendance.rules(scope, company.id).await?;
                let timezone =
                    jiff::tz::TimeZone::get(&rules.timezone).unwrap_or(jiff::tz::TimeZone::UTC);
                let from = self
                    .from()
                    .unwrap_or_else(|| jiff::Timestamp::now().to_zoned(timezone).date());
                let roster = self
                    .core
                    .attendance
                    .roster(scope, company.id, from, 7, Some(&self.list.search))
                    .await?;
                Ok::<_, AttendanceError>((from, roster))
            }
            .await;
            match result {
                Ok((from, roster)) => {
                    self.list.filters.insert("from".into(), from.to_string());
                    if self.dates != roster.dates {
                        self.dates = roster.dates.clone();
                        let mut builtins = vec![BuiltinColumn::new("employee", "Employee")];
                        for (index, day) in self.dates.iter().enumerate() {
                            let label = day
                                .parse::<Date>()
                                .map(|d| d.strftime("%a %d %b").to_string())
                                .unwrap_or_else(|_| day.clone());
                            builtins.push(BuiltinColumn::new(&format!("day-{index}"), &label));
                        }
                        let zoom = self.table.columns.zoom;
                        self.table.columns =
                            bilibi_ui::ListColumns::mount("attendance-roster-grid", builtins)
                                .map_err(|_| Error::Internal)?;
                        self.table.columns.zoom = zoom;
                    }
                    if changed {
                        self.table.load(&roster.employees, |e| e.id.to_string());
                    }
                    self.roster = Some(roster);
                }
                Err(error) => {
                    refusal(error)?;
                    self.pending = None;
                }
            }
        } else {
            self.pending = None;
        }
        Ok(())
    }
    async fn authorized(&mut self, scope: &Scope) -> Result<Option<i64>> {
        let Some(company) = self.company_id() else {
            self.flash.error(NO_PERMISSION);
            self.pending = None;
            return Ok(None);
        };
        let client = self.core.db.get().await?;
        match authorization::authorize_company(&**client, scope, company, ROSTER_MANAGE).await {
            Ok(c) if !c.archived => Ok(Some(company)),
            Ok(_) => {
                self.pending = None;
                self.flash.error("This company is archived and read-only");
                Ok(None)
            }
            Err(error) => {
                refusal(error.into())?;
                self.pending = None;
                self.flash.error(NO_PERMISSION);
                Ok(None)
            }
        }
    }
    async fn event(&mut self, scope: &Scope, payload: &Value) -> Result<bool> {
        match text(payload, "type").as_str() {
            "clear-flash" => self.flash.clear(payload),
            "roster_grid" => self.table.event(payload),
            "dismiss_publish" => self.pending = None,
            "select_company" => {
                self.load(scope, false).await?;
                let chosen = params::positive_id(payload.get("company_id").and_then(Value::as_str));
                if let Some(company) = self.companies.iter().find(|c| Some(c.id) == chosen) {
                    self.list
                        .filters
                        .insert("company_id".into(), company.id.to_string());
                    self.list.filters.insert("from".into(), String::new());
                    self.list.search.clear();
                    self.pending = None;
                }
                return Ok(true);
            }
            "filter" => {
                let mut posted = params::string_values(payload);
                if let Some(q) = posted.remove("q") {
                    posted.insert("search".into(), q);
                }
                // Bilimbi keeps the current period when an invalid date is submitted.
                if posted
                    .get("from")
                    .is_some_and(|s| s.parse::<Date>().is_err())
                {
                    posted.remove("from");
                }
                self.list.apply_filters(&posted);
                self.pending = None;
                return Ok(true);
            }
            "plan" => {
                let Some(company) = self.authorized(scope).await? else {
                    return Ok(true);
                };
                let employee =
                    params::positive_id(payload.get("employee_id").and_then(Value::as_str));
                let day = text(payload, "on_date").parse::<Date>();
                let value = text(payload, "value");
                let value = match value.as_str() {
                    "" => Some(RosterValue::None),
                    "rest" => Some(RosterValue::Rest),
                    value => value
                        .strip_prefix("shift:")
                        .and_then(|id| params::positive_id(Some(id)))
                        .map(RosterValue::Shift),
                };
                if let (Some(employee), Ok(day), Some(value)) = (employee, day, value) {
                    // A cell may only act on an employee and day in this mounted planner.
                    let visible = self.roster.as_ref().is_some_and(|r| {
                        r.employees.iter().any(|e| e.id == employee)
                            && r.dates.contains(&day.to_string())
                    });
                    if visible {
                        match self
                            .core
                            .attendance
                            .plan_roster_entry(scope, company, employee, day, value)
                            .await
                        {
                            Ok(_) => {}
                            Err(error) => self.flash.error(match refusal(error)? {
                                AttendanceError::ShiftUnavailable => {
                                    "That shift template is retired."
                                }
                                AttendanceError::Unauthorized => NO_PERMISSION,
                                AttendanceError::CompanyArchived => {
                                    "This company is archived and read-only"
                                }
                                _ => "The roster entry could not be saved.",
                            }),
                        }
                    } else {
                        self.flash.error("The roster entry could not be saved.");
                    }
                } else {
                    self.flash.error("The roster entry could not be saved.");
                }
                return Ok(true);
            }
            "publish" => {
                let Some(company) = self.authorized(scope).await? else {
                    return Ok(true);
                };
                self.load(scope, false).await?;
                if self.roster.as_ref().is_some_and(|r| r.pending > 0) {
                    if let Some(from) = self.from() {
                        self.pending = Some(Publication { company, from });
                    }
                }
                return Ok(true);
            }
            "confirm_publish" => {
                let Some(company) = self.authorized(scope).await? else {
                    return Ok(true);
                };
                if let Some(pending) = self.pending.take() {
                    if pending.company == company && Some(pending.from) == self.from() {
                        let to = pending
                            .from
                            .checked_add(jiff::Span::new().days(6))
                            .map_err(|_| Error::Invalid)?;
                        match self
                            .core
                            .attendance
                            .publish_roster(scope, company, pending.from, to)
                            .await
                        {
                            Ok(count) => self.flash.success(&format!(
                                "Published {count} roster {}.",
                                if count == 1 { "change" } else { "changes" }
                            )),
                            Err(error) => self.flash.error(match refusal(error)? {
                                AttendanceError::Unauthorized => NO_PERMISSION,
                                AttendanceError::CompanyArchived => {
                                    "This company is archived and read-only"
                                }
                                _ => "The roster could not be published.",
                            }),
                        }
                    }
                }
                return Ok(true);
            }
            _ => return Err(Error::Invalid),
        }
        Ok(false)
    }
    pub(super) fn render(&self) -> Result<String> {
        let mut view = View {
            flash: self.flash.map(),
            company: Html::EMPTY,
            state: Html::EMPTY,
            filter: Html::EMPTY,
            available: self.roster.is_some(),
            pending: String::new(),
            publish: Html::EMPTY,
            no_shifts: false,
            shifts_link: Html::EMPTY,
            truncated: false,
            table: Html::EMPTY,
            confirm: Html::EMPTY,
        };
        if !self.companies.is_empty() {
            let options: Vec<_> = self
                .companies
                .iter()
                .map(|c| (c.name.clone(), c.id.to_string()))
                .collect();
            view.company = render(&Form {
                id: "attendance-rosters-company-form",
                event: "select_company",
                change: true,
                class: "mt-5",
                body: render(
                    &Field::named("company_id")
                        .id("attendance-rosters-company")
                        .wrapper_class("max-w-md")
                        .label("Company")
                        .kind(InputKind::Select)
                        .options(choices(&options))
                        .value(
                            self.company_id()
                                .map_or_else(String::new, |id| id.to_string()),
                        ),
                )?,
                actions: Html::EMPTY,
            })?;
        }
        let Some(roster) = &self.roster else {
            view.state = render(&if self.companies.is_empty() {
                EmptyState::missing("No active company is available for rosters.", "")
                    .id("attendance-rosters-no-company")
            } else {
                EmptyState::missing(
                    "The roster is unavailable for this company.",
                    "The company's workforce is not current.",
                )
                .id("attendance-rosters-unavailable")
            })?;
            return view.render().map_err(|_| Error::Internal);
        };
        let mut fields = render(
            &Field::named("from")
                .label("Week from")
                .kind(InputKind::Date)
                .value(self.from().map_or_else(String::new, |d| d.to_string()))
                .wrapper_class("mb-0"),
        )?;
        fields.push(&render(
            &Field::named("q")
                .label("Search employees")
                .placeholder("Name or number")
                .value(&self.list.search)
                .wrapper_class("mb-0"),
        )?);
        let mut actions = submit("Show")?;
        for (offset, id, label) in [
            (-7, "roster-previous-week", "Previous week"),
            (7, "roster-next-week", "Next week"),
        ] {
            if let Some(day) = self
                .from()
                .and_then(|d| d.checked_add(jiff::Span::new().days(offset)).ok())
            {
                let mut query = self.list.to_params();
                query.insert("from".into(), day.to_string());
                actions.push(&render(&Button {
                    id,
                    href: &params::address("/people/attendance/rosters", &query),
                    body: Html::text(label),
                    ..Button::default()
                })?);
            }
        }
        view.filter = render(&Form {
            id: "attendance-roster-filter",
            event: "filter",
            change: false,
            class: "flex flex-wrap items-end gap-2",
            body: fields,
            actions,
        })?;
        view.pending = if roster.pending == 0 {
            "No unpublished changes this week for this company.".into()
        } else {
            format!("{} unpublished {} this week across the company. Publish week releases all of them, including employees outside this list.", roster.pending, if roster.pending == 1 { "change" } else { "changes" })
        };
        if roster.pending > 0 {
            view.publish = render(&Button {
                id: "attendance-roster-publish",
                event: "publish",
                variant: ButtonVariant::Primary,
                body: Html::text("Publish week"),
                ..Button::default()
            })?;
        }
        view.no_shifts = roster.templates.iter().all(|t| t.status != "active");
        view.shifts_link = render(&Button {
            href: &format!(
                "/people/attendance/rules/shifts?company_id={}",
                self.company_id().unwrap_or_default()
            ),
            body: Html::text("Add shift templates"),
            ..Button::default()
        })?;
        view.truncated = roster.truncated;
        let mut rows = vec![];
        for employee in &roster.employees {
            let mut r = row("roster-row", employee.id).slot(
                "employee",
                Html::text(&format!("{} {}", employee.name, employee.number)),
            );
            for (index, day) in roster.dates.iter().enumerate() {
                let entry = roster
                    .entries
                    .iter()
                    .find(|e| e.employee_id == employee.id && &e.on_date == day);
                let current = entry.and_then(|e| e.shift_template_id);
                let value = match entry {
                    Some(e) if e.kind == "shift" => {
                        format!("shift:{}", e.shift_template_id.unwrap_or_default())
                    }
                    Some(e) if e.kind == "rest" => "rest".into(),
                    _ => String::new(),
                };
                let mut options = vec![
                    Choice::new("Not rostered", ""),
                    Choice::new("Rest day", "rest"),
                ];
                for template in roster
                    .templates
                    .iter()
                    .filter(|t| t.status == "active" || Some(t.id) == current)
                {
                    options.push(Choice::new(
                        format!(
                            "{} {:02}:{:02}–{:02}:{:02}{}",
                            template.code,
                            template.start_minute / 60,
                            template.start_minute % 60,
                            template.end_minute / 60,
                            template.end_minute % 60,
                            if template.status == "active" {
                                ""
                            } else {
                                " (retired)"
                            }
                        ),
                        format!("shift:{}", template.id),
                    ));
                }
                let mut fields = render(
                    &Field::named("employee_id")
                        .kind(InputKind::Hidden)
                        .value(employee.id.to_string()),
                )?;
                fields.push(&render(
                    &Field::named("on_date").kind(InputKind::Hidden).value(day),
                )?);
                let mut field = Field::named("value")
                    .id(format!("roster-value-{}-{day}", employee.id))
                    .kind(InputKind::Select)
                    .aria_label(format!("{} on {day}", employee.name))
                    .options(options)
                    .value(value)
                    .wrapper_class("mb-0");
                if entry.is_some_and(|e| e.pending())
                    && self.table.columns.mode() != RowMode::Compact
                {
                    field = field.hint("Draft");
                }
                fields.push(&render(&field)?);
                let cell = render(&Form {
                    id: &format!("roster-cell-{}-{day}", employee.id),
                    event: "plan",
                    change: true,
                    class: "block min-w-40",
                    body: fields,
                    actions: Html::EMPTY,
                })?;
                r = r.slot(&format!("day-{index}"), cell);
            }
            rows.push(r);
        }
        view.table = self.table.render(
            "Roster for the week",
            rows,
            EmptyState::missing(
                if self.list.search.is_empty() {
                    "No working employees in this company."
                } else {
                    "No employees match this search."
                },
                "",
            )
            .id("attendance-roster-empty"),
        )?;
        if let Some(pending) = &self.pending {
            view.confirm = render(&ConfirmDialog {
                id: "attendance-roster-publish-confirm",
                consequence: "Publish this week to employees?",
                detail: &format!(
                    "{} · Week from {}. {}",
                    self.company.as_ref().map_or("", |c| c.name.as_str()),
                    pending.from,
                    view.pending
                ),
                confirm: "Publish week",
                working: "Publishing…",
                confirm_event: "confirm_publish",
                cancel_event: "dismiss_publish",
            })?;
        }
        view.render().map_err(|_| Error::Internal)
    }
}
impl LiveState<Scope, Error> for Rosters {
    fn update<'a>(&'a mut self, request: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            let changed = match request.input {
                LiveInput::Event(payload) => self.event(request.scope, payload).await?,
                LiveInput::Params(query) => {
                    self.list = ListState::parse(query, spec()?).map_err(|_| Error::Internal)?;
                    self.pending = None;
                    true
                }
                LiveInput::Refresh => true,
            };
            self.load(request.scope, changed).await?;
            Ok(LiveBody {
                html: self.render()?,
                rows: self.roster.as_ref().map_or(0, |r| r.employees.len()),
                query: self.list.to_params(),
                ..LiveBody::default()
            })
        })
    }
}
