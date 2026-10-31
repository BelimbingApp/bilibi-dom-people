//! Employees workbench, mirroring Bilimbi EmployeeWorkspace.Web.IndexLive.
//! Employment creation and detail editing use the Workforce owner.
mod detail;
use crate::{EmployeeWorkspace, NewSavedView, SavedView, WorkspaceError, VIEW};
use askama::Template;
use bilibi_affiliation::{AffiliationService, Search as AffiliationSearch};
use bilibi_database::{Db, Error, Result};
use bilibi_party_ids::{OrganisationId, PersonId};
use bilibi_people_workforce::{
    authorization, CompanySummary, DirectoryEmployee, FieldError, NewEmployment, Workforce,
    WorkforceError, EMPLOYEE_STATUSES, EMPLOYMENT_MANAGE,
};
use bilibi_person::{PersonKind, PersonService, Search as PersonSearch};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::{
        LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState, MountFuture, Page, PageBody,
        PageFuture, PageRequest,
    },
    params, BuiltinColumn, Button, ButtonVariant, Choice, ColumnsOutcome, Combobox, ConfirmDialog,
    EmptyState, ErrorSummary, Field, Filter, FilterToolbar, FlashMap, FlexRow, FlexTable, Html,
    InputKind, ListColumns, ListSpec, ListState, Modal, ModalWidth, RecordLink, SortDir,
    StatusKind, TableEmpty, ToolbarControl, NO_FLASH,
};
use jiff::civil::Date;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

const REFUSAL: &str = "You no longer have permission to view this company's employees.";
struct Core {
    db: Db,
    workspace: Arc<EmployeeWorkspace>,
    workforce: Arc<Workforce>,
    persons: Arc<PersonService>,
    affiliations: Arc<AffiliationService>,
}
pub(crate) struct Pages(Arc<Core>);
impl Pages {
    pub(crate) fn new(
        db: Db,
        workspace: Arc<EmployeeWorkspace>,
        workforce: Arc<Workforce>,
        persons: Arc<PersonService>,
        affiliations: Arc<AffiliationService>,
    ) -> Self {
        Self(Arc::new(Core {
            db,
            workspace,
            workforce,
            persons,
            affiliations,
        }))
    }
}
impl Page<Scope, Error> for Pages {
    fn title(&self, operation: &str) -> Option<&'static str> {
        Some(if operation == "employee" {
            "Employee workbench"
        } else {
            "Employees"
        })
    }
    fn render<'a>(&'a self, request: PageRequest<'a, Scope>) -> PageFuture<'a, Error> {
        Box::pin(async move {
            let scope = request.scope.ok_or(Error::Unauthorized)?;
            if request.operation == "employee" {
                let mut state = detail::Detail::new(self.0.clone(), &request)?;
                state.load(scope).await?;
                return Ok(PageBody {
                    html: state.render()?,
                    live: true,
                    title: None,
                });
            }
            let mut state = Employees::new(self.0.clone(), request.query, request.framed)?;
            state.load(scope).await?;
            Ok(PageBody {
                html: state.render()?,
                live: true,
                title: None,
            })
        })
    }
    fn mount<'a>(&'a self, request: PageRequest<'a, Scope>) -> MountFuture<'a, Scope, Error> {
        Box::pin(async move {
            if request.operation == "employee" {
                let state: Box<dyn LiveState<Scope, Error>> =
                    Box::new(detail::Detail::new(self.0.clone(), &request)?);
                return Ok(Some(state));
            }
            let state: Box<dyn LiveState<Scope, Error>> = Box::new(Employees::new(
                self.0.clone(),
                request.query,
                request.framed,
            )?);
            Ok(Some(state))
        })
    }
}
#[derive(Template)]
#[template(path = "employees.html")]
struct View<'a> {
    flash: FlashMap<'a>,
    subtitle: String,
    has_company: bool,
    filters: Html,
    actions: Html,
    employees: Html,
    views: Html,
    view_name: Field,
    save_actions: Html,
    unavailable: Html,
    dialog: Html,
    confirmation: Html,
}
#[derive(Template)]
#[template(path = "employment_form.html")]
struct EmploymentForm {
    person: Combobox,
    identity_actions: Html,
    selected: bool,
    fields: Vec<Field>,
    actions: Html,
    summary: Html,
}

struct Employees {
    core: Arc<Core>,
    list: ListState,
    columns: ListColumns,
    view_columns: ListColumns,
    company: Option<CompanySummary>,
    companies: Vec<CompanySummary>,
    employees: Vec<DirectoryEmployee>,
    views: Vec<SavedView>,
    flash: Option<(StatusKind, String)>,
    framed: bool,
    can_add: bool,
    add: bool,
    person: Option<i64>,
    persons: Vec<Choice>,
    affiliations: Vec<Choice>,
    types: Vec<Choice>,
    working_statuses: Vec<String>,
    form: HashMap<String, String>,
    errors: Vec<FieldError>,
    pending_delete: Option<i64>,
    view_name: String,
}
fn render(template: &impl Template) -> Result<Html> {
    Html::render(template).map_err(|_| Error::Internal)
}
fn list_spec() -> Result<Arc<ListSpec>> {
    ListSpec {
        sortable: ["number", "name", "status", "type", "supervisor"]
            .into_iter()
            .map(|s| (s.to_owned(), SortDir::Asc))
            .collect(),
        default_sort: Some("name".into()),
        filters: BTreeMap::from([
            (
                "status".into(),
                Filter::one_of(
                    &[
                        "",
                        "pending",
                        "probation",
                        "active",
                        "inactive",
                        "terminated",
                    ],
                    "",
                ),
            ),
            ("company_id".into(), Filter::string("")),
        ]),
        omit_defaults: true,
        ..ListSpec::default()
    }
    .shared()
    .map_err(|_| Error::Internal)
}
impl Employees {
    fn new(core: Arc<Core>, query: &HashMap<String, String>, framed: bool) -> Result<Self> {
        Ok(Self {
            core,
            list: ListState::parse(query, list_spec()?).map_err(|_| Error::Internal)?,
            columns: ListColumns::mount(
                "people-employees-table",
                vec![
                    BuiltinColumn::new("number", "Number").sortable("number"),
                    BuiltinColumn::new("name", "Employee").sortable("name"),
                    BuiltinColumn::new("status", "Status").sortable("status"),
                    BuiltinColumn::new("type", "Employment type").sortable("type"),
                    BuiltinColumn::new("supervisor", "Supervisor").sortable("supervisor"),
                ],
            )
            .map_err(|_| Error::Internal)?,
            view_columns: ListColumns::mount(
                "people-employee-views-table",
                vec![BuiltinColumn::new("name", "View")],
            )
            .map_err(|_| Error::Internal)?,
            company: None,
            companies: vec![],
            employees: vec![],
            views: vec![],
            flash: None,
            framed,
            can_add: false,
            add: false,
            person: None,
            persons: vec![],
            affiliations: vec![],
            types: vec![],
            working_statuses: vec![],
            form: HashMap::new(),
            errors: vec![],
            pending_delete: None,
            view_name: String::new(),
        })
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
    fn success(&mut self, text: &str) {
        self.flash = Some((StatusKind::Success, text.into()));
    }
    fn error(&mut self, text: &str) {
        self.flash = Some((StatusKind::Error, text.into()));
    }
    async fn load(&mut self, scope: &Scope) -> Result<()> {
        let client = self.core.db.get().await?;
        self.companies = match authorization::company_choices(
            &**client,
            scope,
            VIEW,
            params::positive_id(self.list.filters.get("company_id").map(String::as_str)),
        )
        .await
        {
            Ok(companies) => companies,
            Err(WorkforceError::Failed(error)) => return Err(error),
            Err(_) => vec![],
        };
        let requested = self
            .list
            .filters
            .get("company_id")
            .filter(|id| !id.is_empty());
        let id = if let Some(id) = requested {
            params::positive_id(Some(id))
        } else {
            scope.current_company().map(|id| id.get())
        };
        self.company = self.companies.iter().find(|c| Some(c.id) == id).cloned();
        self.can_add = if let Some(company) = &self.company {
            authorization::allowed(&**client, scope, company.id, EMPLOYMENT_MANAGE).await
                && authorization::allowed(&**client, scope, company.id, "admin.person.list").await
                && authorization::allowed(&**client, scope, company.id, "admin.affiliation.list")
                    .await
        } else {
            false
        };
        drop(client);
        if !self.can_add {
            self.add = false;
            self.persons.clear();
            self.affiliations.clear();
        }
        let Some(company) = &self.company else {
            self.employees.clear();
            self.views.clear();
            self.pending_delete = None;
            return Ok(());
        };
        match self.core.workspace.employees(scope, company.id).await {
            Ok(rows) => self.employees = rows,
            Err(WorkspaceError::Workforce(WorkforceError::Failed(error))) => return Err(error),
            Err(_) => {
                self.company = None;
                self.employees.clear();
                self.views.clear();
                self.error(REFUSAL);
                return Ok(());
            }
        }
        self.views = match self.core.workspace.saved_views(scope, company.id).await {
            Ok(rows) => rows,
            Err(error) => {
                self.refused(error, REFUSAL)?;
                vec![]
            }
        };
        Ok(())
    }
    fn refused(&mut self, error: WorkspaceError, fallback: &str) -> Result<()> {
        match error {
            WorkspaceError::Workforce(WorkforceError::Failed(error)) => return Err(error),
            WorkspaceError::Workforce(WorkforceError::Unauthorized) => self.error(REFUSAL),
            WorkspaceError::Workforce(WorkforceError::CompanyArchived) => {
                self.error("This company is archived; nothing in it can change.")
            }
            WorkspaceError::Workforce(WorkforceError::Invalid(errors)) => {
                self.errors = errors;
                self.error(fallback);
            }
            _ => self.error(fallback),
        }
        Ok(())
    }
    async fn open_add(&mut self, scope: &Scope) -> Result<()> {
        let client = self
            .core
            .db
            .get()
            .await
            .map_err(bilibi_database::Error::from)?;
        if !self.can_add {
            self.error("You cannot add an employment for this company.");
            return Ok(());
        }
        let Some(company) = &self.company else {
            return Ok(());
        };
        self.types = match self
            .core
            .workforce
            .employment_types(&**client, scope, company.id)
            .await
        {
            Ok(types) => types
                .into_iter()
                .map(|t| Choice::new(t.label, t.code))
                .collect(),
            Err(e) => {
                self.refused(e.into(), "Employment types are unavailable.")?;
                return Ok(());
            }
        };
        self.working_statuses = self
            .core
            .workforce
            .working_statuses(&**client, scope, company.id)
            .await
            .map_err(|e| match e {
                WorkforceError::Failed(e) => e,
                _ => Error::Invalid,
            })?
            .require_current()
            .map_err(|_| Error::Invalid)?;
        drop(client);
        self.persons.clear();
        let mut page = 1;
        loop {
            let found = match self
                .core
                .persons
                .search(
                    scope,
                    &PersonSearch {
                        kind: Some(PersonKind::Human),
                        page,
                        page_size: 300,
                        ..PersonSearch::default()
                    },
                )
                .await
            {
                Ok(found) => found,
                Err(Error::Forbidden | Error::Unauthorized) => {
                    self.error("You cannot list people. Ask an operator to review your access.");
                    return Ok(());
                }
                Err(e) => return Err(e),
            };
            let total = found.total;
            self.persons.extend(
                found
                    .entries
                    .into_iter()
                    .filter(|p| p.system_key.is_none())
                    .map(|p| Choice::new(p.name, p.id.get().to_string())),
            );
            if page * 300 >= total {
                break;
            }
            page += 1;
        }
        self.add = true;
        self.person = None;
        self.affiliations.clear();
        self.form.clear();
        self.errors.clear();
        self.form.insert("status".into(), "pending".into());
        Ok(())
    }
    async fn select_person(&mut self, scope: &Scope, payload: &Value) -> Result<()> {
        if !self.add || !self.can_add {
            return Ok(());
        }
        let Some(company) = self.company.as_ref().map(|c| c.id) else {
            return Ok(());
        };
        let person =
            params::positive_id(payload.get("identity[person_id]").and_then(Value::as_str));
        if person != self.person {
            self.form.remove("affiliation_id");
        }
        self.person = person.filter(|id| self.persons.iter().any(|p| p.value == id.to_string()));
        self.affiliations.clear();
        if let Some(person) = self.person {
            let mut page = 1;
            loop {
                let found = self
                    .core
                    .affiliations
                    .search(
                        scope,
                        &AffiliationSearch {
                            person_id: Some(PersonId::new(person).map_err(|_| Error::Invalid)?),
                            organisation_id: Some(
                                OrganisationId::new(company).map_err(|_| Error::Invalid)?,
                            ),
                            page,
                            page_size: 100,
                            ..AffiliationSearch::default()
                        },
                    )
                    .await;
                let found = match found {
                    Ok(found) => found,
                    Err(Error::Forbidden | Error::Unauthorized) => {
                        self.error(
                            "You cannot list affiliations. Ask an operator to review your access.",
                        );
                        return Ok(());
                    }
                    Err(e) => return Err(e),
                };
                let total = found.total;
                self.affiliations.extend(found.entries.into_iter().map(|a| {
                    Choice::new(
                        a.affiliation
                            .title
                            .unwrap_or_else(|| format!("Affiliation {}", a.id.get())),
                        a.id.get().to_string(),
                    )
                }));
                if page * 100 >= total {
                    break;
                }
                page += 1;
            }
        }
        Ok(())
    }
    async fn create(&mut self, scope: &Scope, payload: &Value) -> Result<()> {
        let client = self
            .core
            .db
            .get()
            .await
            .map_err(bilibi_database::Error::from)?;
        if !self.add || !self.can_add {
            self.error("You cannot add an employment for this company.");
            return Ok(());
        }
        let Some(company) = self.company.as_ref().map(|c| c.id) else {
            return Ok(());
        };
        self.form = params::form_values(payload, "employment");
        self.errors.clear();
        let Some(person) = self.person else {
            self.errors
                .push(FieldError::new("person_id", "can't be blank"));
            self.error("Choose a person and their affiliation with this company.");
            return Ok(());
        };
        let affiliation = params::positive_id(self.form.get("affiliation_id").map(String::as_str));
        let Some(affiliation) =
            affiliation.filter(|id| self.affiliations.iter().any(|a| a.value == id.to_string()))
        else {
            self.errors
                .push(FieldError::new("affiliation_id", "is invalid"));
            self.error("Choose a person and their affiliation with this company.");
            return Ok(());
        };
        // Re-read the company's choices for this write; the form cannot
        // submit a type that was never offered or has since been retired.
        let types = match self
            .core
            .workforce
            .employment_types(&**client, scope, company)
            .await
        {
            Ok(types) => types,
            Err(e) => {
                self.refused(e.into(), "Employment types are unavailable.")?;
                return Ok(());
            }
        };
        if !types
            .iter()
            .any(|t| t.code == self.value("employment_type"))
        {
            self.errors
                .push(FieldError::new("employment_type", "is invalid"));
            self.error("Please check the employment details.");
            return Ok(());
        }
        drop(client);
        let mut new = NewEmployment::new(
            person,
            affiliation,
            self.value("employee_number"),
            self.value("employment_type"),
            self.value("status"),
        );
        for key in ["employment_start", "employment_end"] {
            let raw = self.value(key);
            let date = if raw.is_empty() {
                None
            } else {
                match raw.parse::<Date>() {
                    Ok(date) => Some(date),
                    Err(_) => {
                        self.errors.push(FieldError::new(key, "is invalid"));
                        None
                    }
                }
            };
            if key == "employment_start" {
                new.employment_start = date;
            } else {
                new.employment_end = date;
            }
        }
        let raw = self.value("supervisor_employment_id");
        new.supervisor_employment_id = params::positive_id(Some(raw));
        if !raw.is_empty() && new.supervisor_employment_id.is_none() {
            self.errors
                .push(FieldError::new("supervisor_employment_id", "is invalid"));
        }
        if !self.errors.is_empty() {
            self.error("Please check the employment details.");
            return Ok(());
        }
        match self
            .core
            .workforce
            .create_employment(scope, company, &new)
            .await
        {
            Ok(_) => {
                self.add = false;
                self.form.clear();
                self.person = None;
                self.success("Employment created successfully.");
            }
            Err(WorkforceError::DuplicateAffiliation) => {
                self.errors.push(FieldError::new(
                    "affiliation_id",
                    "already has an employment record",
                ));
                self.error("This affiliation already has an employment record");
            }
            Err(WorkforceError::AffiliationMismatch | WorkforceError::NotHuman) => {
                self.errors
                    .push(FieldError::new("affiliation_id", "is invalid"));
                self.error("Choose a person and their affiliation with this company.");
            }
            Err(e) => self.refused(e.into(), "Please check the employment details.")?,
        }
        Ok(())
    }
    fn value(&self, key: &str) -> &str {
        self.form.get(key).map_or("", String::as_str)
    }
    async fn event(&mut self, scope: &Scope, payload: &Value) -> Result<()> {
        let kind = payload
            .get("type")
            .and_then(Value::as_str)
            .ok_or(Error::Invalid)?;
        // Page rechecks now; each writing facade authorizes again in its transaction.
        self.load(scope).await?;
        match kind {
            "clear-flash" => {
                if self.flash.as_ref().is_some_and(|(k, _)| {
                    Some(k.name()) == payload.get("key").and_then(Value::as_str)
                }) {
                    self.flash = None;
                }
            }
            "filters" => {
                let company = self.list.filters.get("company_id").cloned();
                self.list
                    .apply_filters(&params::form_values(payload, "filters"));
                self.list.search = self.list.search.chars().take(200).collect();
                if company != self.list.filters.get("company_id").cloned() {
                    self.add = false;
                    self.pending_delete = None;
                    self.form.clear();
                }
            }
            "grid" => {
                if let ColumnsOutcome::Sort(key) =
                    self.columns.handle(&params::string_values(payload))
                {
                    self.list.next_sort(&key);
                }
            }
            "views_grid" => {
                self.view_columns.handle(&params::string_values(payload));
            }
            "open_add" => self.open_add(scope).await?,
            "cancel_add" => {
                self.add = false;
                self.form.clear();
                self.errors.clear();
            }
            "select_person" => self.select_person(scope, payload).await?,
            "create_employment" => self.create(scope, payload).await?,
            "save_view" => {
                if let Some(company) = self.company.as_ref().map(|c| c.id) {
                    self.view_name = payload
                        .get("view[name]")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned();
                    let new = NewSavedView {
                        name: self.view_name.clone(),
                        search: Some(self.list.search.clone()),
                        status: self.list.filters.get("status").cloned(),
                    };
                    match self.core.workspace.save_view(scope, company, &new).await {
                        Ok(_) => {
                            self.view_name.clear();
                            self.success("View saved.");
                        }
                        Err(e) => self.refused(e, "Use a unique view name.")?,
                    }
                } else {
                    self.error(REFUSAL);
                }
            }
            "open_view" => {
                let id = params::positive_id(payload.get("value").and_then(Value::as_str));
                if let Some(view) = self.views.iter().find(|v| Some(v.id) == id) {
                    self.list.apply_filters(&HashMap::from([
                        ("search".into(), view.search.clone().unwrap_or_default()),
                        ("status".into(), view.status.clone().unwrap_or_default()),
                    ]));
                } else {
                    self.error("View unavailable.");
                }
            }
            "delete_view" => {
                let id = params::positive_id(payload.get("value").and_then(Value::as_str));
                self.pending_delete = self.views.iter().find(|v| Some(v.id) == id).map(|v| v.id);
                if self.pending_delete.is_none() {
                    self.error("View unavailable.");
                }
            }
            "cancel_delete" => self.pending_delete = None,
            "confirm_delete" => {
                if let (Some(company), Some(id)) = (
                    self.company.as_ref().map(|c| c.id),
                    self.pending_delete.take(),
                ) {
                    match self.core.workspace.delete_view(scope, company, id).await {
                        Ok(()) => self.success("View deleted."),
                        Err(e) => self.refused(e, "View unavailable.")?,
                    }
                } else {
                    self.error("View unavailable.");
                }
            }
            _ => return Err(Error::Invalid),
        }
        Ok(())
    }
    fn supervisor(&self, employee: &DirectoryEmployee) -> String {
        employee
            .employee
            .supervisor_reference
            .as_ref()
            .and_then(|r| {
                self.employees
                    .iter()
                    .find(|e| e.id.to_string() == r.stable_id)
            })
            .map_or_else(String::new, |e| e.employee.display_name.clone())
    }
    fn visible(&self) -> Vec<&DirectoryEmployee> {
        let search = self.list.search.to_lowercase();
        let status = self.list.filters.get("status").map_or("", String::as_str);
        let mut rows = self
            .employees
            .iter()
            .filter(|e| {
                (search.is_empty()
                    || e.employee.display_name.to_lowercase().contains(&search)
                    || e.employee.employee_number.to_lowercase().contains(&search))
                    && (status.is_empty() || e.status == status)
            })
            .collect::<Vec<_>>();
        let key = |e: &DirectoryEmployee| {
            match self.list.sort_by.as_deref() {
                Some("number") => e.employee.employee_number.clone(),
                Some("status") => e.status.clone(),
                Some("type") => e.employment_type.clone(),
                Some("supervisor") => self.supervisor(e),
                _ => e.employee.display_name.clone(),
            }
            .to_lowercase()
        };
        rows.sort_by(|a, b| key(a).cmp(&key(b)).then(a.id.cmp(&b.id)));
        if self.list.sort_dir == Some(SortDir::Desc) {
            rows.reverse();
        }
        rows
    }
    fn employees_table(&self) -> Result<Html> {
        let rows = self
            .visible()
            .into_iter()
            .map(|e| {
                let id = e.id.to_string();
                let href = params::address(
                    &format!("/people/employees/{id}"),
                    &HashMap::from([(
                        "company_id".into(),
                        e.employee.platform_company_id.to_string(),
                    )]),
                );
                Ok(FlexRow::new(&id)
                    .slot("number", Html::text(&e.employee.employee_number))
                    .slot(
                        "name",
                        render(&RecordLink {
                            workspace: self.framed,
                            kind: "people/workforce",
                            record_id: &id,
                            href: &href,
                            class: "",
                            body: Html::text(&e.employee.display_name),
                        })?,
                    )
                    .slot("status", Html::text(&e.status))
                    .slot("type", Html::text(&e.employment_type))
                    .slot("supervisor", Html::text(&self.supervisor(e))))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut table = FlexTable::new("people-employees-table", self.columns.flex_columns(), rows)
            .sorted(self.list.sort_by.as_deref(), self.list.sort_dir);
        table.event = "grid";
        table.zoom = self.columns.zoom;
        table.suggestions = self.columns.flex_suggestions();
        table.add_query = &self.columns.add_query;
        table.caption = "Employees";
        table.empty = Some(TableEmpty::State(EmptyState::missing(
            "No employees match this view.",
            "",
        )));
        render(&table)
    }
    fn views_table(&self) -> Result<Html> {
        let rows = self
            .views
            .iter()
            .map(|v| {
                let id = v.id.to_string();
                let mut row = FlexRow::new(&id).slot("name", action("open_view", &v.name, &id)?);
                row.actions = action("delete_view", "Remove", &id)?;
                Ok(row)
            })
            .collect::<Result<Vec<_>>>()?;
        let mut table = FlexTable::new(
            "people-employee-views-table",
            self.view_columns.flex_columns(),
            rows,
        );
        table.event = "views_grid";
        table.zoom = self.view_columns.zoom;
        table.suggestions = self.view_columns.flex_suggestions();
        table.add_query = &self.view_columns.add_query;
        table.caption = "Saved views";
        table.actions = true;
        table.empty = Some(TableEmpty::State(EmptyState::missing(
            "No saved views yet.",
            "",
        )));
        render(&table)
    }
    fn field(&self, key: &'static str, label: &str, kind: InputKind) -> Field {
        Field::in_form("employment", key)
            .id(format!("employment-{key}"))
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
    fn employment_dialog(&self) -> Result<Html> {
        if !self.add {
            return Ok(Html::EMPTY);
        }
        let mut fields = vec![
            self.field("affiliation_id", "Affiliation", InputKind::Select)
                .options(self.affiliations.clone())
                .prompt("Choose affiliation")
                .required(),
            self.field("employee_number", "Employee number", InputKind::Text)
                .attr("maxlength", "100")
                .required(),
            self.field("employment_type", "Employment type", InputKind::Select)
                .options(self.types.clone())
                .prompt("Choose type")
                .required(),
            self.field("status", "Status", InputKind::Select)
                .options(
                    EMPLOYEE_STATUSES
                        .iter()
                        .map(|s| Choice::new(capitalize(s), *s))
                        .collect(),
                )
                .required(),
            self.field("employment_start", "Employment start", InputKind::Date),
            self.field("employment_end", "Employment end", InputKind::Date),
        ];
        fields.push(
            self.field("supervisor_employment_id", "Supervisor", InputKind::Select)
                .prompt("None")
                .options(
                    self.employees
                        .iter()
                        .filter(|e| self.working_statuses.contains(&e.status))
                        .map(|e| {
                            Choice::new(
                                format!(
                                    "{} · {}",
                                    e.employee.display_name, e.employee.employee_number
                                ),
                                e.id.to_string(),
                            )
                        })
                        .collect(),
                ),
        );
        let mut actions = submit("employment-create", "Add employment")?;
        actions.push(&action("cancel_add", "Cancel", "")?);
        let mut summary = ErrorSummary::new("employment-errors");
        for error in &self.errors {
            summary.push(
                format!("employment-{}", error.field),
                bilibi_ui::forms::error_sentence(&error.field.replace('_', " "), &error.message),
            );
        }
        let body = render(&EmploymentForm {
            person: Combobox::new("employment-person", "identity[person_id]")
                .label("Person")
                .value(self.person.map(|p| p.to_string()).unwrap_or_default())
                .options(self.persons.clone())
                .placeholder("Choose a person")
                .errors(
                    self.errors
                        .iter()
                        .filter(|e| e.field == "person_id")
                        .map(|e| e.message.clone())
                        .collect(),
                ),
            identity_actions: if self.person.is_none() {
                action("cancel_add", "Cancel", "")?
            } else {
                Html::EMPTY
            },
            selected: self.person.is_some(),
            fields,
            actions,
            summary: if !summary.is_empty() {
                render(&summary)?
            } else if self.person.is_some() && self.affiliations.is_empty() {
                render(&EmptyState::missing(
                    "No linked affiliation for this company.",
                    "Add the person's affiliation in Contacts before creating their employment.",
                ))?
            } else if self.types.is_empty() {
                render(&EmptyState::missing(
                    "No employment types have been added.",
                    "Add an employment type in People references.",
                ))?
            } else {
                Html::EMPTY
            },
        })?;
        render(&Modal {
            id: "add-employment",
            title: "Add employment",
            cancel_event: "cancel_add",
            width: ModalWidth::Wide,
            flash: Some(self.flash()),
            body,
            ..Modal::default()
        })
    }
    fn render(&self) -> Result<String> {
        let controls = vec![
            ToolbarControl::search(
                "employee-search",
                "search",
                "Search employees",
                &self.list.search,
                "Search name or number",
            ),
            ToolbarControl::select(
                "employee-status",
                "status",
                "Filter status",
                self.list.filters.get("status").map_or("", String::as_str),
                std::iter::once(Choice::new("All statuses", ""))
                    .chain(
                        EMPLOYEE_STATUSES
                            .iter()
                            .map(|s| Choice::new(capitalize(s), *s)),
                    )
                    .collect(),
            ),
            ToolbarControl::select(
                "employee-company",
                "company_id",
                "Company",
                &self
                    .company
                    .as_ref()
                    .map(|c| c.id.to_string())
                    .unwrap_or_default(),
                std::iter::once(Choice::new("Choose company", ""))
                    .chain(
                        self.companies
                            .iter()
                            .map(|c| Choice::new(&c.name, c.id.to_string())),
                    )
                    .collect(),
            ),
        ];
        let confirmation = if self.pending_delete.is_some() {
            render(&ConfirmDialog {
                id: "delete-employee-view",
                consequence: "Remove saved view?",
                detail: "This cannot be undone.",
                confirm: "Remove",
                working: "Removing…",
                confirm_event: "confirm_delete",
                cancel_event: "cancel_delete",
            })?
        } else {
            Html::EMPTY
        };
        View {
            flash: self.flash(),
            subtitle: self
                .company
                .as_ref()
                .map(|c| format!("{} · People workbench", c.name))
                .unwrap_or_default(),
            has_company: self.company.is_some(),
            filters: render(&FilterToolbar::new("people-employee-filter", controls))?,
            actions: if self.can_add {
                action("open_add", "Add employment", "")?
            } else {
                Html::EMPTY
            },
            employees: self.employees_table()?,
            views: self.views_table()?,
            view_name: Field::in_form("view", "name")
                .id("people-view-name")
                .label("View name")
                .value(&self.view_name)
                .required()
                .attr("maxlength", "120"),
            save_actions: submit("people-view-save", "Save current view")?,
            unavailable: render(&EmptyState::missing("This company is unavailable.", ""))?,
            dialog: self.employment_dialog()?,
            confirmation,
        }
        .render()
        .map_err(|_| Error::Internal)
    }
}
impl LiveState<Scope, Error> for Employees {
    fn update<'a>(&'a mut self, request: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            match request.input {
                LiveInput::Refresh => {}
                LiveInput::Params(query) => {
                    let company = self.list.filters.get("company_id").cloned();
                    self.list =
                        ListState::parse(query, list_spec()?).map_err(|_| Error::Internal)?;
                    if company != self.list.filters.get("company_id").cloned() {
                        self.add = false;
                        self.pending_delete = None;
                    }
                }
                LiveInput::Event(payload) => self.event(request.scope, payload).await?,
            }
            self.load(request.scope).await?;
            Ok(LiveBody {
                html: self.render()?,
                query: self.list.to_params(),
                rows: self.visible().len(),
                ..LiveBody::default()
            })
        })
    }
}
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map_or_else(String::new, |c| {
        c.to_uppercase().to_string() + chars.as_str()
    })
}
fn action(event: &str, label: &str, value: &str) -> Result<Html> {
    // Live buttons post their native value only when it has a name.
    // Keep saved-view ids in the event payload through the shared Button.
    render(&Button {
        event,
        name: "value",
        value,
        body: Html::text(label),
        ..Button::default()
    })
}
fn submit(id: &str, label: &str) -> Result<Html> {
    render(&Button {
        id,
        button_type: "submit",
        variant: ButtonVariant::Primary,
        body: Html::text(label),
        ..Button::default()
    })
}
