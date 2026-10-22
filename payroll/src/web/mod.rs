//! Bilimbi Payroll.Web.SetupLive and AttendanceMappingsLive. Pages keep drafts
//! and confirmation intent; the public Payroll facade owns every business write.
mod attendance;
mod setup;
use crate::{
    AttendanceAllowances, Payroll, PayrollError, Setup, Sources, ATTENDANCE_MAPPINGS_MANAGE,
    MANAGE, VIEW,
};
use bilibi_database::{Db, Error, Result};
use bilibi_people_workforce::{authorization, CompanySummary};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::*, params, BuiltinColumn, Button, ButtonVariant, EmptyState, Field, FlashMap, FlexRow,
    FlexTable, Form, Html, InputKind, ListColumns, ListSpec, ListState, StatusKind, TableEmpty,
    NO_FLASH,
};
use jiff::civil::Date;
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};
pub(crate) const PAGES: &str = "people/payroll.setup_pages";
const NO_PERMISSION: &str = "You cannot change this company's payroll setup.";
struct Core {
    db: Db,
    payroll: Arc<Payroll>,
    display: bilibi_datetime::DisplayPolicy,
}
pub(crate) struct Pages(Arc<Core>);
impl Pages {
    pub(crate) fn new(db: Db, payroll: Arc<Payroll>) -> Self {
        let display = bilibi_datetime::DisplayPolicy::new(payroll.settings.clone());
        Self(Arc::new(Core {
            db,
            payroll,
            display,
        }))
    }
}
impl Page<Scope, Error> for Pages {
    fn title(&self, operation: &str) -> Option<&'static str> {
        match operation {
            "setup" | "mappings" => Some("Payroll setup"),
            "attendance_mappings" => Some("Attendance pay-item mappings"),
            _ => None,
        }
    }
    fn render<'a>(&'a self, request: PageRequest<'a, Scope>) -> PageFuture<'a, Error> {
        Box::pin(async move {
            let mut state = State::new(self.0.clone(), request.operation, request.query)?;
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
            Ok(Some(Box::new(State::new(
                self.0.clone(),
                request.operation,
                request.query,
            )?) as Box<dyn LiveState<Scope, Error>>))
        })
    }
}
#[derive(Default)]
struct Flash(Option<(StatusKind, String)>);
impl Flash {
    fn error(&mut self, text: &str) {
        self.0 = Some((StatusKind::Error, text.into()));
    }
    fn success(&mut self, text: &str) {
        self.0 = Some((StatusKind::Success, text.into()));
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
    fn dismiss(&mut self, value: &Value) {
        if self
            .0
            .as_ref()
            .is_some_and(|(kind, _)| Some(kind.name()) == value.get("key").and_then(Value::as_str))
        {
            self.0 = None;
        }
    }
}
struct Table {
    columns: ListColumns,
    list: ListState,
    event: String,
}
impl Table {
    fn new(id: &str, labels: &[(&str, &str)]) -> Result<Self> {
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
            .map_err(|_| Error::Invalid)?,
            event: format!("{id}_grid"),
        })
    }
    fn render(
        &self,
        caption: &str,
        rows: Vec<FlexRow>,
        empty: &str,
        empty_id: &str,
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
struct Draft {
    values: HashMap<String, String>,
    errors: Vec<crate::FieldError>,
}
enum Pending {
    Freeze {
        company: i64,
        period: i64,
        code: String,
        currency: String,
    },
    Lock {
        company: i64,
        run: i64,
    },
}
struct State {
    core: Arc<Core>,
    operation: &'static str,
    company_id: Option<i64>,
    companies: Vec<CompanySummary>,
    company: Option<CompanySummary>,
    setup: Option<Setup>,
    sources: Sources,
    attendance: Option<AttendanceAllowances>,
    can_manage: bool,
    can_map: bool,
    pending: Option<Pending>,
    flash: Flash,
    drafts: HashMap<String, Draft>,
    tables: HashMap<&'static str, Table>,
    today: Date,
    display: bilibi_datetime::DisplayContext,
}
impl State {
    fn new(
        core: Arc<Core>,
        operation: &'static str,
        query: &HashMap<String, String>,
    ) -> Result<Self> {
        if !["setup", "mappings", "attendance_mappings"].contains(&operation) {
            return Err(Error::NotFound);
        }
        let mut tables = HashMap::new();
        for (key, cols) in [
            (
                "classifications",
                vec![
                    ("code", "Code"),
                    ("name", "Name"),
                    ("effective", "Effective"),
                ],
            ),
            (
                "items",
                vec![
                    ("code", "Code"),
                    ("name", "Name"),
                    ("amount", "Amount"),
                    ("currency", "Currency"),
                    ("effective", "Effective"),
                ],
            ),
            (
                "periods",
                vec![
                    ("code", "Period code"),
                    ("starts_on", "From"),
                    ("ends_on", "To"),
                    ("pay_on", "Pay on"),
                ],
            ),
            (
                "mappings",
                vec![
                    ("source", "Source"),
                    ("item", "Pay item"),
                    ("effective", "Effective"),
                ],
            ),
            (
                "runs",
                vec![
                    ("period", "Period"),
                    ("currency", "Currency"),
                    ("country", "Country"),
                    ("unmapped", "Unmapped"),
                    ("locked", "Locked"),
                    ("actions", "Actions"),
                ],
            ),
            (
                "rules",
                vec![
                    ("rule", "Attendance rule"),
                    ("value", "Value"),
                    ("effective", "Effective"),
                    ("items", "Pay items today"),
                ],
            ),
            (
                "history",
                vec![
                    ("rule", "Attendance rule"),
                    ("item", "Pay item"),
                    ("effective", "Effective"),
                ],
            ),
        ] {
            tables.insert(key, Table::new(&format!("payroll-{key}-table"), &cols)?);
        }
        Ok(Self {
            core,
            operation,
            company_id: params::positive_id(query.get("company_id").map(String::as_str)),
            companies: vec![],
            company: None,
            setup: None,
            sources: Sources::new(),
            attendance: None,
            can_manage: false,
            can_map: false,
            pending: None,
            flash: Flash::default(),
            drafts: HashMap::new(),
            tables,
            display: bilibi_datetime::DisplayContext::anonymous(),
            today: jiff::Timestamp::now()
                .to_zoned(jiff::tz::TimeZone::UTC)
                .date(),
        })
    }
    async fn load(&mut self, scope: &Scope) -> Result<()> {
        let c = self.core.db.get().await?;
        let cap = if self.operation == "attendance_mappings" {
            ATTENDANCE_MAPPINGS_MANAGE
        } else {
            VIEW
        };
        self.companies = match authorization::selectable_companies(&**c, scope, cap).await {
            Ok(companies) => companies.into_iter().filter(|c| !c.archived).collect(),
            Err(bilibi_people_workforce::WorkforceError::Failed(e)) => return Err(e),
            Err(_) => vec![],
        };
        if self.company_id.is_none() {
            self.company_id = self.companies.first().map(|c| c.id);
        }
        self.company = self
            .companies
            .iter()
            .find(|c| Some(c.id) == self.company_id)
            .cloned();
        self.display = self
            .core
            .display
            .display(
                &**c,
                scope,
                self.company
                    .as_ref()
                    .and_then(|c| bilibi_party_ids::OrganisationId::new(c.id).ok()),
                "en",
            )
            .await?;
        drop(c);
        self.setup = None;
        self.attendance = None;
        self.sources.clear();
        self.can_manage = false;
        self.can_map = false;
        let Some(company) = &self.company else {
            self.pending = None;
            return Ok(());
        };
        self.can_map = self
            .core
            .payroll
            .allowed(scope, company.id, ATTENDANCE_MAPPINGS_MANAGE)
            .await;
        if self.operation == "attendance_mappings" {
            self.attendance = self
                .core
                .payroll
                .attendance_allowances(scope, company.id, self.today)
                .await
                .ok();
            if !self.can_map || self.attendance.is_none() {
                self.pending = None;
            }
        } else {
            self.can_manage = self.core.payroll.allowed(scope, company.id, MANAGE).await;
            self.setup = self.core.payroll.setup(scope, company.id).await.ok();
            self.sources = self
                .core
                .payroll
                .sources(scope, company.id)
                .await
                .unwrap_or_default();
            if !self.can_manage || self.setup.is_none() {
                self.pending = None;
            }
        }
        Ok(())
    }
    async fn authorized(&mut self, scope: &Scope) -> Option<i64> {
        let cap = if self.operation == "attendance_mappings" {
            ATTENDANCE_MAPPINGS_MANAGE
        } else {
            MANAGE
        };
        if let Some(company) = self.company_id {
            if self.core.payroll.allowed(scope, company, cap).await {
                return Some(company);
            }
        }
        self.pending = None;
        self.flash
            .error(if self.operation == "attendance_mappings" {
                "Choose an allowance rule and a pay item effective for the whole mapping period."
            } else {
                NO_PERMISSION
            });
        None
    }
    fn outcome(&mut self, event: &str, result: crate::Result<()>) {
        match result {
            Ok(()) => {
                self.drafts.remove(event);
                self.flash
                    .success(if self.operation == "attendance_mappings" {
                        "Attendance allowance mapped."
                    } else {
                        "Payroll setup saved."
                    });
            }
            Err(error) => {
                let message = if self.operation == "attendance_mappings" {
                    if matches!(error, PayrollError::OverlappingVersion) {
                        "This rule already has a pay item for those dates.".into()
                    } else {
                        "Choose an allowance rule and a pay item effective for the whole mapping period.".into()
                    }
                } else {
                    match &error {
                        PayrollError::Failed(_) => {
                            "Payroll setup could not be saved. Try again.".into()
                        }
                        _ => error.to_string(),
                    }
                };
                if let PayrollError::Invalid(fields) = error {
                    self.drafts.entry(event.into()).or_default().errors = fields;
                }
                self.flash.error(&message);
            }
        }
    }
    async fn event(&mut self, scope: &Scope, payload: &Value) -> Result<()> {
        let event = text(payload, "type");
        match event.as_str() {
            "clear-flash" => {
                self.flash.dismiss(payload);
                return Ok(());
            }
            "cancel_lock" | "cancel_freeze" => {
                self.pending = None;
                return Ok(());
            }
            "select_company" => {
                self.load(scope).await?;
                let chosen = params::positive_id(payload.get("company_id").and_then(Value::as_str));
                if self.companies.iter().any(|c| Some(c.id) == chosen) {
                    self.company_id = chosen;
                    self.pending = None;
                    self.drafts.clear();
                }
                return Ok(());
            }
            _ => {}
        }
        if let Some(table) = self.tables.values_mut().find(|t| t.event == event) {
            table.columns.handle(&params::string_values(payload));
            return Ok(());
        }
        let writes = if self.operation == "attendance_mappings" {
            &["create"][..]
        } else {
            &[
                "save_settings",
                "create_classification",
                "create_item",
                "create_period",
                "create_mapping",
                "create_run",
                "confirm_freeze",
                "prepare_lock",
                "lock_run",
            ][..]
        };
        if !writes.contains(&event.as_str()) {
            return Err(Error::Invalid);
        }
        let Some(company) = self.authorized(scope).await else {
            return Ok(());
        };
        match event.as_str() {
            "prepare_lock" => {
                self.load(scope).await?;
                let id = params::positive_id(payload.get("id").and_then(Value::as_str));
                self.pending = self
                    .setup
                    .as_ref()
                    .and_then(|s| {
                        s.runs
                            .iter()
                            .find(|r| Some(r.id) == id && r.locked_at.is_none())
                    })
                    .map(|r| Pending::Lock { company, run: r.id });
                if self.pending.is_none() {
                    self.flash
                        .error("Choose a configured country, currency and unused period.");
                }
            }
            "lock_run" => {
                if let Some(Pending::Lock {
                    company: selected,
                    run,
                }) = self.pending.take()
                {
                    let result = if selected == company {
                        self.core
                            .payroll
                            .lock_run(scope, company, run)
                            .await
                            .map(|_| ())
                    } else {
                        Err(PayrollError::RunUnavailable)
                    };
                    self.outcome(&event, result);
                } else {
                    self.outcome(&event, Err(PayrollError::RunUnavailable));
                }
            }
            "confirm_freeze" => {
                if let Some(Pending::Freeze {
                    company: selected,
                    period,
                    currency,
                    ..
                }) = self.pending.take()
                {
                    let result = if selected == company {
                        self.core
                            .payroll
                            .freeze_setup(scope, company, period, &currency)
                            .await
                            .map(|_| ())
                    } else {
                        Err(PayrollError::RunUnavailable)
                    };
                    self.outcome("create_run", result);
                } else {
                    self.outcome(&event, Err(PayrollError::RunUnavailable));
                }
            }
            "create_run" => {
                let draft = Draft::read(payload, "");
                let period = draft.id("period_id");
                let currency = draft.get("currency").to_owned();
                self.drafts.insert(event.clone(), draft);
                self.load(scope).await?;
                self.pending = self.setup.as_ref().and_then(|s| {
                    let p = s.periods.iter().find(|p| Some(p.id) == period)?;
                    if s.country.is_empty()
                        || !s.currencies.contains(&currency)
                        || s.runs
                            .iter()
                            .any(|r| r.period_id == p.id && r.currency == currency)
                    {
                        return None;
                    }
                    Some(Pending::Freeze {
                        company,
                        period: p.id,
                        code: p.code.clone(),
                        currency,
                    })
                });
                if self.pending.is_none() {
                    self.outcome(&event, Err(PayrollError::RunUnavailable));
                }
            }
            _ => {
                let namespace = match event.as_str() {
                    "save_settings" => "",
                    "create" => "mapping",
                    _ => "record",
                };
                self.drafts
                    .insert(event.clone(), Draft::read(payload, namespace));
                let result = self.write(scope, company, &event).await;
                self.outcome(&event, result);
            }
        }
        Ok(())
    }
    fn render(&self) -> Result<String> {
        if self.operation == "attendance_mappings" {
            self.render_attendance()
        } else {
            self.render_setup()
        }
    }
    fn company_field(&self, id: &str) -> Field {
        Field::named("company_id")
            .id(id)
            .label("Company")
            .kind(InputKind::Select)
            .wrapper_class("mb-0 max-w-md")
            .options(
                self.companies
                    .iter()
                    .map(|c| bilibi_ui::Choice::new(&c.name, c.id.to_string()))
                    .collect(),
            )
            .value(self.company_id.map_or_else(String::new, |c| c.to_string()))
    }
}
impl LiveState<Scope, Error> for State {
    fn update<'a>(&'a mut self, request: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            match request.input {
                LiveInput::Event(payload) => self.event(request.scope, payload).await?,
                LiveInput::Params(query) => {
                    self.company_id =
                        params::positive_id(query.get("company_id").map(String::as_str));
                    self.pending = None;
                    self.drafts.clear();
                }
                LiveInput::Refresh => {}
            }
            self.load(request.scope).await?;
            Ok(LiveBody {
                html: self.render()?,
                query: self
                    .company_id
                    .map(|id| HashMap::from([("company_id".into(), id.to_string())]))
                    .unwrap_or_default(),
                ..LiveBody::default()
            })
        })
    }
}
impl Draft {
    // Payroll setup has more than 64 controls. Read the submitting form's
    // namespace directly instead of truncating with params::form_values.
    fn read(payload: &Value, namespace: &str) -> Self {
        let mut values = HashMap::new();
        if let Some(object) = payload.as_object() {
            for (key, value) in object {
                let key = if namespace.is_empty() {
                    Some(key.as_str())
                } else {
                    key.strip_prefix(&format!("{namespace}["))
                        .and_then(|k| k.strip_suffix(']'))
                };
                if let (Some(key), Some(value)) = (key, value.as_str()) {
                    if key != "type" && value.len() <= 4096 {
                        values.insert(key.into(), value.into());
                    }
                }
            }
        }
        Self {
            values,
            errors: vec![],
        }
    }
    fn get(&self, key: &str) -> &str {
        self.values.get(key).map(String::as_str).unwrap_or_default()
    }
    fn id(&self, key: &str) -> Option<i64> {
        params::positive_id(Some(self.get(key)))
    }
    fn date(&self, key: &'static str) -> crate::Result<Date> {
        self.get(key).parse().map_err(|_| {
            PayrollError::Invalid(vec![crate::FieldError::new(key, "must be a valid date")])
        })
    }
    fn until(&self) -> crate::Result<Option<Date>> {
        if self.get("effective_to").is_empty() {
            Ok(None)
        } else {
            self.date("effective_to").map(Some)
        }
    }
}
fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .into()
}
fn render(template: &impl askama::Template) -> Result<Html> {
    Html::render(template).map_err(|_| Error::Internal)
}
fn row(prefix: &str, id: i64) -> FlexRow {
    FlexRow {
        dom_id: Some(format!("{prefix}-{id}")),
        ..FlexRow::new(id.to_string())
    }
}
fn effective(from: &str, to: Option<&str>) -> String {
    format!("{from} to {}", to.unwrap_or("open"))
}
fn submit(label: &str) -> Result<Html> {
    render(&Button {
        button_type: "submit",
        variant: ButtonVariant::Primary,
        body: Html::text(label),
        ..Button::default()
    })
}
