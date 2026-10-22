//! Bilimbi Skills.Web.ActionsLive and Skills.Web.PolicyLive.
//! These pages call the one Skills facade; drafts and confirmations live here.
mod actions;
mod policy;
use crate::{Skills, SkillsError};
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

pub(crate) const PAGES: &str = "people/skills.action-policy-pages";
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
}
impl Page<Scope, Error> for Pages {
    fn title(&self, operation: &str) -> Option<&'static str> {
        match operation {
            "actions" => Some("Development actions"),
            "policy" => Some("Skills policy"),
            _ => None,
        }
    }
    fn render<'a>(&'a self, request: PageRequest<'a, Scope>) -> PageFuture<'a, Error> {
        Box::pin(async move {
            let scope = request.scope.ok_or(Error::Unauthorized)?;
            let html = match request.operation {
                "actions" => {
                    let mut p = actions::Actions::new(self.0.clone(), request.query)?;
                    p.load(scope, true).await?;
                    p.render()?
                }
                "policy" => {
                    let mut p = policy::PolicyPage::new(self.0.clone(), request.query)?;
                    p.load(scope, true).await?;
                    p.render()?
                }
                _ => return Err(Error::NotFound),
            };
            Ok(PageBody {
                html,
                live: true,
                title: None,
            })
        })
    }
    fn mount<'a>(&'a self, request: PageRequest<'a, Scope>) -> MountFuture<'a, Scope, Error> {
        Box::pin(async move {
            let state: Box<dyn LiveState<Scope, Error>> = match request.operation {
                "actions" => Box::new(actions::Actions::new(self.0.clone(), request.query)?),
                "policy" => Box::new(policy::PolicyPage::new(self.0.clone(), request.query)?),
                _ => return Ok(None),
            };
            Ok(Some(state))
        })
    }
}

#[derive(Default)]
struct Flash(Option<(StatusKind, String)>);
impl Flash {
    fn success(&mut self, message: &str) {
        self.0 = Some((StatusKind::Success, message.into()));
    }
    fn error(&mut self, message: &str) {
        self.0 = Some((StatusKind::Error, message.into()));
    }
    fn result<T>(&mut self, result: crate::Result<T>, success: &str) -> bool {
        match result {
            Ok(_) => {
                self.success(success);
                true
            }
            Err(e) => {
                self.error(&message(&e));
                false
            }
        }
    }
    fn clear(&mut self, payload: &Value) {
        if self
            .0
            .as_ref()
            .is_some_and(|(k, _)| Some(k.name()) == payload.get("key").and_then(Value::as_str))
        {
            self.0 = None;
        }
    }
    fn map(&self) -> FlashMap<'_> {
        let mut map = NO_FLASH;
        if let Some((kind, text)) = &self.0 {
            match kind {
                StatusKind::Success => map.success = text,
                StatusKind::Error => map.error = text,
                StatusKind::Info => map.info = text,
                StatusKind::Warning => map.warning = text,
            }
        }
        map
    }
}
fn message(e: &SkillsError) -> String {
    match e {
        SkillsError::Failed(_) => "That could not be done.".into(),
        SkillsError::NotCurrent(_) => {
            "The workforce is not current, so nothing was changed.".into()
        }
        _ => e.to_string(),
    }
}

/// Bilimbi's bounded record lists have neither server sorting nor paging.
/// Each retains those bounds, with the shared column picker and row density.
struct Table {
    columns: ListColumns,
    list: ListState,
    event: &'static str,
}
impl Table {
    fn new(id: &str, event: &'static str, labels: &[(&str, &str)]) -> Result<Self> {
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
            event,
        })
    }
    fn load<T>(&mut self, rows: &[T], key: impl Fn(&T) -> String) {
        self.columns.load(rows, key);
    }
    fn event(&mut self, payload: &Value) {
        self.columns.handle(&params::string_values(payload));
    }
    fn compact(&self) -> bool {
        self.columns.mode() == bilibi_ui::flex_table::RowMode::Compact
    }
    fn render(&self, caption: &str, rows: Vec<FlexRow>, empty: &str) -> Result<Html> {
        let none = rows.is_empty();
        let mut t = FlexTable::new(&self.columns.id, self.columns.flex_columns(), rows)
            .sorted(self.list.sort_by.as_deref(), self.list.sort_dir);
        t.event = self.event;
        t.caption = caption;
        t.zoom = self.columns.zoom;
        t.suggestions = self.columns.flex_suggestions();
        t.add_query = &self.columns.add_query;
        if none {
            t.empty = Some(TableEmpty::State(EmptyState::missing(empty, "").id(
                match self.columns.id.as_str() {
                    "actions" => "actions-empty".to_owned(),
                    "action-types" => "action-types-none".to_owned(),
                    "reminders-due" => "reminders-none".to_owned(),
                    _ => format!("{}-empty", self.columns.id),
                },
            )));
        }
        render(&t)
    }
}
fn render(t: &impl askama::Template) -> Result<Html> {
    Html::render(t).map_err(|_| Error::Internal)
}
fn section(id: &str, title: &str, body: Html) -> Result<Html> {
    let mut h = render(&SectionHeading {
        id,
        title,
        class: "mt-5 mb-2",
        ..SectionHeading::default()
    })?;
    h.push(&render(&Card {
        id: &format!("{id}-card"),
        body,
        ..Card::default()
    })?);
    Ok(h)
}
fn button(event: &str, label: &str, id: i64) -> Result<Html> {
    render(&Button {
        event,
        name: "id",
        value: &id.to_string(),
        body: Html::text(label),
        ..Button::default()
    })
}
fn form(id: &str, event: &str, fields: Vec<Field>, label: &str) -> Result<Html> {
    form_with_class(id, event, fields, label, "grid gap-2 sm:grid-cols-2")
}
fn form_with_class(
    id: &str,
    event: &str,
    mut fields: Vec<Field>,
    label: &str,
    class: &str,
) -> Result<Html> {
    let mut summary = bilibi_ui::ErrorSummary::new(format!("{id}-errors"));
    for f in &mut fields {
        if f.kind.is_hidden() {
            f.id = format!("{id}-{}", f.name);
        }
        f.errors = f
            .errors
            .iter()
            .map(|error| bilibi_ui::error_sentence(&f.label, error))
            .collect();
        for error in &f.errors {
            summary.push(&f.id, error);
        }
    }
    let mut body = render(&summary)?;
    body.push(
        &fields
            .iter()
            .map(render)
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .collect(),
    );
    render(&Form {
        id,
        event,
        change: false,
        class,
        body,
        actions: render(&Button {
            button_type: "submit",
            variant: ButtonVariant::Primary,
            body: Html::text(label),
            ..Button::default()
        })?,
    })
}
// Dirty controls belong to the company in which they were typed. Changing
// their key makes the shared patcher replace them when the company changes.
fn company_fields(company: Option<i64>, mut fields: Vec<Field>) -> Vec<Field> {
    for f in &mut fields {
        f.id = format!("{}-company-{}", f.id, company.unwrap_or_default());
    }
    fields
}

fn field(name: &str, label: &str) -> Field {
    Field::named(name)
        .id(name.replace(['[', ']'], "-"))
        .label(label)
        .wrapper_class("mb-0")
}
fn select(name: &str, label: &str, options: Vec<(String, String)>) -> Field {
    field(name, label)
        .kind(InputKind::Select)
        .options(bilibi_ui::choices(&options))
}
fn hidden(name: &str, value: i64) -> Field {
    field(name, "")
        .kind(InputKind::Hidden)
        .value(value.to_string())
}
fn row(id: &str, key: i64) -> FlexRow {
    FlexRow {
        dom_id: Some(format!("{id}-{key}")),
        ..FlexRow::new(key.to_string())
    }
}
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or_default()
}
fn id(v: &Value, key: &str) -> Option<i64> {
    params::positive_id(v.get(key).and_then(Value::as_str))
}
fn label(value: &str) -> String {
    let s = value.replace('_', " ");
    let mut c = s.chars();
    c.next()
        .map(|x| x.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}
fn optional(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.trim().to_owned())
}
fn date(value: &str) -> crate::Result<Option<jiff::civil::Date>> {
    if value.trim().is_empty() {
        Ok(None)
    } else {
        value
            .parse()
            .map(Some)
            .map_err(|_| SkillsError::InvalidDate)
    }
}
fn key() -> String {
    format!("action:{}", jiff::Timestamp::now())
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
    async fn load(&mut self, core: &Core, scope: &Scope, capability: &str) -> Result<()> {
        let client = core.db.get().await?;
        self.companies = authorization::selectable_companies(&**client, scope, capability)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|c| !c.archived)
            .collect();
        if self.id.is_none() {
            self.id = self.companies.first().map(|c| c.id);
        }
        self.selected = self
            .companies
            .iter()
            .find(|c| Some(c.id) == self.id)
            .cloned();
        self.banner = core
            .instances
            .resolve()
            .await?
            .banner("skills-confirm-instance", "")
            .map_err(|_| Error::Internal)?;
        Ok(())
    }
    async fn allowed(&self, core: &Core, scope: &Scope, capability: &str) -> bool {
        let Some(company) = self.selected.as_ref() else {
            return false;
        };
        let Ok(client) = core.db.get().await else {
            return false;
        };
        authorization::authorize_company(&**client, scope, company.id, capability)
            .await
            .is_ok_and(|c| !c.archived)
    }
    fn field(&self, prefix: &str) -> Field {
        select(
            "company_id",
            "Company",
            self.companies
                .iter()
                .map(|c| (c.name.clone(), c.id.to_string()))
                .collect(),
        )
        .id(format!("{prefix}-company"))
        .value(self.id.map(|id| id.to_string()).unwrap_or_default())
    }
    fn query(&self) -> HashMap<String, String> {
        self.id
            .map(|id| HashMap::from([("company_id".into(), id.to_string())]))
            .unwrap_or_default()
    }
}
fn confirm(
    pending: bool,
    company: &Company,
    consequence: &str,
    detail: &str,
    verb: &str,
) -> Result<Html> {
    if !pending {
        return Ok(Html::EMPTY);
    }
    render(&Confirmation {
        dialog: ConfirmDialog {
            id: "skills-confirm",
            consequence,
            detail,
            confirm: verb,
            working: "Saving…",
            confirm_event: "confirm",
            cancel_event: "dismiss_confirm",
        },
        banner: &company.banner,
    })
}

#[derive(askama::Template)]
#[template(path = "pk6.html")]
struct View<'a> {
    flash: FlashMap<'a>,
    page_id: &'a str,
    title: &'a str,
    subtitle: String,
    has_company: bool,
    company_form: &'a str,
    company: Field,
    body: Html,
    confirm: Html,
}

// ConfirmDialog has no banner slot at the pinned Platform revision. Compose
// its typed contract with the shared modal and buttons so identity stays inside.
#[derive(askama::Template)]
#[template(path = "pk6_confirm.html")]
struct Confirmation<'a> {
    dialog: ConfirmDialog<'a>,
    banner: &'a Html,
}

fn record_facts(id: &str, fields: &[(&str, String)]) -> Result<Html> {
    render(&bilibi_ui::Facts {
        id,
        items: fields
            .iter()
            .map(|(title, value)| bilibi_ui::Fact {
                title,
                value: Html::text(value),
                ..bilibi_ui::Fact::default()
            })
            .collect(),
        ..bilibi_ui::Facts::default()
    })
}
