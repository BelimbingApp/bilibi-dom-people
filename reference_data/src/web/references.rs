//! `/people/references` and `/people/companies/{company_id}/references`
//! (Bilimbi `ReferenceData.Web.IndexLive`): the authorized company chooser,
//! and one company's reference entries, aliases and calendar exceptions, each
//! in a FlexTable, with their add forms. Bilimbi's lists carry no sort, filter
//! or paging, so the tables keep the column settings and row heights only.
//!
//! `people.references.manage` is read again, with the company reach, before
//! every event (Bilimbi's `:reference_access` hook), a write-shaped event is
//! refused before the facade is called when it is gone, and the facade checks
//! it once more inside the write.
use super::{render, Core, Flash};
use crate::{Alias, CalendarException, Entry, NewCalendarException, NewEntry, MANAGE};
use askama::Template;
use bilibi_database::{Error, Result};
use bilibi_people_workforce::CompanySummary;
use bilibi_people_workforce::{authorization, WorkforceError};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    choices,
    page::{LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState},
    params, BuiltinColumn, Button, ButtonVariant, ColumnsOutcome, EmptyState, Field, FlashMap,
    FlexRow, FlexTable, Html, InputKind, ListColumns, TableEmpty,
};
use jiff::civil::Date;
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

const NO_PERMISSION: &str = "You cannot change this company's references.";
const NO_LONGER_PERMITTED: &str =
    "You no longer have permission to change this company's references.";
const COMPANY_UNAVAILABLE: &str = "This company is unavailable to you.";
const ARCHIVED: &str = "This company is archived; nothing in it can change.";

#[derive(Template)]
#[template(path = "references.html")]
struct View<'a> {
    flash: FlashMap<'a>,
    subtitle: String,
    has_companies: bool,
    company_field: Field,
    picker_actions: Html,
    state: Html,
    has_company: bool,
    entries: Html,
    entry_fields: Vec<Field>,
    entry_actions: Html,
    aliases: Html,
    alias_fields: Vec<Field>,
    alias_actions: Html,
    exceptions: Html,
    exception_fields: Vec<Field>,
    exception_actions: Html,
}

pub(super) struct References {
    core: Arc<Core>,
    /// The company named by the address, when it is a positive id.
    company_id: Option<i64>,
    /// The companies the actor may manage references for now.
    companies: Vec<CompanySummary>,
    /// The addressed company, while the actor may manage it.
    company: Option<CompanySummary>,
    entries: Vec<Entry>,
    aliases: Vec<Alias>,
    exceptions: Vec<CalendarException>,
    entry_form: HashMap<String, String>,
    alias_form: HashMap<String, String>,
    exception_form: HashMap<String, String>,
    entry_columns: ListColumns,
    alias_columns: ListColumns,
    exception_columns: ListColumns,
    /// Whether this update read the rows anew, so the columns forget an
    /// unfinished add box.
    rows_changed: bool,
    flash: Flash,
}

/// How a write the facade did not run ended, as Bilimbi's page words it.
enum Refusal {
    /// The grant or the company reach went away.
    Permission,
    Archived,
    /// Anything else the facade refused: the fields, a taken code or alias.
    Rejected,
}

/// Sorts a facade error into a refusal the page flashes, or the fault it is.
fn refusal(error: WorkforceError) -> Result<Refusal> {
    match error {
        WorkforceError::Failed(error) => Err(error),
        WorkforceError::Unauthorized => Ok(Refusal::Permission),
        WorkforceError::CompanyArchived => Ok(Refusal::Archived),
        _ => Ok(Refusal::Rejected),
    }
}

/// Columns Bilimbi shows, none of them sortable.
fn builtins(columns: &[(&str, &str)]) -> Vec<BuiltinColumn> {
    columns
        .iter()
        .map(|(id, label)| BuiltinColumn::new(id, label))
        .collect()
}

impl References {
    pub(super) fn new(core: Arc<Core>, path: &HashMap<String, String>) -> Result<Self> {
        let mount = |id, columns: &[(&str, &str)]| {
            ListColumns::mount(id, builtins(columns)).map_err(|_| Error::Internal)
        };
        Ok(Self {
            core,
            company_id: params::positive_id(path.get("company_id").map(String::as_str)),
            companies: Vec::new(),
            company: None,
            entries: Vec::new(),
            aliases: Vec::new(),
            exceptions: Vec::new(),
            entry_form: HashMap::new(),
            alias_form: HashMap::new(),
            exception_form: HashMap::new(),
            entry_columns: mount(
                "reference-entries-table",
                &[
                    ("label", "Label"),
                    ("kind", "Kind"),
                    ("code", "Code"),
                    ("aliases", "Aliases"),
                    ("state", "State"),
                ],
            )?,
            alias_columns: mount(
                "reference-aliases-table",
                &[
                    ("alias", "Alias"),
                    ("reference", "Reference"),
                    ("kind", "Kind"),
                ],
            )?,
            exception_columns: mount(
                "reference-exceptions-table",
                &[("date", "Date"), ("label", "Label")],
            )?,
            rows_changed: true,
            flash: Flash::default(),
        })
    }

    /// The actor's current reach for the capability: the companies they may
    /// choose and whether the addressed one is among them. A company the
    /// actor lost, or never had, withdraws the controls and the records.
    async fn refresh_access(&mut self, scope: &Scope) -> Result<()> {
        let client = self.core.db.get().await?;
        self.companies = authorization::selectable_companies(&**client, scope, MANAGE)
            .await
            .or_else(|error| refusal(error).map(|_| Vec::new()))?;
        self.company = match self.company_id {
            Some(id) => authorization::authorize_company(&**client, scope, id, MANAGE)
                .await
                .map(Some)
                .or_else(|error| refusal(error).map(|_| None))?,
            None => None,
        };
        if self.company.is_none() {
            self.entries.clear();
            self.aliases.clear();
            self.exceptions.clear();
        }
        Ok(())
    }

    pub(super) async fn load(&mut self, scope: &Scope) -> Result<()> {
        self.refresh_access(scope).await?;
        self.read(scope).await
    }

    async fn read(&mut self, scope: &Scope) -> Result<()> {
        let Some(company) = &self.company else {
            return Ok(());
        };
        if !self.rows_changed {
            return Ok(());
        }
        let references = &self.core.references;
        self.entries = or_empty(references.list_entries(scope, company.id).await)?;
        self.aliases = or_empty(references.list_aliases(scope, company.id).await)?;
        self.exceptions = or_empty(references.list_calendar_exceptions(scope, company.id).await)?;
        self.entry_columns
            .load(&self.entries, |entry| entry.id.to_string());
        self.alias_columns
            .load(&self.aliases, |alias| alias.id.to_string());
        self.exception_columns
            .load(&self.exceptions, |exception| exception.id.to_string());
        Ok(())
    }

    /// Runs one event; a returned path is where the browser goes next.
    async fn event(&mut self, scope: &Scope, payload: &Value) -> Result<Option<String>> {
        let kind = payload
            .get("type")
            .and_then(Value::as_str)
            .ok_or(Error::Invalid)?;
        // Access is judged afresh for every event, including those sent from
        // controls rendered before a grant was revoked.
        self.refresh_access(scope).await?;
        match kind {
            "entries_grid" => self.columns_event(payload, 0),
            "aliases_grid" => self.columns_event(payload, 1),
            "exceptions_grid" => self.columns_event(payload, 2),
            "clear-flash" => {
                self.flash.clear(payload.get("key").and_then(Value::as_str));
                self.rows_changed = false;
            }
            "select_company" => return self.select_company(scope, payload).await,
            "create_entry" | "add_alias" | "create_exception" => {
                if self.company.is_some() {
                    self.write(scope, kind, payload).await?;
                } else {
                    self.flash.error(NO_PERMISSION);
                }
            }
            _ => return Err(Error::Invalid),
        }
        Ok(None)
    }

    /// A column operation changes only what is on screen.
    fn columns_event(&mut self, payload: &Value, table: usize) {
        self.rows_changed = false;
        let columns = match table {
            0 => &mut self.entry_columns,
            1 => &mut self.alias_columns,
            _ => &mut self.exception_columns,
        };
        let _: ColumnsOutcome = columns.handle(&params::string_values(payload));
    }

    /// Opens the chosen company's page when the actor may manage it now
    /// (Bilimbi `select_company`, which navigates).
    async fn select_company(&mut self, scope: &Scope, payload: &Value) -> Result<Option<String>> {
        let chosen = params::positive_id(payload.get("company_id").and_then(Value::as_str));
        let client = self.core.db.get().await?;
        let authorized = match chosen {
            Some(id) => authorization::authorize_company(&**client, scope, id, MANAGE)
                .await
                .map(Some)
                .or_else(|error| refusal(error).map(|_| None))?,
            None => None,
        };
        match authorized {
            Some(company) => Ok(Some(format!("/people/companies/{}/references", company.id))),
            None => {
                self.flash.error(COMPANY_UNAVAILABLE);
                self.rows_changed = false;
                Ok(None)
            }
        }
    }

    async fn write(&mut self, scope: &Scope, kind: &str, payload: &Value) -> Result<()> {
        let Some(company) = self.company.as_ref().map(|company| company.id) else {
            return Ok(());
        };
        let references = &self.core.references;
        match kind {
            "create_entry" => {
                let form = form_values(payload, "entry", &["kind", "code", "label"]);
                let value = |key: &str| form.get(key).cloned().unwrap_or_default();
                let result = references
                    .create_entry(
                        scope,
                        company,
                        &NewEntry::new(value("kind"), value("code"), value("label")),
                    )
                    .await;
                match result {
                    Ok(_) => {
                        self.entry_form.clear();
                        self.flash.success("Reference added.");
                    }
                    Err(error) => {
                        self.entry_form = form;
                        self.refused(
                            refusal(error)?,
                            "Check the reference fields and unique code.",
                        );
                    }
                }
            }
            "add_alias" => {
                let form = form_values(payload, "alias", &["entry_id", "label"]);
                let value = |key: &str| form.get(key).cloned().unwrap_or_default();
                let entry_id = params::positive_id(form.get("entry_id").map(String::as_str));
                let result = match entry_id {
                    Some(entry_id) => references
                        .add_alias(scope, company, entry_id, &value("label"))
                        .await
                        .map(|_| ()),
                    // Bilimbi passes a missing choice on and the facade
                    // answers it with its entry refusal.
                    None => Err(WorkforceError::InvalidOptions),
                };
                match result {
                    Ok(()) => {
                        self.alias_form.clear();
                        self.flash.success("Alias added.");
                    }
                    Err(error) => {
                        self.alias_form = form;
                        self.refused(refusal(error)?, "Choose a reference and a unique alias.");
                    }
                }
            }
            "create_exception" => {
                let form = form_values(payload, "exception", &["on_date", "label"]);
                let value = |key: &str| form.get(key).cloned().unwrap_or_default();
                let result = match value("on_date").trim().parse::<Date>() {
                    Ok(on_date) => references
                        .create_calendar_exception(
                            scope,
                            company,
                            &NewCalendarException::new(on_date, value("label")),
                        )
                        .await
                        .map(|_| ()),
                    Err(_) => Err(WorkforceError::InvalidOptions),
                };
                match result {
                    Ok(()) => {
                        self.exception_form.clear();
                        self.flash.success("Calendar exception added.");
                    }
                    Err(error) => {
                        self.exception_form = form;
                        self.refused(refusal(error)?, "Check the date and label.");
                    }
                }
            }
            _ => return Err(Error::Invalid),
        }
        Ok(())
    }

    /// The flash for a refused write. A refusal of the permission names it as
    /// the cause and withdraws the controls and private records, as the
    /// facade is the authority and `company` only decides what renders.
    fn refused(&mut self, refusal: Refusal, rejected: &str) {
        match refusal {
            Refusal::Permission => {
                self.company = None;
                self.entries.clear();
                self.aliases.clear();
                self.exceptions.clear();
                self.flash.error(NO_LONGER_PERMITTED);
            }
            Refusal::Archived => self.flash.error(ARCHIVED),
            Refusal::Rejected => self.flash.error(rejected),
        }
    }

    pub(super) fn render(&self) -> Result<String> {
        let company = self.company.as_ref();
        let has_companies = !self.companies.is_empty();
        // Bilimbi's three states under the chooser: no company to manage, an
        // address the actor may not use, and no address at all.
        let state = if !has_companies {
            Some(
                EmptyState::missing(
                    "No companies available",
                    "You have no company available for managing People references. Ask an operator to review your access.",
                )
                .id("reference-companies-empty"),
            )
        } else if company.is_none() {
            let title = if self.company_id.is_some() {
                "This company is unavailable in your tenant."
            } else {
                "Choose a company"
            };
            Some(
                EmptyState::missing(
                    title,
                    "Select an authorized company to manage its references and calendar exceptions.",
                )
                .id("reference-company-unselected"),
            )
        } else {
            None
        };
        View {
            flash: self.flash.map(),
            subtitle: company.map_or_else(String::new, |company| {
                format!(
                    "{} · Reference values and calendar exceptions",
                    company.name
                )
            }),
            has_companies,
            company_field: company_field(&self.companies, self.company_id),
            picker_actions: submit("reference-company-open", "Open references")?,
            state: match state {
                Some(state) => render(&state)?,
                None => Html::EMPTY,
            },
            has_company: company.is_some(),
            entries: self.entries_table()?,
            entry_fields: entry_fields(&self.entry_form),
            entry_actions: submit("reference-entry-add", "Add reference")?,
            aliases: self.aliases_table()?,
            alias_fields: alias_fields(&self.entries, &self.alias_form),
            alias_actions: submit("reference-alias-add", "Add alias")?,
            exceptions: self.exceptions_table()?,
            exception_fields: exception_fields(&self.exception_form),
            exception_actions: submit("reference-exception-add", "Add exception")?,
        }
        .render()
        .map_err(|_| Error::Internal)
    }

    fn entries_table(&self) -> Result<Html> {
        let mut rows = Vec::with_capacity(self.entries.len());
        for entry in &self.entries {
            let aliases = self
                .aliases
                .iter()
                .filter(|alias| alias.entry_id == entry.id)
                .map(|alias| alias.label.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            rows.push(
                FlexRow::new(entry.id.to_string())
                    .slot("label", Html::text(&entry.label))
                    .slot("kind", Html::text(&entry.kind))
                    .slot("code", Html::text(&entry.code))
                    .slot("aliases", Html::text(&aliases))
                    .slot(
                        "state",
                        Html::text(if entry.active { "Active" } else { "Inactive" }),
                    ),
            );
        }
        table(
            "reference-entries-table",
            "Reference entries",
            "entries_grid",
            &self.entry_columns,
            rows,
            EmptyState::missing("No references have been added for this company.", ""),
        )
    }

    fn aliases_table(&self) -> Result<Html> {
        let mut rows = Vec::with_capacity(self.aliases.len());
        for alias in &self.aliases {
            let entry = self.entries.iter().find(|entry| entry.id == alias.entry_id);
            rows.push(
                FlexRow::new(alias.id.to_string())
                    .slot("alias", Html::text(&alias.label))
                    .slot(
                        "reference",
                        Html::text(entry.map_or("", |entry| &entry.label)),
                    )
                    .slot("kind", Html::text(entry.map_or("", |entry| &entry.kind))),
            );
        }
        table(
            "reference-aliases-table",
            "Aliases",
            "aliases_grid",
            &self.alias_columns,
            rows,
            EmptyState::missing("No aliases have been added for this company.", ""),
        )
    }

    fn exceptions_table(&self) -> Result<Html> {
        let mut rows = Vec::with_capacity(self.exceptions.len());
        for exception in &self.exceptions {
            rows.push(
                FlexRow::new(exception.id.to_string())
                    .slot("date", Html::text(&exception.on_date.to_string()))
                    .slot("label", Html::text(&exception.label)),
            );
        }
        table(
            "reference-exceptions-table",
            "Calendar exceptions",
            "exceptions_grid",
            &self.exception_columns,
            rows,
            EmptyState::missing(
                "No calendar exceptions have been added for this company.",
                "",
            ),
        )
    }
}

impl LiveState<Scope, Error> for References {
    fn update<'a>(&'a mut self, request: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            self.rows_changed = true;
            let mut redirect = None;
            match request.input {
                LiveInput::Refresh | LiveInput::Params(_) => {
                    self.refresh_access(request.scope).await?
                }
                LiveInput::Event(payload) => redirect = self.event(request.scope, payload).await?,
            }
            self.read(request.scope).await?;
            Ok(LiveBody {
                html: self.render()?,
                rows: self.entries.len() + self.aliases.len() + self.exceptions.len(),
                redirect,
                ..LiveBody::default()
            })
        })
    }
}

/// A read the page shows as empty when the grant or the company went away
/// since mount, as Bilimbi's `load_records` would on its refusal.
fn or_empty<T>(result: bilibi_people_workforce::Result<Vec<T>>) -> Result<Vec<T>> {
    match result {
        Ok(rows) => Ok(rows),
        Err(error) => refusal(error).map(|_| Vec::new()),
    }
}

#[allow(clippy::too_many_arguments)] // one call per table, each naming every part
fn table(
    id: &str,
    caption: &str,
    event: &str,
    columns: &ListColumns,
    rows: Vec<FlexRow>,
    empty: EmptyState,
) -> Result<Html> {
    let none = rows.is_empty();
    let mut table = FlexTable::new(id, columns.flex_columns(), rows).sorted(None, None);
    table.event = event;
    table.zoom = columns.zoom;
    table.suggestions = columns.flex_suggestions();
    table.add_query = &columns.add_query;
    table.caption = caption;
    table.framed = false;
    if none {
        table.empty = Some(TableEmpty::State(empty));
    }
    render(&table)
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

/// The chooser: the companies the actor may manage, with the addressed one
/// selected (Bilimbi `reference-company`).
fn company_field(companies: &[CompanySummary], selected: Option<i64>) -> Field {
    let pairs: Vec<(String, String)> = companies
        .iter()
        .map(|company| (company.name.clone(), company.id.to_string()))
        .collect();
    Field::named("company_id")
        .id("reference-company")
        .label("Company")
        .kind(InputKind::Select)
        .prompt("Choose a company")
        .options(choices(&pairs))
        .value(selected.map_or_else(String::new, |id| id.to_string()))
}

fn entry_fields(form: &HashMap<String, String>) -> Vec<Field> {
    [
        ("kind", "Kind", "80"),
        ("code", "Code", "100"),
        ("label", "Label", "200"),
    ]
    .iter()
    .map(|(key, label, max)| {
        Field::in_form("entry", key)
            .label(*label)
            .value(form.get(*key).cloned().unwrap_or_default())
            .required()
            .attr("maxlength", max)
    })
    .collect()
}

/// The alias form, offered once the company has an entry to name.
fn alias_fields(entries: &[Entry], form: &HashMap<String, String>) -> Vec<Field> {
    if entries.is_empty() {
        return Vec::new();
    }
    let pairs: Vec<(String, String)> = entries
        .iter()
        .map(|entry| (entry.label.clone(), entry.id.to_string()))
        .collect();
    vec![
        Field::in_form("alias", "entry_id")
            .label("Reference")
            .kind(InputKind::Select)
            .options(choices(&pairs))
            .value(form.get("entry_id").cloned().unwrap_or_default()),
        Field::in_form("alias", "label")
            .label("Alias")
            .value(form.get("label").cloned().unwrap_or_default())
            .required()
            .attr("maxlength", "200"),
    ]
}

fn exception_fields(form: &HashMap<String, String>) -> Vec<Field> {
    vec![
        Field::in_form("exception", "on_date")
            .label("Date")
            .kind(InputKind::Date)
            .value(form.get("on_date").cloned().unwrap_or_default())
            .required(),
        Field::in_form("exception", "label")
            .label("Label")
            .value(form.get("label").cloned().unwrap_or_default())
            .required()
            .attr("maxlength", "200"),
    ]
}

/// Keep each submitted field intact for the facade's validation: the shared
/// query-form coercion drops values over 1 KiB, and an overlong label must be
/// refused, never silently discarded.
fn form_values(payload: &Value, namespace: &str, fields: &[&str]) -> HashMap<String, String> {
    fields
        .iter()
        .filter_map(|field| {
            let value = payload.get(format!("{namespace}[{field}]"))?.as_str()?;
            Some(((*field).to_owned(), value.to_owned()))
        })
        .collect()
}
