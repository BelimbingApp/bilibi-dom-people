//! Bilimbi Skills.Web.CatalogLive and Skills.Web.ProfileLive.
//! The mounted page owns drafts and confirmations; the Skills facade owns rules.
mod catalog;
mod profile;
use crate::{Skills, SkillsError, CATALOG_VIEW};
use bilibi_database::{Db, Error, Result};
use bilibi_people_workforce::{authorization, CompanySummary};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::{LiveState, MountFuture, Page, PageBody, PageFuture, PageRequest},
    params, BuiltinColumn, Button, ButtonVariant, Card, ConfirmDialog, EmptyState, Field, FlashMap,
    FlexRow, FlexTable, Form, Html, InputKind, ListColumns, ListSpec, ListState, SectionHeading,
    StatusKind, TableEmpty, NO_FLASH,
};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

pub(crate) const PAGES: &str = "people/skills.catalog-profile-pages";
const FORBIDDEN: &str = "You cannot change this company's skills.";
const PUBLISH_FORBIDDEN: &str = "You cannot publish or retire this company's profiles.";
struct Core {
    db: Db,
    skills: Arc<Skills>,
    instances: Arc<bilibi_system::Instances>,
}
pub(crate) struct Pages(Arc<Core>);
impl Pages {
    pub(crate) fn new(
        db: Db,
        skills: Arc<Skills>,
        instances: Arc<bilibi_system::Instances>,
    ) -> Self {
        Self(Arc::new(Core {
            db,
            skills,
            instances,
        }))
    }
    fn state(&self, request: &PageRequest<'_, Scope>) -> Result<Box<dyn LiveState<Scope, Error>>> {
        match request.operation {
            "catalog" => Ok(Box::new(catalog::Catalog::new(
                self.0.clone(),
                request.query,
                request.framed,
            )?)),
            "profile" => Ok(Box::new(profile::ProfilePage::new(
                self.0.clone(),
                request.query,
                params::positive_id(request.path.get("id").map(String::as_str)),
                request.framed,
            )?)),
            _ => Err(Error::NotFound),
        }
    }
}
impl Page<Scope, Error> for Pages {
    fn title(&self, operation: &str) -> Option<&'static str> {
        match operation {
            "catalog" => Some("Skills"),
            "profile" => Some("Requirement profile"),
            _ => None,
        }
    }
    fn render<'a>(&'a self, request: PageRequest<'a, Scope>) -> PageFuture<'a, Error> {
        Box::pin(async move {
            let scope = request.scope.ok_or(Error::Unauthorized)?;
            let mut state = self.state(&request)?;
            let body = state
                .update(bilibi_ui::page::LiveRequest {
                    scope,
                    credential: "",
                    input: bilibi_ui::page::LiveInput::Refresh,
                })
                .await?;
            Ok(PageBody {
                html: body.html,
                live: true,
                title: None,
            })
        })
    }
    fn mount<'a>(&'a self, request: PageRequest<'a, Scope>) -> MountFuture<'a, Scope, Error> {
        Box::pin(async move { Ok(Some(self.state(&request)?)) })
    }
}

#[derive(Default)]
struct Flash(Option<(StatusKind, String)>);
impl Flash {
    fn error(&mut self, message: &str) {
        self.0 = Some((StatusKind::Error, message.into()));
    }
    fn success(&mut self, message: &str) {
        self.0 = Some((StatusKind::Success, message.into()));
    }
    fn dismiss(&mut self, payload: &Value) {
        if self.0.as_ref().is_some_and(|(kind, _)| {
            Some(kind.name()) == payload.get("key").and_then(Value::as_str)
        }) {
            self.0 = None;
        }
    }
    fn map(&self) -> FlashMap<'_> {
        let mut map = NO_FLASH;
        if let Some((kind, text)) = &self.0 {
            match kind {
                StatusKind::Success => map.success = text,
                _ => map.error = text,
            }
        }
        map
    }
}
fn message(error: &SkillsError) -> String {
    match error {
        SkillsError::Failed(_) => "That could not be done.".into(),
        _ => error.to_string(),
    }
}

struct Company {
    id: Option<i64>,
    companies: Vec<CompanySummary>,
    selected: Option<CompanySummary>,
    banner: Html,
}
impl Company {
    fn new(query: &HashMap<String, String>) -> Self {
        Self {
            id: params::positive_id(query.get("company_id").map(String::as_str)),
            companies: vec![],
            selected: None,
            banner: Html::EMPTY,
        }
    }
    async fn load(&mut self, core: &Core, scope: &Scope, fallback: bool) -> Result<()> {
        let client = core.db.get().await?;
        self.companies = authorization::selectable_companies(&**client, scope, CATALOG_VIEW)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|c| !c.archived)
            .collect();
        self.selected = self
            .companies
            .iter()
            .find(|c| Some(c.id) == self.id)
            .cloned();
        if fallback && self.selected.is_none() {
            self.selected = self.companies.first().cloned();
        }
        self.id = self.selected.as_ref().map(|c| c.id);
        self.banner = core
            .instances
            .resolve()
            .await?
            .banner("skills-catalog-confirm-instance", "")
            .map_err(|_| Error::Internal)?;
        Ok(())
    }
    async fn allowed(&self, core: &Core, scope: &Scope, capability: &str) -> bool {
        let Some(id) = self.id else {
            return false;
        };
        let Ok(client) = core.db.get().await else {
            return false;
        };
        authorization::authorize_company(&**client, scope, id, CATALOG_VIEW)
            .await
            .is_ok_and(|c| !c.archived)
            && authorization::authorize_company(&**client, scope, id, capability)
                .await
                .is_ok_and(|c| !c.archived)
    }
    fn query(&self) -> HashMap<String, String> {
        self.id
            .map(|id| HashMap::from([("company_id".into(), id.to_string())]))
            .unwrap_or_default()
    }
    fn field(&self) -> Field {
        select(
            "company_id",
            "Company",
            self.companies
                .iter()
                .map(|c| (c.name.clone(), c.id.to_string()))
                .collect(),
        )
        .id("skills-company")
        .value(self.id.map(|id| id.to_string()).unwrap_or_default())
    }
}

// These Bilimbi lists are not paginated or sortable. ListState and ListColumns
// supply the common table notch and row density without adding a new workflow.
struct Table {
    columns: ListColumns,
    list: ListState,
    event: String,
}
impl Table {
    fn new(id: &str, event: &str, labels: &[(&str, &str)]) -> Result<Self> {
        Ok(Self {
            columns: ListColumns::mount(
                id,
                labels
                    .iter()
                    .map(|(k, l)| BuiltinColumn::new(k, l))
                    .collect(),
            )
            .map_err(|_| Error::Internal)?,
            list: ListState::parse(
                &HashMap::new(),
                ListSpec {
                    omit_defaults: true,
                    ..ListSpec::default()
                }
                .shared()
                .map_err(|_| Error::Internal)?,
            )
            .map_err(|_| Error::Internal)?,
            event: event.into(),
        })
    }
    fn load<T>(&mut self, rows: &[T], key: impl Fn(&T) -> String) {
        self.columns.load(rows, key);
    }
    fn event(&mut self, payload: &Value) {
        self.columns.handle(&params::string_values(payload));
    }
    fn render(
        &self,
        caption: &str,
        rows: Vec<FlexRow>,
        empty_id: &str,
        empty: &str,
    ) -> Result<Html> {
        let none = rows.is_empty();
        let mut table = FlexTable::new(&self.columns.id, self.columns.flex_columns(), rows)
            .sorted(self.list.sort_by.as_deref(), self.list.sort_dir);
        table.event = &self.event;
        table.caption = caption;
        table.zoom = self.columns.zoom;
        table.suggestions = self.columns.flex_suggestions();
        table.add_query = &self.columns.add_query;
        if none {
            table.empty = Some(TableEmpty::State(
                EmptyState::missing(empty, "").id(empty_id),
            ));
        }
        render(&table)
    }
}
#[derive(Default)]
struct Drafts {
    values: HashMap<String, HashMap<String, String>>,
    errors: HashMap<String, Vec<crate::FieldError>>,
}
impl Drafts {
    fn remember(&mut self, form: &str, payload: &Value) {
        self.values.insert(
            form.into(),
            payload
                .as_object()
                .map(|o| {
                    o.iter()
                        .filter_map(|(k, v)| {
                            v.as_str()
                                .map(|v| (k.clone(), v.chars().take(4001).collect()))
                        })
                        .take(64)
                        .collect()
                })
                .unwrap_or_default(),
        );
        self.errors.remove(form);
    }
    fn result<T>(
        &mut self,
        form: &str,
        flash: &mut Flash,
        result: crate::Result<T>,
        success: &str,
        failure: &str,
    ) -> Option<T> {
        match result {
            Ok(value) => {
                self.values.remove(form);
                self.errors.remove(form);
                flash.success(success);
                Some(value)
            }
            Err(error) => {
                let text = if let SkillsError::Invalid(errors) = &error {
                    self.errors.insert(form.into(), errors.clone());
                    failure.to_owned()
                } else if matches!(error, SkillsError::Unauthorized) {
                    FORBIDDEN.into()
                } else {
                    message(&error)
                };
                flash.error(&text);
                None
            }
        }
    }
    fn form(
        &self,
        id: &str,
        event: &str,
        context: i64,
        mut fields: Vec<Field>,
        label: &str,
    ) -> Result<Html> {
        let mut summary = bilibi_ui::ErrorSummary::new(format!("{id}-errors"));
        for field in &mut fields {
            field.id = format!("{id}-{}-{context}", field.name.replace(['[', ']'], "-"));
            if let Some(value) = self.values.get(id).and_then(|v| v.get(&field.name)) {
                field.value.clone_from(value);
                if field.kind == InputKind::Checkbox {
                    field.checked = value == "true";
                }
            }
            let key = field
                .name
                .split('[')
                .next_back()
                .unwrap_or(&field.name)
                .trim_end_matches(']');
            if let Some(errors) = self.errors.get(id) {
                for error in errors.iter().filter(|e| e.field == key) {
                    let text = bilibi_ui::error_sentence(&field.label, &error.message);
                    summary.push(&field.id, &text);
                    field.errors.push(text);
                }
            }
        }
        let mut body = render(&summary)?;
        for field in &fields {
            body.push(&render(field)?);
        }
        render(&Form {
            id,
            event,
            change: false,
            class: "mt-4 grid gap-2 sm:grid-cols-2",
            body,
            actions: render(&Button {
                button_type: "submit",
                variant: ButtonVariant::Primary,
                body: Html::text(label),
                ..Button::default()
            })?,
        })
    }
}
fn render(t: &impl askama::Template) -> Result<Html> {
    Html::render(t).map_err(|_| Error::Internal)
}
fn section(id: &str, title: &str, body: Html) -> Result<Html> {
    let mut html = render(&SectionHeading {
        id,
        title,
        class: "mt-5 mb-2",
        ..SectionHeading::default()
    })?;
    html.push(&render(&Card {
        id: &format!("{id}-card"),
        body,
        ..Card::default()
    })?);
    Ok(html)
}
fn button(event: &str, label: &str, id: impl ToString) -> Result<Html> {
    render(&Button {
        event,
        name: "id",
        value: &id.to_string(),
        body: Html::text(label),
        ..Button::default()
    })
}
fn field(name: &str, label: &str) -> Field {
    Field::named(name).label(label).wrapper_class("mb-0")
}
fn select(name: &str, label: &str, options: Vec<(String, String)>) -> Field {
    field(name, label)
        .kind(InputKind::Select)
        .options(bilibi_ui::choices(&options))
}
fn hidden(name: &str, value: impl ToString) -> Field {
    field(name, "")
        .kind(InputKind::Hidden)
        .value(value.to_string())
}
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or_default()
}
fn id(v: &Value, key: &str) -> Option<i64> {
    params::positive_id(v.get(key).and_then(Value::as_str))
}
fn integer(v: &Value, key: &str) -> i32 {
    text(v, key).trim().parse().unwrap_or(-1)
}
fn optional(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.trim().to_owned())
}
fn row(prefix: &str, id: impl ToString) -> FlexRow {
    FlexRow {
        dom_id: Some(format!("{prefix}-{}", id.to_string())),
        ..FlexRow::new(id.to_string())
    }
}
fn yes(value: bool) -> &'static str {
    if value {
        "Yes"
    } else {
        "No"
    }
}
fn scale_label(scales: &[crate::Scale], id: i64) -> String {
    scales
        .iter()
        .find(|s| s.id == id)
        .map(|s| format!("{} v{}", s.name, s.version))
        .unwrap_or_default()
}
fn published_scales(scales: &[crate::Scale]) -> Vec<(String, String)> {
    scales
        .iter()
        .filter(|s| s.status == "published")
        .map(|s| {
            (
                format!("{} ({}) v{}", s.name, s.code, s.version),
                s.id.to_string(),
            )
        })
        .collect()
}

#[derive(askama::Template)]
#[template(path = "pk4.html")]
struct View<'a> {
    flash: FlashMap<'a>,
    page_id: &'a str,
    title: &'a str,
    subtitle: String,
    company: Option<Field>,
    body: Html,
    confirm: Html,
    profile: Option<String>,
}
#[derive(askama::Template)]
#[template(path = "pk4_confirm.html")]
struct Confirmation<'a> {
    dialog: ConfirmDialog<'a>,
    banner: &'a Html,
}
fn confirmation(company: &Company, verb: &str, detail: &str) -> Result<Html> {
    render(&Confirmation {
        dialog: ConfirmDialog {
            id: "skills-catalog-confirm",
            consequence: verb,
            detail,
            confirm: verb,
            working: "Saving…",
            confirm_event: "confirm",
            cancel_event: "dismiss_confirm",
        },
        banner: &company.banner,
    })
}
