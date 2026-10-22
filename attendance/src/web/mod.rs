//! Bilimbi Attendance.Web.RostersLive and Attendance.Web.ApprovalsLive. The facade owns
//! identity and writes; mounted pages keep only drafts and confirmation intent.
mod approvals;
mod rosters;

use crate::{Attendance, AttendanceError};
use bilibi_database::{Db, Error, Result};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::{LiveState, MountFuture, Page, PageBody, PageFuture, PageRequest},
    params, BuiltinColumn, Button, ButtonVariant, EmptyState, FlashMap, FlexRow, FlexTable, Html,
    ListColumns, ListSpec, ListState, StatusKind, TableEmpty, NO_FLASH,
};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

pub(crate) const PAGES: &str = "people/attendance.pages";
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
    fn title(&self, operation: &str) -> Option<&'static str> {
        match operation {
            "rosters" => Some("Rosters"),
            "approvals" => Some("Attendance approvals"),
            _ => None,
        }
    }
    fn render<'a>(&'a self, request: PageRequest<'a, Scope>) -> PageFuture<'a, Error> {
        Box::pin(async move {
            let scope = request.scope.ok_or(Error::Unauthorized)?;
            let html = match request.operation {
                "rosters" => {
                    let mut page = rosters::Rosters::new(self.0.clone(), request.query)?;
                    page.load(scope, true).await?;
                    page.render()?
                }
                "approvals" => {
                    let mut page = approvals::Approvals::new(self.0.clone(), request.query)?;
                    page.load(scope, true).await?;
                    page.render()?
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
            let page: Box<dyn LiveState<Scope, Error>> = match request.operation {
                "rosters" => Box::new(rosters::Rosters::new(self.0.clone(), request.query)?),
                "approvals" => Box::new(approvals::Approvals::new(self.0.clone(), request.query)?),
                _ => return Ok(None),
            };
            Ok(Some(page))
        })
    }
}

#[derive(Default)]
struct Flash(Option<(StatusKind, String)>);
impl Flash {
    fn success(&mut self, text: &str) {
        self.0 = Some((StatusKind::Success, text.into()));
    }
    fn error(&mut self, text: &str) {
        self.0 = Some((StatusKind::Error, text.into()));
    }
    fn clear(&mut self, payload: &Value) {
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
                StatusKind::Error => map.error = text,
                StatusKind::Warning => map.warning = text,
                StatusKind::Info => map.info = text,
            }
        }
        map
    }
}

/// Bilimbi bounds these tables without sorting or paging. The roster keeps
/// its company, period and search in its outer ListState; this adapter owns
/// only column arrangement and compact rows.
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
                    .map(|(key, label)| BuiltinColumn::new(key, label))
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
    fn render(&self, caption: &str, rows: Vec<FlexRow>, empty: EmptyState) -> Result<Html> {
        let none = rows.is_empty();
        let mut table = FlexTable::new(&self.columns.id, self.columns.flex_columns(), rows)
            .sorted(self.list.sort_by.as_deref(), self.list.sort_dir);
        table.event = self.event;
        table.caption = caption;
        table.zoom = self.columns.zoom;
        table.suggestions = self.columns.flex_suggestions();
        table.add_query = &self.columns.add_query;
        if none {
            table.empty = Some(TableEmpty::State(empty));
        }
        render(&table)
    }
}
fn render(template: &impl askama::Template) -> Result<Html> {
    Html::render(template).map_err(|_| Error::Internal)
}
fn submit(label: &str) -> Result<Html> {
    render(&Button {
        button_type: "submit",
        variant: ButtonVariant::Primary,
        body: Html::text(label),
        ..Button::default()
    })
}
fn text(payload: &Value, key: &str) -> String {
    payload
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}
/// Expected refusals stay within the page; faults still reach the host log.
fn refusal(error: AttendanceError) -> Result<AttendanceError> {
    match error {
        AttendanceError::Failed(error) => Err(error),
        other => Ok(other),
    }
}
fn row(prefix: &str, id: i64) -> FlexRow {
    FlexRow {
        dom_id: Some(format!("{prefix}-{id}")),
        ..FlexRow::new(id.to_string())
    }
}
