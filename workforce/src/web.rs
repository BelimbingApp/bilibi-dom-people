//! Bilimbi Workforce.Web.SettingsLive: company working statuses and employment
//! counts. Legacy claims have no counterpart in Bilibi's fresh schema.
use crate::{
    authorization, CompanySummary, EmploymentSummary, Workforce, WorkforceError, EMPLOYEE_STATUSES,
    SETTINGS_MANAGE,
};
use askama::Template;
use bilibi_database::{Db, Error, Result};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    choices,
    page::{
        LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState, MountFuture, Page, PageBody,
        PageFuture, PageRequest,
    },
    params, BuiltinColumn, Button, ButtonVariant, EmptyState, Fact, Facts, Field, FlashMap,
    FlexRow, FlexTable, Html, InputKind, ListColumns, ListSpec, ListState, StatusKind, NO_FLASH,
};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

pub(crate) struct Pages(Arc<Core>);
struct Core {
    db: Db,
    workforce: Arc<Workforce>,
}
impl Pages {
    pub(crate) fn new(db: Db, workforce: Arc<Workforce>) -> Self {
        Self(Arc::new(Core { db, workforce }))
    }
}
impl Page<Scope, Error> for Pages {
    fn title(&self, _: &str) -> Option<&'static str> {
        Some("Workforce Settings")
    }
    fn render<'a>(&'a self, req: PageRequest<'a, Scope>) -> PageFuture<'a, Error> {
        Box::pin(async move {
            let mut state = SettingsPage::new(self.0.clone(), req.query)?;
            state
                .load(req.scope.ok_or(Error::Unauthorized)?, true)
                .await?;
            Ok(PageBody {
                html: state.render()?,
                live: true,
                title: None,
            })
        })
    }
    fn mount<'a>(&'a self, req: PageRequest<'a, Scope>) -> MountFuture<'a, Scope, Error> {
        Box::pin(async move {
            let mut state = SettingsPage::new(self.0.clone(), req.query)?;
            state
                .load(req.scope.ok_or(Error::Unauthorized)?, true)
                .await?;
            Ok(Some(Box::new(state) as Box<dyn LiveState<Scope, Error>>))
        })
    }
}

#[derive(Template)]
#[template(path = "settings.html")]
struct View<'a> {
    flash: FlashMap<'a>,
    company: Field,
    has_company: bool,
    empty: Html,
    statuses: Vec<Field>,
    actions: Html,
    counts: Html,
    summary: Html,
    has_counts: bool,
}
struct SettingsPage {
    core: Arc<Core>,
    company_id: Option<i64>,
    companies: Vec<CompanySummary>,
    selected: Option<CompanySummary>,
    statuses: Vec<String>,
    counts: Option<EmploymentSummary>,
    columns: ListColumns,
    list: ListState,
    flash: Option<(StatusKind, String)>,
}
impl SettingsPage {
    fn new(core: Arc<Core>, query: &HashMap<String, String>) -> Result<Self> {
        let spec = Arc::new(ListSpec {
            page_sizes: vec![5],
            default_page_size: 5,
            omit_defaults: true,
            ..ListSpec::default()
        });
        Ok(Self {
            core,
            company_id: params::positive_id(query.get("company_id").map(String::as_str)),
            companies: vec![],
            selected: None,
            statuses: vec![],
            counts: None,
            columns: ListColumns::mount(
                "workforce-employment-counts",
                vec![
                    BuiltinColumn::new("status", "Status"),
                    BuiltinColumn::new("count", "Employees").right(),
                ],
            )
            .map_err(|_| Error::Internal)?,
            list: ListState::parse(&HashMap::new(), spec).map_err(|_| Error::Internal)?,
            flash: None,
        })
    }
    async fn load(&mut self, scope: &Scope, choose_default: bool) -> Result<()> {
        let client = self.core.db.get().await?;
        self.companies =
            authorization::company_choices(&**client, scope, SETTINGS_MANAGE, self.company_id)
                .await
                .map_err(failed)?;
        if choose_default && !self.companies.iter().any(|c| Some(c.id) == self.company_id) {
            self.company_id = self.companies.first().map(|c| c.id);
        }
        self.selected = self
            .companies
            .iter()
            .find(|c| Some(c.id) == self.company_id)
            .cloned();
        self.counts = None;
        let Some(company) = &self.selected else {
            self.statuses.clear();
            return Ok(());
        };
        self.statuses = self
            .core
            .workforce
            .working_statuses(&**client, scope, company.id)
            .await
            .map_err(failed)?
            .value
            .unwrap_or_default();
        self.counts = match self
            .core
            .workforce
            .employment_summary(&**client, scope, company.id)
            .await
        {
            Ok(read) => read.value,
            Err(WorkforceError::Unauthorized) => None,
            Err(error) => return Err(failed(error)),
        };
        if let Some(counts) = &self.counts {
            self.columns
                .load(&counts.by_status, |row| row.status.clone());
        }
        Ok(())
    }
    async fn event(&mut self, scope: &Scope, payload: &Value) -> Result<()> {
        match payload
            .get("type")
            .and_then(Value::as_str)
            .ok_or(Error::Invalid)?
        {
            "clear-flash" => {
                if self.flash.as_ref().is_some_and(|(kind, _)| {
                    Some(kind.name()) == payload.get("key").and_then(Value::as_str)
                }) {
                    self.flash = None;
                }
            }
            "counts_grid" => {
                self.columns.handle(&params::string_values(payload));
            }
            "select_company" => {
                self.company_id =
                    params::positive_id(payload.get("company_id").and_then(Value::as_str));
                self.load(scope, true).await?;
            }
            "save_statuses" => {
                // The company is the server's selection, never a company from
                // this form. Re-prove permission before any write-shaped work.
                let Some(id) = self.company_id else {
                    return Ok(());
                };
                let client = self.core.db.get().await?;
                if authorization::authorize(&**client, scope, id, SETTINGS_MANAGE)
                    .await
                    .is_err()
                {
                    self.flash = Some((
                        StatusKind::Error,
                        "Working statuses could not be saved.".into(),
                    ));
                    self.load(scope, false).await?;
                    return Ok(());
                }
                drop(client);
                let selected = params::form_values(payload, "statuses");
                let statuses: Vec<String> = selected
                    .into_iter()
                    .filter(|(_, v)| v == "true")
                    .map(|(k, _)| k)
                    .collect();
                let outcome = self
                    .core
                    .workforce
                    .put_working_statuses(scope, id, &statuses)
                    .await;
                let (kind, message) = match outcome {
                    Ok(_) => (StatusKind::Success, "Working statuses saved."),
                    Err(WorkforceError::InvalidStatuses) => {
                        (StatusKind::Error, "Choose at least one employee status.")
                    }
                    Err(_) => (StatusKind::Error, "Working statuses could not be saved."),
                };
                self.flash = Some((kind, message.into()));
                self.load(scope, false).await?;
                if kind == StatusKind::Error {
                    self.statuses = statuses;
                }
            }
            _ => return Err(Error::Invalid),
        }
        Ok(())
    }
    fn render(&self) -> Result<String> {
        let pairs: Vec<_> = self
            .companies
            .iter()
            .map(|c| (c.name.clone(), c.id.to_string()))
            .collect();
        let company = Field::named("company_id")
            .id("workforce-company")
            .label("Company")
            .kind(InputKind::Select)
            .options(choices(&pairs))
            .value(
                self.selected
                    .as_ref()
                    .map_or(String::new(), |c| c.id.to_string()),
            );
        let statuses = EMPLOYEE_STATUSES
            .iter()
            .map(|status| {
                Field::in_form("statuses", status)
                    .id(format!("workforce-status-{status}"))
                    .label(label(status))
                    .kind(InputKind::Checkbox)
                    .checked(self.statuses.iter().any(|s| s == status))
            })
            .collect();
        let mut flash = NO_FLASH;
        if let Some((kind, message)) = &self.flash {
            match kind {
                StatusKind::Success => flash.success = message,
                _ => flash.error = message,
            }
        }
        let (counts, summary) = if let Some(counts) = &self.counts {
            let rows = counts
                .by_status
                .iter()
                .map(|c| {
                    FlexRow::new(&c.status)
                        .slot("status", Html::text(&label(&c.status)))
                        .slot("count", Html::text(&c.count.to_string()))
                })
                .collect();
            let mut table = FlexTable::new(
                "workforce-employment-counts",
                self.columns.flex_columns(),
                rows,
            )
            .sorted(None, None);
            table.event = "counts_grid";
            table.zoom = self.columns.zoom;
            table.suggestions = self.columns.flex_suggestions();
            table.add_query = &self.columns.add_query;
            table.caption = "Employment records";
            table.framed = false;
            (
                html(&table)?,
                html(&Facts {
                    id: "workforce-employment-summary",
                    items: vec![
                        Fact {
                            id: "workforce-employment-total",
                            title: "Employees",
                            value: Html::text(&counts.total.to_string()),
                        },
                        Fact {
                            id: "workforce-employment-working",
                            title: "Working staff",
                            value: Html::text(&counts.working.to_string()),
                        },
                    ],
                })?,
            )
        } else {
            (Html::default(), Html::default())
        };
        View {
            flash,
            company,
            has_company: self.selected.is_some(),
            statuses,
            counts,
            summary,
            has_counts: self.counts.is_some(),
            empty: html(
                &EmptyState::missing("No active company is available for workforce settings.", "")
                    .id("workforce-settings-empty"),
            )?,
            actions: html(&Button {
                id: "workforce-settings-save",
                button_type: "submit",
                variant: ButtonVariant::Primary,
                body: Html::text("Save"),
                ..Button::default()
            })?,
        }
        .render()
        .map_err(|_| Error::Internal)
    }
    fn query(&self) -> HashMap<String, String> {
        let mut query = self.list.to_params();
        if let Some(id) = self.company_id {
            query.insert("company_id".into(), id.to_string());
        }
        query
    }
}
impl LiveState<Scope, Error> for SettingsPage {
    fn update<'a>(&'a mut self, req: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            match req.input {
                LiveInput::Refresh => self.load(req.scope, false).await?,
                LiveInput::Params(params) => {
                    self.company_id =
                        params::positive_id(params.get("company_id").map(String::as_str));
                    self.load(req.scope, true).await?;
                }
                LiveInput::Event(payload) => self.event(req.scope, payload).await?,
            }
            Ok(LiveBody {
                html: self.render()?,
                query: self.query(),
                rows: self.counts.as_ref().map_or(0, |c| c.by_status.len()),
                ..LiveBody::default()
            })
        })
    }
}
fn html(template: &impl Template) -> Result<Html> {
    Html::render(template).map_err(|_| Error::Internal)
}
fn failed(error: WorkforceError) -> Error {
    match error {
        WorkforceError::Failed(e) => e,
        WorkforceError::Unauthorized => Error::Forbidden,
        WorkforceError::NotFound => Error::NotFound,
        _ => Error::Invalid,
    }
}
fn label(status: &str) -> String {
    let mut chars = status.chars();
    chars.next().map_or(String::new(), |c| {
        c.to_uppercase().collect::<String>() + chars.as_str()
    })
}
