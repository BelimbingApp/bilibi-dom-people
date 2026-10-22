//! Bilimbi Organisation.Web.ExplorerLive, using the shared list and dialog UI.
use crate::{NewAssignment, Organisation, OrganisationError, PositionProjection, MANAGE, VIEW};
use askama::Template;
use bilibi_database::{Db, Error, Result};
use bilibi_people_workforce::{
    authorization, CompanySummary, Employee, FieldError, Workforce, WorkforceError,
};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::{
        LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState, MountFuture, Page, PageBody,
        PageFuture, PageRequest,
    },
    params, BuiltinColumn, Button, ButtonVariant, Choice, ConfirmDialog, EmptyState, ErrorSummary,
    Field, Filter, FilterToolbar, FlashMap, FlexRow, FlexTable, Html, InputKind, ListColumns,
    ListSpec, ListState, Modal, ModalWidth, Pagination, StatusKind, TableEmpty, ToolbarControl,
    NO_FLASH,
};
use jiff::civil::Date;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

const FORBIDDEN: &str = "You no longer have permission to change this company's assignments.";
const END_FAILED: &str = "The assignment could not be ended on that date.";
const ASSIGN_FAILED: &str = "The assignment could not be recorded with those details.";
struct Core {
    db: Db,
    organisation: Arc<Organisation>,
    workforce: Arc<Workforce>,
}
pub(crate) struct Pages(Arc<Core>);
impl Pages {
    pub(crate) fn new(db: Db, organisation: Arc<Organisation>, workforce: Arc<Workforce>) -> Self {
        Self(Arc::new(Core {
            db,
            organisation,
            workforce,
        }))
    }
}
impl Page<Scope, Error> for Pages {
    fn title(&self, _: &str) -> Option<&'static str> {
        Some("Organisation")
    }
    fn render<'a>(&'a self, request: PageRequest<'a, Scope>) -> PageFuture<'a, Error> {
        Box::pin(async move {
            let mut state = Explorer::new(self.0.clone(), request.query)?;
            state
                .load(request.scope.ok_or(Error::Unauthorized)?)
                .await?;
            Ok(PageBody {
                html: state.render()?,
                live: true,
                title: None,
            })
        })
    }
    fn mount<'a>(&'a self, request: PageRequest<'a, Scope>) -> MountFuture<'a, Scope, Error> {
        Box::pin(async move {
            let state: Box<dyn LiveState<Scope, Error>> =
                Box::new(Explorer::new(self.0.clone(), request.query)?);
            Ok(Some(state))
        })
    }
}
fn today() -> Date {
    jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date()
}
fn spec() -> Result<Arc<ListSpec>> {
    ListSpec {
        page_sizes: vec![25, 50, 100],
        default_page_size: 50,
        filters: BTreeMap::from([
            ("company_id".into(), Filter::string("")),
            ("as_of".into(), Filter::string(&today().to_string())),
        ]),
        ..ListSpec::default()
    }
    .shared()
    .map_err(|_| Error::Internal)
}
#[derive(Clone, Copy)]
enum Selection {
    Assign { company: i64, position: i64 },
    End { company: i64, assignment: i64 },
}
enum Pending {
    Assign {
        company: i64,
        position: i64,
        input: NewAssignment,
    },
    End {
        company: i64,
        assignment: i64,
        day: Date,
    },
}
struct Explorer {
    core: Arc<Core>,
    list: ListState,
    columns: ListColumns,
    companies: Vec<CompanySummary>,
    company: Option<CompanySummary>,
    positions: Vec<PositionProjection>,
    total: u64,
    can_manage: bool,
    employees: Vec<Employee>,
    selection: Option<Selection>,
    pending: Option<Pending>,
    values: HashMap<String, String>,
    errors: Vec<FieldError>,
    flash: Option<(StatusKind, String)>,
}
impl Explorer {
    fn new(core: Arc<Core>, query: &HashMap<String, String>) -> Result<Self> {
        let mut state = Self {
            core,
            list: ListState::parse(query, spec()?).map_err(|_| Error::Internal)?,
            columns: ListColumns::mount(
                "organisation-positions",
                vec![
                    BuiltinColumn::new("title", "Position"),
                    BuiltinColumn::new("parent", "Parent"),
                    BuiltinColumn::new("holders", "Holders"),
                    BuiltinColumn::new("vacancy", "Vacancy"),
                ],
            )
            .map_err(|_| Error::Internal)?,
            companies: vec![],
            company: None,
            positions: vec![],
            total: 0,
            can_manage: false,
            employees: vec![],
            selection: None,
            pending: None,
            values: HashMap::new(),
            errors: vec![],
            flash: None,
        };
        state.normalize();
        Ok(state)
    }
    fn normalize(&mut self) {
        if self.list.page > 10_000 {
            self.list.page = 1;
        }
        let day = self
            .list
            .filters
            .get("as_of")
            .and_then(|d| d.parse::<Date>().ok())
            .unwrap_or_else(today);
        self.list.filters.insert("as_of".into(), day.to_string());
    }
    fn day(&self) -> Date {
        self.list.filters["as_of"]
            .parse()
            .unwrap_or_else(|_| today())
    }
    fn message(&mut self, kind: StatusKind, text: &str) {
        self.flash = Some((kind, text.into()));
    }
    fn flash(&self) -> FlashMap<'_> {
        let mut flash = NO_FLASH;
        if let Some((kind, text)) = &self.flash {
            match kind {
                StatusKind::Success => flash.success = text,
                StatusKind::Error => flash.error = text,
                _ => {}
            }
        }
        flash
    }
    fn clear_selection(&mut self) {
        self.selection = None;
        self.pending = None;
        self.values.clear();
        self.errors.clear();
        self.employees.clear();
    }
    async fn load(&mut self, scope: &Scope) -> Result<()> {
        let client = self.core.db.get().await?;
        self.companies = match authorization::selectable_companies(&**client, scope, VIEW).await {
            Ok(rows) => rows.into_iter().filter(|c| !c.archived).collect(),
            Err(WorkforceError::Failed(e)) => return Err(e),
            Err(_) => vec![],
        };
        // Workforce summaries expose archival state only. Keep Bilimbi's
        // active-only picker with a tenant-scoped status projection, after
        // the owner has authorised every candidate company and named it.
        let candidates: Vec<i64> = self.companies.iter().map(|c| c.id).collect();
        let active: Vec<i64> = client.query(
            "SELECT id FROM companies WHERE tenant_id=$1 AND id=ANY($2) AND status='active' AND deleted_at IS NULL",
            &[&scope.tenant(), &candidates],
        ).await?.iter().map(|r| r.get(0)).collect();
        self.companies.retain(|c| active.contains(&c.id));
        let requested = &self.list.filters["company_id"];
        self.company = if requested.is_empty() {
            self.companies.first().cloned()
        } else {
            self.companies
                .iter()
                .find(|c| c.id.to_string() == *requested)
                .cloned()
        };
        self.can_manage = if let Some(c) = &self.company {
            authorization::allowed(&**client, scope, c.id, MANAGE).await
        } else {
            false
        };
        drop(client);
        if let Some(company) = &self.company {
            let id = company.id;
            self.list
                .filters
                .insert("company_id".into(), id.to_string());
            let result = async {
                let total = self.core.organisation.count_positions(scope, id).await?;
                let rows = self
                    .core
                    .organisation
                    .positions(
                        scope,
                        id,
                        self.day(),
                        self.list.page as i64,
                        self.list.page_size as i64,
                    )
                    .await?;
                Ok::<_, OrganisationError>((total, rows))
            }
            .await;
            match result {
                Ok((total, rows)) => {
                    self.total = total as u64;
                    self.positions = rows;
                }
                Err(OrganisationError::Failed(e)) => return Err(e),
                Err(_) => {
                    self.company = None;
                    self.positions.clear();
                    self.total = 0;
                }
            }
        } else {
            self.positions.clear();
            self.total = 0;
        }
        let selected_company = match self.selection {
            Some(Selection::Assign { company, .. } | Selection::End { company, .. }) => {
                Some(company)
            }
            None => None,
        };
        if !self.can_manage
            || selected_company.is_some_and(|id| self.company.as_ref().map(|c| c.id) != Some(id))
        {
            self.clear_selection();
        }
        Ok(())
    }
    async fn may_write(&mut self, scope: &Scope) -> Result<bool> {
        let Some(company) = self.company.as_ref().map(|c| c.id) else {
            self.clear_selection();
            self.message(StatusKind::Error, FORBIDDEN);
            return Ok(false);
        };
        let client = self.core.db.get().await?;
        for capability in [VIEW, MANAGE] {
            match authorization::authorize(&**client, scope, company, capability).await {
                Ok(_) => {}
                Err(WorkforceError::Failed(e)) => return Err(e),
                Err(_) => {
                    self.clear_selection();
                    self.message(StatusKind::Error, FORBIDDEN);
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
    fn refused(&mut self, error: OrganisationError, fallback: &str) -> Result<()> {
        match error {
            OrganisationError::Failed(e) => return Err(e),
            OrganisationError::Unauthorized => {
                self.clear_selection();
                self.message(StatusKind::Error, FORBIDDEN);
            }
            OrganisationError::CompanyArchived => {
                self.clear_selection();
                self.message(
                    StatusKind::Error,
                    "This company is archived; nothing in it can change.",
                );
            }
            OrganisationError::Invalid(errors) => {
                self.errors = errors;
                self.message(StatusKind::Error, fallback);
            }
            _ => self.message(StatusKind::Error, fallback),
        }
        Ok(())
    }
    async fn event(&mut self, scope: &Scope, payload: &Value) -> Result<()> {
        let event = payload.get("event").and_then(Value::as_str).unwrap_or("");
        if matches!(
            event,
            "assign" | "end_assignment" | "review_assignment" | "confirm_assignment"
        ) && !self.may_write(scope).await?
        {
            return Ok(());
        }
        match event {
            "dismiss-flash" => self.flash = None,
            "filters" | "filter" => {
                self.clear_selection();
                self.list
                    .apply_filters(&params::form_values(payload, "filters"));
                self.normalize();
            }
            "page" => {
                self.clear_selection();
                self.list
                    .put_page(payload.get("page").and_then(Value::as_str));
                self.normalize();
            }
            "grid" => {
                self.columns.handle(&params::string_values(payload));
            }
            "cancel_assignment" => self.clear_selection(),
            "cancel_confirmation" => self.pending = None,
            "assign" | "end_assignment" => {
                self.clear_selection();
                let id = payload.get("value").and_then(Value::as_str);
                let company = self.company.as_ref().map(|c| c.id).ok_or(Error::Invalid)?;
                if event == "assign" {
                    let position = self
                        .positions
                        .iter()
                        .find(|p| Some(p.reference.stable_id.as_str()) == id)
                        .and_then(|p| p.reference.stable_id.parse::<i64>().ok());
                    if let Some(position) = position {
                        match self.core.workforce.employees(scope, company).await {
                            Ok(read) => match read.require_current() {
                                Ok(employees) => self.employees = employees,
                                Err(_) => {
                                    self.message(StatusKind::Error, "Employees are unavailable.");
                                    return Ok(());
                                }
                            },
                            Err(WorkforceError::Failed(e)) => return Err(e),
                            Err(_) => {
                                self.message(StatusKind::Error, "Employees are unavailable.");
                                return Ok(());
                            }
                        }
                        self.selection = Some(Selection::Assign { company, position });
                        self.values.insert("kind".into(), "substantive".into());
                        self.values
                            .insert("effective_from".into(), self.day().to_string());
                    }
                } else {
                    let assignment = self
                        .positions
                        .iter()
                        .flat_map(|p| &p.assignments)
                        .find(|a| Some(a.reference.stable_id.as_str()) == id)
                        .and_then(|a| a.reference.stable_id.parse::<i64>().ok());
                    if let Some(assignment) = assignment {
                        self.selection = Some(Selection::End {
                            company,
                            assignment,
                        });
                        self.values
                            .insert("effective_to".into(), self.day().to_string());
                    }
                }
                if self.selection.is_none() {
                    self.message(StatusKind::Error, "The assignment is unavailable.");
                }
            }
            "review_assignment" => self.review(payload),
            "confirm_assignment" => {
                // The confirmation payload never chooses a company, record or date.
                match self.pending.take() {
                    Some(Pending::Assign {
                        company,
                        position,
                        input,
                    }) => {
                        match self
                            .core
                            .organisation
                            .assign(scope, company, position, &input)
                            .await
                        {
                            Ok(_) => {
                                self.clear_selection();
                                self.message(StatusKind::Success, "Assignment recorded.");
                            }
                            Err(e) => self.refused(e, ASSIGN_FAILED)?,
                        }
                    }
                    Some(Pending::End {
                        company,
                        assignment,
                        day,
                    }) => {
                        match self
                            .core
                            .organisation
                            .end_assignment(scope, company, assignment, day)
                            .await
                        {
                            Ok(_) => {
                                self.clear_selection();
                                self.message(StatusKind::Success, "Assignment ended.");
                            }
                            Err(e) => self.refused(e, END_FAILED)?,
                        }
                    }
                    None => self.message(StatusKind::Error, "The assignment is unavailable."),
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn review(&mut self, payload: &Value) {
        // A duplicate submit cannot replace an already-reviewed operation.
        if self.pending.is_some() {
            return;
        }
        self.values = params::form_values(payload, "assignment");
        self.errors.clear();
        match self.selection {
            Some(Selection::Assign { company, position }) => {
                let employee =
                    params::positive_id(self.values.get("employee_id").map(String::as_str));
                let employee = employee.filter(|id| {
                    self.employees
                        .iter()
                        .any(|e| e.reference.stable_id == id.to_string())
                });
                if employee.is_none() {
                    self.errors
                        .push(FieldError::new("employee_id", "is invalid"));
                }
                let kind = self.value("kind").to_owned();
                if !crate::ASSIGNMENT_KINDS.contains(&kind.as_str()) {
                    self.errors.push(FieldError::new("kind", "is invalid"));
                }
                let start = self.date("effective_from", true);
                let end = self.date("effective_to", false);
                if start.zip(end).is_some_and(|(start, end)| end < start) {
                    self.errors.push(FieldError::new(
                        "effective_to",
                        "must be on or after the start",
                    ));
                }
                if self.errors.is_empty() {
                    if let (Some(employee_id), Some(effective_from)) = (employee, start) {
                        self.pending = Some(Pending::Assign {
                            company,
                            position,
                            input: NewAssignment {
                                employee_id,
                                kind,
                                effective_from,
                                effective_to: end,
                            },
                        });
                    }
                } else {
                    self.message(StatusKind::Error, ASSIGN_FAILED);
                }
            }
            Some(Selection::End {
                company,
                assignment,
            }) => {
                if let Some(day) = self.date("effective_to", true) {
                    self.pending = Some(Pending::End {
                        company,
                        assignment,
                        day,
                    });
                } else {
                    self.message(StatusKind::Error, END_FAILED);
                }
            }
            None => self.message(StatusKind::Error, "The assignment is unavailable."),
        }
    }
    fn value(&self, key: &str) -> &str {
        self.values.get(key).map_or("", String::as_str)
    }
    fn date(&mut self, key: &'static str, required: bool) -> Option<Date> {
        let raw = self.value(key);
        if raw.is_empty() && !required {
            return None;
        }
        match raw.parse() {
            Ok(day) => Some(day),
            Err(_) => {
                self.errors.push(FieldError::new(key, "is invalid"));
                None
            }
        }
    }
    fn table(&self) -> Result<Html> {
        let rows = self
            .positions
            .iter()
            .map(|p| {
                let mut holders = Html::EMPTY;
                for assignment in &p.assignments {
                    let end = if self.can_manage {
                        action("end_assignment", "End", &assignment.reference.stable_id)?
                    } else {
                        Html::EMPTY
                    };
                    holders.push(&render(&Holder {
                        label: format!(
                            "{} · Employee {}",
                            capitalize(&assignment.kind),
                            assignment.employee_reference.stable_id
                        ),
                        end,
                    })?);
                }
                if p.assignments_incomplete {
                    holders.push(&render(&EmptyState::missing(
                        "More assignments exist beyond this page's display limit.",
                        "",
                    ))?);
                }
                let mut row = FlexRow::new(&p.reference.stable_id)
                    .slot(
                        "title",
                        render(&TitleCell {
                            title: p.title.as_deref().unwrap_or(&p.code),
                            code: &p.code,
                            compact: self.columns.mode() == bilibi_ui::RowMode::Compact,
                        })?,
                    )
                    .slot(
                        "parent",
                        Html::text(
                            &p.parent_reference
                                .as_ref()
                                .map(|r| format!("Reports to position {}", r.stable_id))
                                .unwrap_or_default(),
                        ),
                    )
                    .slot("holders", holders)
                    .slot(
                        "vacancy",
                        Html::text(if p.vacant {
                            "Vacant"
                        } else {
                            "Substantive holder assigned"
                        }),
                    );
                if self.can_manage {
                    row.actions = action("assign", "Assign", &p.reference.stable_id)?;
                }
                Ok(row)
            })
            .collect::<Result<Vec<_>>>()?;
        let mut table = FlexTable::new("organisation-positions", self.columns.flex_columns(), rows);
        table.event = "grid";
        table.zoom = self.columns.zoom;
        table.suggestions = self.columns.flex_suggestions();
        table.add_query = &self.columns.add_query;
        table.caption = "Positions";
        table.actions = self.can_manage;
        table.empty = Some(TableEmpty::State(
            EmptyState::missing("No positions have been recorded for this company.", "")
                .id("organisation-empty"),
        ));
        render(&table)
    }
    fn field(&self, key: &str, label: &str, kind: InputKind) -> Field {
        Field::in_form("assignment", key)
            .id(format!("assignment-{key}"))
            .label(label)
            .kind(kind)
            .value(self.value(key))
            .errors(
                self.errors
                    .iter()
                    .filter(|e| e.field == key)
                    .map(|e| e.message.clone())
                    .collect(),
            )
    }
    fn dialog(&self) -> Result<Html> {
        let Some(selection) = self.selection else {
            return Ok(Html::EMPTY);
        };
        if self.pending.is_some() {
            return Ok(Html::EMPTY);
        }
        let (title, id, fields) = match selection {
            Selection::Assign { position, .. } => (
                format!("Assign employee to position {position}"),
                "assign-position",
                vec![
                    self.field("employee_id", "Employee", InputKind::Select)
                        .required()
                        .prompt("Choose employee")
                        .options(
                            self.employees
                                .iter()
                                .map(|e| {
                                    Choice::new(
                                        format!("{} · {}", e.display_name, e.employee_number),
                                        &e.reference.stable_id,
                                    )
                                })
                                .collect(),
                        ),
                    self.field("kind", "Kind", InputKind::Select)
                        .required()
                        .options(
                            crate::ASSIGNMENT_KINDS
                                .iter()
                                .map(|k| Choice::new(capitalize(k), *k))
                                .collect(),
                        ),
                    self.field("effective_from", "First day of assignment", InputKind::Date)
                        .required(),
                    self.field("effective_to", "Last day of assignment", InputKind::Date),
                ],
            ),
            Selection::End { assignment, .. } => (
                format!("End assignment {assignment}"),
                "end-assignment",
                vec![self
                    .field("effective_to", "Last day of assignment", InputKind::Date)
                    .required()],
            ),
        };
        let mut summary = ErrorSummary::new("assignment-errors");
        for e in &self.errors {
            summary.push(
                format!("assignment-{}", e.field),
                bilibi_ui::forms::error_sentence(&e.field.replace('_', " "), &e.message),
            );
        }
        let body = render(&AssignmentForm {
            fields,
            summary: if summary.is_empty() {
                Html::EMPTY
            } else {
                render(&summary)?
            },
            actions: {
                let mut actions = render(&Button {
                    button_type: "submit",
                    variant: ButtonVariant::Primary,
                    body: Html::text("Review"),
                    ..Button::default()
                })?;
                actions.push(&action("cancel_assignment", "Cancel", "")?);
                actions
            },
        })?;
        render(&Modal {
            id,
            title: &title,
            cancel_event: "cancel_assignment",
            width: ModalWidth::Narrow,
            flash: Some(self.flash()),
            body,
            ..Modal::default()
        })
    }
    fn confirmation(&self) -> Result<Html> {
        let (consequence, detail, confirm) = match &self.pending {
            Some(Pending::Assign { company, position, input }) => ("Assign employee?", format!("Company {company} · Position {position} · Employee {} · {} · {} to {}", input.employee_id, capitalize(&input.kind), input.effective_from, input.effective_to.map_or_else(|| "no end date".into(), |d| d.to_string())), "Assign"),
            Some(Pending::End { company, assignment, day }) => ("End assignment?", format!("Company {company} · Assignment {assignment} · Last day {day}. This cannot be undone."), "End"),
            None => return Ok(Html::EMPTY),
        };
        render(&ConfirmDialog {
            id: "organisation-confirm-assignment",
            consequence,
            detail: &detail,
            confirm,
            working: "Saving…",
            confirm_event: "confirm_assignment",
            cancel_event: "cancel_confirmation",
        })
    }
    fn render(&self) -> Result<String> {
        let filters = if self.companies.is_empty() {
            Html::EMPTY
        } else {
            render(&FilterToolbar::new(
                "organisation-filter",
                vec![
                    ToolbarControl::select(
                        "organisation-company",
                        "company_id",
                        "Company",
                        &self.list.filters["company_id"],
                        self.companies
                            .iter()
                            .map(|c| Choice::new(&c.name, c.id.to_string()))
                            .collect(),
                    ),
                    ToolbarControl::date(
                        "organisation-date",
                        "as_of",
                        "As of",
                        &self.day().to_string(),
                    ),
                ],
            ))?
        };
        let unavailable = if self.companies.is_empty() {
            render(
                &EmptyState::missing("No active company is available.", "")
                    .id("organisation-no-company"),
            )?
        } else if self.company.is_none() {
            render(
                &EmptyState::missing("This company is unavailable.", "")
                    .id("organisation-unavailable"),
            )?
        } else {
            Html::EMPTY
        };
        let mut pagination = Pagination::new(
            "organisation-pagination",
            bilibi_ui::PageCounts::new(self.list.page, self.list.page_size, self.total),
            &[25, 50, 100],
        );
        pagination.filters_event = "filters";
        View {
            flash: self.flash(),
            filters,
            unavailable,
            table: if self.company.is_some() {
                self.table()?
            } else {
                Html::EMPTY
            },
            pagination: if self.company.is_some() {
                render(&pagination)?
            } else {
                Html::EMPTY
            },
            dialog: self.dialog()?,
            confirmation: self.confirmation()?,
        }
        .render()
        .map_err(|_| Error::Internal)
    }
}
impl LiveState<Scope, Error> for Explorer {
    fn update<'a>(&'a mut self, request: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            // Mount is followed by Refresh; events always re-read company reach.
            if matches!(request.input, LiveInput::Event(_)) {
                self.load(request.scope).await?;
            }
            match request.input {
                LiveInput::Refresh => {}
                LiveInput::Params(query) => {
                    self.clear_selection();
                    self.list = ListState::parse(query, spec()?).map_err(|_| Error::Internal)?;
                    self.normalize();
                }
                LiveInput::Event(payload) => self.event(request.scope, payload).await?,
            }
            self.load(request.scope).await?;
            Ok(LiveBody {
                html: self.render()?,
                query: self.list.to_params(),
                rows: self.positions.len(),
                ..LiveBody::default()
            })
        })
    }
}
#[derive(Template)]
#[template(path = "explorer.html")]
struct View<'a> {
    flash: FlashMap<'a>,
    filters: Html,
    unavailable: Html,
    table: Html,
    pagination: Html,
    dialog: Html,
    confirmation: Html,
}
#[derive(Template)]
#[template(path = "assignment_form.html")]
struct AssignmentForm {
    fields: Vec<Field>,
    summary: Html,
    actions: Html,
}
#[derive(Template)]
#[template(
    source = "<div class=\"flex flex-wrap items-center gap-2\">{{ label }}{{ end }}</div>",
    ext = "html"
)]
struct Holder {
    label: String,
    end: Html,
}
#[derive(Template)]
#[template(
    source = "<div>{{ title }}</div>{% if !compact %}<div class=\"text-sm text-ink-muted\">{{ code }}</div>{% endif %}",
    ext = "html"
)]
struct TitleCell<'a> {
    title: &'a str,
    code: &'a str,
    compact: bool,
}
fn render(template: &impl Template) -> Result<Html> {
    Html::render(template).map_err(|_| Error::Internal)
}
fn action(event: &str, label: &str, value: &str) -> Result<Html> {
    render(&Button {
        event,
        name: "value",
        value,
        body: Html::text(label),
        ..Button::default()
    })
}
fn capitalize(value: &str) -> String {
    let mut chars = value.chars();
    chars.next().map_or_else(String::new, |c| {
        c.to_uppercase().to_string() + chars.as_str()
    })
}
