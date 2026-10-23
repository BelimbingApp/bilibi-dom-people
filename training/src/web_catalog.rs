//! Bilimbi Training.Web.CoursesLive and SessionsLive: catalog and company calendar.
use crate::*;
use askama::Template;
use bilibi_database::{Error, Result as DbResult};
use bilibi_menu::{MenuItem, Requirement};
use bilibi_module_registry::{
    contributions::{Access, Method, PageWrites, Pending, Route, PAGE_WRITES, ROUTES},
    Contribution, Provenance,
};
use bilibi_ui::{
    page::{
        LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState, MountFuture, Page, PageBody,
        PageFuture, PageRequest,
    },
    params, BuiltinColumn, Button, Cell, Choice, CommitStates, CommitStatus, ConfirmDialog,
    EmptyState, ErrorSummary, Field, Filter, FilterToolbar, FlashMap, FlexRow, FlexTable, Html,
    InlineEdit, InlineField, InlineLongText, InputKind, ListColumns, ListSpec, ListState, Modal,
    Pagination, StatusKind, TableEmpty, ToolbarControl, NO_FLASH,
};
use jiff::tz::TimeZone;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

pub(crate) const PAGES: &str = "people/training.catalog_pages";
const SOURCE: Provenance = Provenance {
    module: MODULE,
    source: file!(),
    line: line!(),
};
inventory::submit! { Contribution::new(PAGE_WRITES, PAGES, SOURCE, &PageWrites { service: PAGES, contracts: &[], pending: Pending::Every { reason: "Course and session catalog writes use the Training facade; operation contracts await review." } }) }
macro_rules! route {
    ($key:literal, $cap:expr) => { inventory::submit! { Contribution::new(ROUTES, concat!("people/training.", $key), SOURCE, &Route { method: Method::Get, path: concat!("/people/training/", $key), service: PAGES, operation: $key, access: Access::Capability($cap), framed: true }) } };
}
route!("courses", COURSES_VIEW);
route!("sessions", SESSIONS_VIEW);
bilibi_menu::contribute!(
    "people/training",
    MenuItem::page(
        "people.development.courses",
        "Courses",
        "/people/training/courses"
    )
    .icon("building-library")
    .under("people.development")
    .requires(Requirement::Capability(COURSES_VIEW))
    .order(60)
);
bilibi_menu::contribute!(
    "people/training",
    MenuItem::page(
        "people.development.sessions",
        "Sessions & calendar",
        "/people/training/sessions"
    )
    .icon("squares-2x2")
    .under("people.development")
    .requires(Requirement::Capability(SESSIONS_VIEW))
    .order(70)
);

pub(crate) struct Pages(pub Arc<Training>);
impl Page<Scope, Error> for Pages {
    fn title(&self, operation: &str) -> Option<&'static str> {
        match operation {
            "courses" => Some("Courses"),
            "sessions" => Some("Sessions & calendar"),
            _ => None,
        }
    }
    fn render<'a>(&'a self, req: PageRequest<'a, Scope>) -> PageFuture<'a, Error> {
        Box::pin(async move {
            let mut state = Catalog::new(self.0.clone(), req.operation, req.query)?;
            state.load(req.scope.ok_or(Error::Unauthorized)?).await?;
            Ok(PageBody {
                html: state.render()?,
                live: true,
                title: None,
            })
        })
    }
    fn mount<'a>(&'a self, req: PageRequest<'a, Scope>) -> MountFuture<'a, Scope, Error> {
        Box::pin(async move {
            Ok(Some(
                Box::new(Catalog::new(self.0.clone(), req.operation, req.query)?)
                    as Box<dyn LiveState<Scope, Error>>,
            ))
        })
    }
}
fn spec(page: &str, sessions: bool) -> DbResult<Arc<ListSpec>> {
    let mut filters = BTreeMap::from([("company_id".into(), Filter::string(""))]);
    if sessions {
        filters.insert("date".into(), Filter::string(""));
        filters.insert("view".into(), Filter::one_of(&["list", "calendar"], "list"));
    }
    let mut spec = ListSpec {
        filters,
        page_size_param: "perPage".into(),
        omit_blank: vec!["company_id".into()],
        ..ListSpec::default()
    };
    spec.param_names.page = page.into();
    spec.shared().map_err(|_| Error::Internal)
}
fn columns(id: &str, labels: &[(&str, &str)]) -> DbResult<ListColumns> {
    ListColumns::mount(
        id,
        labels
            .iter()
            .map(|(key, label)| BuiltinColumn::new(key, label))
            .collect(),
    )
    .map_err(|_| Error::Internal)
}
struct Catalog {
    training: Arc<Training>,
    courses_page: bool,
    list: ListState,
    session_list: ListState,
    columns: ListColumns,
    session_columns: ListColumns,
    companies: Vec<bilibi_people_workforce::CompanySummary>,
    company: Option<i64>,
    courses: Vec<Course>,
    choices: Vec<CourseOption>,
    events: Vec<Event>,
    sessions: Vec<Session>,
    month: Date,
    zone: String,
    clock: Option<bilibi_datetime::DisplayContext>,
    error: Option<String>,
    can_manage: bool,
    flash: Option<(StatusKind, String)>,
    modal: Option<&'static str>,
    draft: HashMap<String, String>,
    summary: ErrorSummary,
    commits: CommitStates,
}
impl Catalog {
    fn new(
        training: Arc<Training>,
        operation: &str,
        query: &HashMap<String, String>,
    ) -> DbResult<Self> {
        let courses_page = match operation {
            "courses" => true,
            "sessions" => false,
            _ => return Err(Error::NotFound),
        };
        Ok(Self {
            training,
            courses_page,
            list: ListState::parse(
                query,
                spec(
                    if courses_page { "page" } else { "event_page" },
                    !courses_page,
                )?,
            )
            .map_err(|_| Error::Internal)?,
            session_list: ListState::parse(query, spec("session_page", true)?)
                .map_err(|_| Error::Internal)?,
            columns: if courses_page {
                columns(
                    "courses",
                    &[
                        ("course", "Course"),
                        ("code", "Code"),
                        ("description", "Description"),
                        ("status", "Status"),
                        ("actions", "Actions"),
                    ],
                )?
            } else {
                columns(
                    "training-events",
                    &[
                        ("event", "Event"),
                        ("course", "Course"),
                        ("capacity", "Capacity"),
                    ],
                )?
            },
            session_columns: columns(
                "training-sessions",
                &[
                    ("session", "Session"),
                    ("event", "Event"),
                    ("start", "Start"),
                    ("end", "End"),
                    ("zone", "Delivery time zone"),
                    ("capacity", "Capacity"),
                ],
            )?,
            companies: vec![],
            company: None,
            courses: vec![],
            choices: vec![],
            events: vec![],
            sessions: vec![],
            month: jiff::Timestamp::now()
                .to_zoned(TimeZone::UTC)
                .date()
                .first_of_month(),
            zone: "UTC".into(),
            clock: None,
            error: None,
            can_manage: false,
            flash: None,
            modal: None,
            draft: HashMap::new(),
            summary: ErrorSummary::new("catalog-errors"),
            commits: CommitStates::default(),
        })
    }
    fn view_cap(&self) -> &'static str {
        if self.courses_page {
            COURSES_VIEW
        } else {
            SESSIONS_VIEW
        }
    }
    fn write_cap(&self) -> &'static str {
        if self.courses_page {
            COURSES_MANAGE
        } else {
            SESSIONS_MANAGE
        }
    }
    fn denied(&self) -> &'static str {
        if self.courses_page {
            "You cannot change this company's courses."
        } else {
            "You cannot change this company's sessions."
        }
    }
    fn title(&self) -> &'static str {
        if self.courses_page {
            "Courses"
        } else {
            "Sessions & calendar"
        }
    }
    fn fail(&mut self, text: impl Into<String>) {
        self.flash = Some((StatusKind::Error, text.into()));
    }
    async fn load(&mut self, scope: &Scope) -> DbResult<()> {
        let client = self.training.db.get().await?;
        self.companies = authorization::selectable_companies(&**client, scope, self.view_cap())
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|c| !c.archived)
            .collect();
        let selected = self
            .list
            .filters
            .get("company_id")
            .filter(|s| !s.is_empty());
        self.company = match selected {
            Some(value) => params::positive_id(Some(value))
                .filter(|id| self.companies.iter().any(|c| c.id == *id)),
            None => self.companies.first().map(|c| c.id),
        };
        self.courses.clear();
        self.events.clear();
        self.sessions.clear();
        self.choices.clear();
        self.error = None;
        self.can_manage = false;
        let Some(company) = self.company else {
            self.error = Some(if self.courses_page {
                "No company is available. Ask an operator to check your company access.".into()
            } else {
                TrainingError::Unauthorized.to_string()
            });
            self.modal = None;
            return Ok(());
        };
        // Bind the first company to the URL once; lost reach never retargets an open form.
        self.list
            .filters
            .insert("company_id".into(), company.to_string());
        self.can_manage = self
            .training
            .allowed(scope, company, self.write_cap())
            .await;
        self.clock = Some(
            bilibi_datetime::DisplayPolicy::new(self.training.settings.clone())
                .display(
                    &**client,
                    scope,
                    Some(OrganisationId::new(company).map_err(|_| Error::Invalid)?),
                    "en",
                )
                .await?,
        );
        drop(client);
        let result = if self.courses_page {
            self.training
                .courses(scope, company)
                .await
                .map(|rows| self.courses = rows)
        } else {
            self.load_sessions(scope, company).await
        };
        if let Err(error) = result {
            self.error = Some(error.to_string());
        }
        if self.error.is_some() || !self.can_manage {
            self.can_manage = false;
            self.modal = None;
        }
        if self.courses_page {
            self.columns
                .set_available(&[bilibi_ui::RecordField::new("actions", "Actions")], |_| {
                    self.can_manage
                });
            self.list.paginate(&self.courses);
        }
        if !self.courses_page {
            self.list.paginate(&self.events);
            self.session_list.paginate(&self.sessions);
        }
        Ok(())
    }
    async fn load_sessions(&mut self, scope: &Scope, company: i64) -> crate::Result<()> {
        self.month = self
            .list
            .filters
            .get("date")
            .and_then(|s| s.parse::<Date>().ok())
            .unwrap_or_else(|| Timestamp::now().to_zoned(TimeZone::UTC).date())
            .first_of_month();
        self.list
            .filters
            .insert("date".into(), self.month.to_string());
        let until = self
            .month
            .last_of_month()
            .checked_add(jiff::Span::new().days(1))
            .map_err(|_| TrainingError::InvalidCalendarRange)?;
        let (from, until, zone) = self
            .training
            .company_day_bounds(scope, company, self.month, until)
            .await?;
        // The bounds and grid must use the same zone if settings change mid-read.
        self.zone = zone;
        self.sessions = self.training.calendar(scope, company, from, until).await?;
        self.events = self.training.events(scope, company).await?;
        if self.can_manage {
            self.choices = self.training.event_courses(scope, company).await?;
        }
        self.session_list.filters.clone_from(&self.list.filters);
        Ok(())
    }
    async fn event(&mut self, scope: &Scope, payload: &Value) -> DbResult<()> {
        let event = posted(payload, "type");
        match event.as_str() {
            "clear-flash" => {
                if self
                    .flash
                    .as_ref()
                    .is_some_and(|(kind, _)| kind.name() == posted(payload, "key"))
                {
                    self.flash = None;
                }
                return Ok(());
            }
            "grid" => {
                self.columns.handle(&params::string_values(payload));
                return Ok(());
            }
            "session-grid" => {
                self.session_columns.handle(&params::string_values(payload));
                return Ok(());
            }
            "filter" | "select_company" | "filters" => {
                let values = params::form_values(payload, "filters");
                self.list.apply_filters(&values);
                self.session_list.apply_filters(&values);
                self.modal = None;
                self.draft.clear();
                return Ok(());
            }
            "page" | "event_page" => {
                self.list
                    .put_page(payload.get("page").and_then(Value::as_str));
                return Ok(());
            }
            "session_page" => {
                self.session_list
                    .put_page(payload.get("page").and_then(Value::as_str));
                return Ok(());
            }
            "close_modal" => {
                self.modal = None;
                return Ok(());
            }
            _ => {}
        }
        let Some(company) = self.company else {
            self.fail(self.denied());
            return Ok(());
        };
        // Check every write-shaped event before decoding a form or selecting a record.
        if !self.can_manage
            || !self
                .training
                .allowed(scope, company, self.write_cap())
                .await
        {
            self.commits.clear_saved();
            self.fail(self.denied());
            return Ok(());
        }
        match event.as_str() {
            "open_course" if self.courses_page => {
                self.open("course");
            }
            "open_event" if !self.courses_page && !self.choices.is_empty() => {
                self.open("event");
            }
            "open_session" if !self.courses_page && !self.events.is_empty() => {
                self.open("session");
                self.draft.insert("time_zone".into(), self.zone.clone());
            }
            "create_course" if self.courses_page && self.modal == Some("course") => {
                self.draft = entry_values(payload, "course");
                let result = self
                    .training
                    .create_course(
                        scope,
                        company,
                        &NewCourse {
                            code: self.value("code"),
                            name: self.value("name"),
                            description: Some(self.value("description")),
                        },
                    )
                    .await;
                self.finish(result, "Course added.");
            }
            "create_event" if !self.courses_page && self.modal == Some("event") => {
                self.draft = entry_values(payload, "entry");
                let result = self
                    .training
                    .create_event(
                        scope,
                        company,
                        &NewEvent {
                            course_id: self.integer("course_id"),
                            name: self.value("name"),
                            capacity: self.integer("capacity"),
                        },
                    )
                    .await;
                self.finish(result, "Event added.");
            }
            "create_session" if !self.courses_page && self.modal == Some("session") => {
                self.draft = entry_values(payload, "entry");
                let result = self
                    .training
                    .create_session(
                        scope,
                        company,
                        &NewSession {
                            event_id: self.integer("event_id"),
                            name: self.value("name"),
                            capacity: self.integer("capacity"),
                            time_zone: self.value("time_zone"),
                            starts_local: self.value("starts_local"),
                            ends_local: self.value("ends_local"),
                        },
                    )
                    .await;
                self.finish(result, "Session added.");
            }
            "save_course_field" if self.courses_page => {
                let id = params::positive_id(payload.get("id").and_then(Value::as_str));
                let field = posted(payload, "field");
                let value = posted(payload, "value");
                if let Some(id) = id.filter(|id| self.courses.iter().any(|c| c.id == *id)) {
                    let update = match field.as_str() {
                        "name" => CourseUpdate {
                            name: Some(value.clone()),
                            ..CourseUpdate::default()
                        },
                        "description" => CourseUpdate {
                            description: Some(value.clone()),
                            ..CourseUpdate::default()
                        },
                        _ => return Ok(()),
                    };
                    let outcome = match self
                        .training
                        .update_course_versioned(
                            scope,
                            company,
                            id,
                            &update,
                            &posted(payload, "version"),
                        )
                        .await
                    {
                        Ok(_) => CommitStatus::Saved,
                        Err(TrainingError::Invalid(errors)) => {
                            let reasons = errors
                                .iter()
                                .filter(|e| e.field == field)
                                .map(|e| e.message.as_str())
                                .collect::<Vec<_>>();
                            CommitStatus::refusal(
                                if field == "name" {
                                    "Course name"
                                } else {
                                    "Course description"
                                },
                                &value,
                                &reasons,
                            )
                        }
                        Err(e) => CommitStatus::Error(e.to_string()),
                    };
                    self.commits.put(format!("{id}:{field}"), outcome);
                }
            }
            "set_active" if self.courses_page => {
                if let Some((id, active)) = posted(payload, "value").split_once(':') {
                    let id = params::positive_id(Some(id));
                    let active = match active {
                        "true" => Some(true),
                        "false" => Some(false),
                        _ => None,
                    };
                    if let (Some(id), Some(active)) = (id, active) {
                        if self.courses.iter().any(|c| c.id == id) {
                            let result = self
                                .training
                                .set_course_active(scope, company, id, active)
                                .await;
                            self.finish(result, "Course updated.");
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn open(&mut self, kind: &'static str) {
        self.modal = Some(kind);
        self.draft.clear();
        self.flash = None;
        self.summary = ErrorSummary::new("catalog-errors");
    }
    fn value(&self, key: &str) -> String {
        self.draft.get(key).cloned().unwrap_or_default()
    }
    fn integer(&self, key: &str) -> i64 {
        self.value(key).parse().unwrap_or(0)
    }
    fn finish<T>(&mut self, result: crate::Result<T>, message: &str) {
        match result {
            Ok(_) => {
                self.modal = None;
                self.flash = Some((StatusKind::Success, message.into()));
                self.summary = ErrorSummary::new("catalog-errors");
            }
            Err(error) => {
                self.summary = ErrorSummary::new("catalog-errors");
                if let TrainingError::Invalid(errors) = &error {
                    for e in errors {
                        self.summary.push(
                            format!(
                                "{}_{}",
                                if self.modal == Some("course") {
                                    "course"
                                } else {
                                    "entry"
                                },
                                e.field
                            ),
                            format!("{} {}", e.field, e.message),
                        );
                    }
                }
                self.fail(error.to_string());
            }
        }
    }
    fn flash_map(&self) -> FlashMap<'_> {
        let mut f = NO_FLASH;
        if let Some((kind, text)) = &self.flash {
            match kind {
                StatusKind::Success => f.success = text,
                StatusKind::Error => f.error = text,
                StatusKind::Warning => f.warning = text,
                StatusKind::Info => f.info = text,
            }
        }
        f
    }
    fn course_rows(&self) -> DbResult<Vec<FlexRow>> {
        self.list
            .clone()
            .paginate(&self.courses)
            .entries
            .iter()
            .map(|c| {
                let inline = |name: &str, label: &str, value: &str| {
                    let mut f =
                        InlineField::new(format!("course-{name}-{}", c.id), name, label, value)
                            .status(self.commits.get(&format!("{}:{name}", c.id)));
                    f.id_value = c.id.to_string();
                    f.version = c.edit_version();
                    f.save_event = "save_course_field".into();
                    f.editable = self.can_manage;
                    if name == "description" {
                        f.allow_empty = true;
                    }
                    f
                };
                let name = render(&InlineEdit {
                    field: inline("name", "Course name", &c.name),
                })?;
                let description = render(&InlineLongText {
                    field: inline(
                        "description",
                        "Course description",
                        c.description.as_deref().unwrap_or(""),
                    ),
                    rows: 3,
                })?;
                let mut row = FlexRow::new(c.id.to_string())
                    .slot("course", name)
                    .cell("code", Cell::text(&c.code))
                    .cell(
                        "status",
                        Cell::text(if c.active { "Active" } else { "Inactive" }),
                    );
                row = if self.columns.mode() == bilibi_ui::RowMode::Compact {
                    row.cell(
                        "description",
                        Cell::text(
                            c.description
                                .as_deref()
                                .unwrap_or("")
                                .split_whitespace()
                                .collect::<Vec<_>>()
                                .join(" "),
                        ),
                    )
                } else {
                    row.slot("description", description)
                };
                row.dom_id = Some(format!("course-{}", c.id));
                if self.can_manage {
                    row = row.slot(
                        "actions",
                        render(&Button {
                            event: "set_active",
                            button_type: "button",
                            name: "value",
                            value: &format!("{}:{}", c.id, !c.active),
                            working: "Saving…",
                            body: Html::text(if c.active { "Deactivate" } else { "Reactivate" }),
                            ..Button::default()
                        })?,
                    );
                }
                Ok(row)
            })
            .collect()
    }
    fn time(
        &self,
        id: &str,
        instant: Timestamp,
        format: bilibi_datetime::Format,
        company: bool,
    ) -> Html {
        let Some(clock) = &self.clock else {
            return Html::EMPTY;
        };
        let mut clock = clock.clone();
        if company {
            clock.mode = bilibi_datetime::Mode::Company;
            clock.timezone.clone_from(&self.zone);
        }
        let value = clock.render(instant, format, bilibi_datetime::Precision::Minute);
        if company {
            return Html::text(&value.text);
        }
        // The shared instant follows the saved clock in the session list.
        render(&TimeView {
            id,
            instant: bilibi_ui::Instant::new(
                &value.iso,
                &value.text,
                &value.company,
                &value.utc,
                value.mode.as_str(),
            ),
        })
        .unwrap_or(Html::EMPTY)
    }
    fn session_rows(&self) -> DbResult<Vec<FlexRow>> {
        Ok(self
            .session_list
            .clone()
            .paginate(&self.sessions)
            .entries
            .iter()
            .map(|s| {
                let mut row = FlexRow::new(s.id.to_string())
                    .cell("session", Cell::text(&s.name))
                    .cell(
                        "event",
                        Cell::text(
                            self.events
                                .iter()
                                .find(|e| e.id == s.event_id)
                                .map_or("", |e| e.name.as_str()),
                        ),
                    )
                    .slot(
                        "start",
                        self.time(
                            &format!("session-start-{}", s.id),
                            s.starts_at,
                            bilibi_datetime::Format::DateTime,
                            false,
                        ),
                    )
                    .slot(
                        "end",
                        self.time(
                            &format!("session-end-{}", s.id),
                            s.ends_at,
                            bilibi_datetime::Format::DateTime,
                            false,
                        ),
                    )
                    .cell("zone", Cell::text(&s.time_zone))
                    .cell("capacity", Cell::text(s.capacity.to_string()));
                row.dom_id = Some(format!("session-{}", s.id));
                row
            })
            .collect())
    }
    fn table(
        &self,
        id: &str,
        columns: &ListColumns,
        rows: Vec<FlexRow>,
        empty: Option<EmptyState>,
        event: &str,
    ) -> DbResult<Html> {
        let mut table = FlexTable::new(id, columns.flex_columns(), rows);
        table.event = event;
        table.zoom = columns.zoom;
        table.suggestions = columns.flex_suggestions();
        table.add_query = &columns.add_query;
        table.empty = empty.map(TableEmpty::State);
        render(&table)
    }
    fn pager(&self, id: &str, list: &ListState, total: usize, event: &str) -> DbResult<Html> {
        let mut pager = Pagination::new(
            id,
            bilibi_ui::PageCounts::new(list.page, list.page_size, total as u64),
            &[25, 50, 100, 300],
        );
        pager.page_event = event;
        render(&pager)
    }
    fn dialog(&self) -> DbResult<Html> {
        let Some(kind) = self.modal else {
            return Ok(Html::EMPTY);
        };
        let mut fields = Vec::new();
        let field = |key: &str, label: &str| {
            Field::in_form(if kind == "course" { "course" } else { "entry" }, key)
                .id(format!(
                    "{}_{key}",
                    if kind == "course" { "course" } else { "entry" }
                ))
                .label(label)
                .value(self.value(key))
        };
        if kind == "course" {
            fields.push(field("code", "Code").required().attr("maxlength", "80"));
        }
        if kind == "event" {
            fields.push(
                field("course_id", "Course")
                    .kind(InputKind::Select)
                    .required()
                    .options(
                        self.choices
                            .iter()
                            .map(|c| Choice::new(&c.name, c.id.to_string()))
                            .collect(),
                    ),
            );
        }
        if kind == "session" {
            fields.push(
                field("event_id", "Event")
                    .kind(InputKind::Select)
                    .required()
                    .options(
                        self.events
                            .iter()
                            .map(|e| Choice::new(&e.name, e.id.to_string()))
                            .collect(),
                    ),
            );
        }
        fields.push(field("name", "Name").required().attr("maxlength", "160"));
        if kind == "course" {
            fields.push(
                field("description", "Description")
                    .kind(InputKind::Textarea)
                    .attr("maxlength", "4000"),
            );
        } else {
            fields.push(
                field("capacity", "Capacity")
                    .kind(InputKind::Number)
                    .required()
                    .attr("min", "1"),
            );
        }
        if kind == "session" {
            fields.push(field("time_zone", "IANA time zone").required());
            fields.push(
                field("starts_local", "Local start")
                    .kind(InputKind::DatetimeLocal)
                    .required(),
            );
            fields.push(
                field("ends_local", "Local end")
                    .kind(InputKind::DatetimeLocal)
                    .required(),
            );
        }
        let event = format!("create_{kind}");
        let body = render(&Entry {
            id: if kind == "course" {
                "course-form"
            } else {
                "training-entry-form"
            },
            fields,
            event: &event,
            summary: &self.summary,
            actions: [
                button("close_modal", "Cancel", "")?,
                render(&Button {
                    button_type: "submit",
                    working: "Adding…",
                    body: Html::text(if kind == "course" {
                        "Add course"
                    } else {
                        "Add"
                    }),
                    ..Button::default()
                })?,
            ]
            .into_iter()
            .collect(),
        })?;
        render(&Modal {
            id: if kind == "course" {
                "course-modal"
            } else {
                "session-modal"
            },
            title: if kind == "course" {
                "Add course"
            } else if kind == "event" {
                "Add event"
            } else {
                "Add session"
            },
            cancel_event: "close_modal",
            flash: Some(self.flash_map()),
            body,
            ..Modal::default()
        })
    }
    fn calendar(&self) -> DbResult<Html> {
        let zone = TimeZone::get(&self.zone).map_err(|_| Error::Invalid)?;
        let mut days = vec![];
        for n in 0..self.month.days_in_month() {
            let day = self
                .month
                .checked_add(jiff::Span::new().days(i64::from(n)))
                .map_err(|_| Error::Invalid)?;
            let entries = self
                .sessions
                .iter()
                .filter(|s| {
                    let start = s.starts_at.to_zoned(zone.clone()).date();
                    let finish = s
                        .ends_at
                        .checked_sub(jiff::SignedDuration::from_secs(1))
                        .unwrap_or(s.ends_at)
                        .to_zoned(zone.clone())
                        .date();
                    start <= day && day <= finish
                })
                .map(|s| CalendarEntry {
                    name: s.name.clone(),
                    time: self.time("", s.starts_at, bilibi_datetime::Format::Time, true),
                    capacity: s.capacity,
                })
                .collect();
            days.push(CalendarDay {
                id: day.to_string(),
                label: day.strftime("%a %d").to_string(),
                entries,
            });
        }
        render(&Calendar { days })
    }
    fn render(&self) -> DbResult<String> {
        let mut controls = vec![ToolbarControl::select(
            if self.courses_page {
                "course-company"
            } else {
                "session-company"
            },
            "company_id",
            "Company",
            &self.company.map(|id| id.to_string()).unwrap_or_default(),
            self.companies
                .iter()
                .map(|c| Choice::new(&c.name, c.id.to_string()))
                .collect(),
        )];
        if !self.courses_page {
            controls.push(ToolbarControl::date(
                "session-month",
                "date",
                "Month containing date",
                &self.month.to_string(),
            ));
            controls.push(ToolbarControl::select(
                "session-view",
                "view",
                "View",
                self.list.filters.get("view").map_or("list", String::as_str),
                vec![
                    Choice::new("List", "list"),
                    Choice::new("Calendar", "calendar"),
                ],
            ));
        }
        let toolbar = render(&FilterToolbar {
            id: if self.courses_page {
                "course-filters"
            } else {
                "session-filters"
            },
            event: if self.courses_page {
                "select_company"
            } else {
                "filter"
            },
            controls,
            class: "",
            target: "",
        })?;
        let mut actions = Html::EMPTY;
        if self.can_manage {
            if self.courses_page {
                actions.push(&button("open_course", "Add course", "")?);
            } else {
                if !self.choices.is_empty() {
                    actions.push(&button("open_event", "Add event", "")?);
                }
                if !self.events.is_empty() {
                    actions.push(&button("open_session", "Add session", "")?);
                }
            }
        }
        let mut main = Html::EMPTY;
        let mut sessions = Html::EMPTY;
        if self.error.is_none() {
            if self.courses_page {
                main.push(&self.table(
                    "courses",
                    &self.columns,
                    self.course_rows()?,
                    Some(EmptyState::missing(
                        "No courses yet",
                        "An operator can add courses for this company.",
                    )),
                    "grid",
                )?);
                main.push(&self.pager(
                    "course-pagination",
                    &self.list,
                    self.courses.len(),
                    "page",
                )?);
            } else {
                if self.can_manage && self.choices.is_empty() {
                    main.push(&render(&EmptyState::missing(
                        "No active courses",
                        "Add an active course in Courses before creating an event.",
                    ))?);
                }
                if self.events.is_empty() {
                    main.push(&render(&EmptyState::missing("No events yet","An operator can add an event for an active course, then schedule its sessions."))?);
                } else {
                    let rows = self
                        .list
                        .clone()
                        .paginate(&self.events)
                        .entries
                        .iter()
                        .map(|e| {
                            FlexRow::new(e.id.to_string())
                                .cell("event", Cell::text(&e.name))
                                .cell("course", Cell::text(&e.course_name))
                                .cell("capacity", Cell::text(e.capacity.to_string()))
                        })
                        .collect();
                    main.push(&render(&bilibi_ui::SectionHeading {
                        title: "Events",
                        ..bilibi_ui::SectionHeading::default()
                    })?);
                    main.push(&self.table("training-events", &self.columns, rows, None, "grid")?);
                    main.push(&self.pager(
                        "event-pagination",
                        &self.list,
                        self.events.len(),
                        "event_page",
                    )?);
                }
                if self.sessions.is_empty() {
                    sessions.push(&render(&EmptyState::missing(
                        "No sessions this month",
                        "Choose another month or ask an operator to schedule a session.",
                    ))?);
                }
                if self
                    .list
                    .filters
                    .get("view")
                    .is_some_and(|v| v == "calendar")
                {
                    sessions.push(&self.calendar()?);
                } else if !self.sessions.is_empty() {
                    sessions.push(&self.table(
                        "training-sessions",
                        &self.session_columns,
                        self.session_rows()?,
                        None,
                        "session-grid",
                    )?);
                    sessions.push(&self.pager(
                        "session-pagination",
                        &self.session_list,
                        self.sessions.len(),
                        "session_page",
                    )?);
                }
            }
        }
        View {
            title: self.title(),
            flash: self.flash_map(),
            actions,
            toolbar,
            courses_page: self.courses_page,
            zone: &self.zone,
            error: self.error.as_deref().unwrap_or(""),
            readonly: !self.can_manage && self.error.is_none(),
            limited: self.sessions.len() == CALENDAR_LIMIT,
            main,
            sessions,
            dialog: self.dialog()?,
        }
        .render()
        .map_err(|_| Error::Internal)
    }
}
impl LiveState<Scope, Error> for Catalog {
    fn update<'a>(&'a mut self, req: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            self.load(req.scope).await?;
            match req.input {
                LiveInput::Refresh => {}
                LiveInput::Params(query) => {
                    self.list = ListState::parse(
                        query,
                        spec(
                            if self.courses_page {
                                "page"
                            } else {
                                "event_page"
                            },
                            !self.courses_page,
                        )?,
                    )
                    .map_err(|_| Error::Internal)?;
                    self.session_list = ListState::parse(query, spec("session_page", true)?)
                        .map_err(|_| Error::Internal)?;
                    self.modal = None;
                }
                LiveInput::Event(payload) => self.event(req.scope, payload).await?,
            }
            self.load(req.scope).await?;
            let mut query = self.list.to_params();
            if !self.courses_page {
                if let Some(page) = self.session_list.to_params().get("session_page") {
                    query.insert("session_page".into(), page.clone());
                }
            }
            Ok(LiveBody {
                html: self.render()?,
                query,
                rows: if self.courses_page {
                    self.courses.len()
                } else {
                    self.sessions.len()
                },
                ..LiveBody::default()
            })
        })
    }
}
fn posted(payload: &Value, key: &str) -> String {
    payload
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .into()
}
fn render(template: &impl Template) -> DbResult<Html> {
    Html::render(template).map_err(|_| Error::Internal)
}
fn button(event: &str, label: &str, value: &str) -> DbResult<Html> {
    render(&Button {
        event,
        button_type: "button",
        name: "value",
        value,
        body: Html::text(label),
        ..Button::default()
    })
}
#[derive(Template)]
#[template(path = "catalog.html")]
struct View<'a> {
    title: &'a str,
    flash: FlashMap<'a>,
    actions: Html,
    toolbar: Html,
    courses_page: bool,
    zone: &'a str,
    error: &'a str,
    readonly: bool,
    limited: bool,
    main: Html,
    sessions: Html,
    dialog: Html,
}
#[derive(Template)]
#[template(path = "catalog_entry.html")]
struct Entry<'a> {
    id: &'a str,
    fields: Vec<Field>,
    event: &'a str,
    summary: &'a ErrorSummary,
    actions: Html,
}
struct CalendarEntry {
    name: String,
    time: Html,
    capacity: i32,
}
struct CalendarDay {
    id: String,
    label: String,
    entries: Vec<CalendarEntry>,
}
#[derive(Template)]
#[template(path = "catalog_calendar.html")]
struct Calendar {
    days: Vec<CalendarDay>,
}

#[derive(Template)]
#[template(
    source = "{% import \"flex_table.html\" as table %}{% call table::instant(id=id, i=instant) %}{% endcall %}",
    ext = "html"
)]
struct TimeView<'a> {
    id: &'a str,
    instant: bilibi_ui::Instant,
}

// The shared parser limits strings to 1 KiB; course descriptions allow 4,000
// characters. Keep the small fixed field vocabulary and let the facade validate.
fn entry_values(payload: &Value, namespace: &str) -> HashMap<String, String> {
    [
        "code",
        "name",
        "description",
        "capacity",
        "course_id",
        "event_id",
        "time_zone",
        "starts_local",
        "ends_local",
    ]
    .into_iter()
    .filter_map(|key| {
        payload
            .get(format!("{namespace}[{key}]"))?
            .as_str()
            .map(|value| (key.to_owned(), value.to_owned()))
    })
    .collect()
}
