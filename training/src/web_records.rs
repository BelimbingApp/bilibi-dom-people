//! Bilimbi Training.Web.RecordsLive and PassportLive: attendance and learning passports.
//! The mounted state owns evidence selection, upload bytes and retention confirmations.
use crate::*;
use askama::Template;
use bilibi_artifacts::PurgeHold;
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
    params,
    upload::{UploadSpec, Uploads},
    ActionLink, BuiltinColumn, Button, ButtonVariant, Choice, ConfirmDialog, EmptyState,
    ErrorSummary, Field, Filter, FilterToolbar, FlashMap, FlexRow, FlexTable, Html, InputKind,
    ListColumns, ListSpec, ListState, Pagination, StatusKind, Tab, TableEmpty, Tabs,
    ToolbarControl, NO_FLASH,
};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

pub(crate) const PAGES: &str = "people/training.records_pages";
const NO_WRITE: &str = "You cannot change training records.";
const NO_GENERATE: &str = "You cannot generate this passport. Current workforce and document generation access are required.";
const PROVENANCE: Provenance = Provenance {
    module: MODULE,
    source: file!(),
    line: line!(),
};
inventory::submit! {
    Contribution::new(PAGE_WRITES, PAGES, PROVENANCE, &PageWrites {
        service: PAGES, contracts: &[], pending: Pending::Every { reason: "Attendance, evidence, retention and passport generation call Training; their operation contracts are not yet reviewed." },
    })
}
macro_rules! route {
    ($key:literal,$path:literal,$cap:expr) => {
        inventory::submit! {
            Contribution::new(ROUTES, concat!("people/training.records_", $key), PROVENANCE, &Route {
                method: Method::Get, path: $path, service: PAGES, operation: $key,
                access: Access::Capability($cap), framed: true,
            })
        }
    };
}
route!(
    "records",
    "/people/training/records",
    RECORDS_WORKSPACE_VIEW
);
route!("my", "/people/training/records/my", PASSPORT_MY_VIEW);
route!("team", "/people/training/records/team", PASSPORT_TEAM_VIEW);
bilibi_menu::contribute!(
    "people/training",
    MenuItem::page(
        "people.development.training_records",
        "Training records",
        "/people/training/records"
    )
    .icon("clipboard-document-list")
    .under("people.development")
    .requires(Requirement::Capability(RECORDS_WORKSPACE_VIEW))
    .order(80)
);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Task {
    Records,
    My,
    Team,
}
impl Task {
    fn parse(operation: &str) -> DbResult<Self> {
        match operation {
            "records" => Ok(Self::Records),
            "my" => Ok(Self::My),
            "team" => Ok(Self::Team),
            _ => Err(Error::NotFound),
        }
    }
    fn capability(self) -> &'static str {
        match self {
            Self::Records => RECORDS_WORKSPACE_VIEW,
            Self::My => PASSPORT_MY_VIEW,
            Self::Team => PASSPORT_TEAM_VIEW,
        }
    }
    fn path(self) -> &'static str {
        match self {
            Self::Records => "/people/training/records",
            Self::My => "/people/training/records/my",
            Self::Team => "/people/training/records/team",
        }
    }
    fn audience(self) -> PassportAudience {
        if self == Self::Team {
            PassportAudience::Team
        } else {
            PassportAudience::SelfService
        }
    }
}
pub(crate) struct Pages(pub Arc<Training>);
impl Page<Scope, Error> for Pages {
    fn title(&self, _: &str) -> Option<&'static str> {
        Some("Training records")
    }
    fn render<'a>(&'a self, req: PageRequest<'a, Scope>) -> PageFuture<'a, Error> {
        Box::pin(async move {
            let mut state = Records::new(self.0.clone(), Task::parse(req.operation)?, req.query)?;
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
            let mut state = Records::new(self.0.clone(), Task::parse(req.operation)?, req.query)?;
            state.load(req.scope.ok_or(Error::Unauthorized)?).await?;
            Ok(Some(Box::new(state) as Box<dyn LiveState<Scope, Error>>))
        })
    }
}
fn spec() -> DbResult<Arc<ListSpec>> {
    ListSpec {
        page_size_param: "perPage".into(),
        filters: BTreeMap::from([
            ("company_id".into(), Filter::string("")),
            ("session_id".into(), Filter::string("")),
            ("employee_id".into(), Filter::string("")),
        ]),
        omit_blank: vec![
            "company_id".into(),
            "session_id".into(),
            "employee_id".into(),
        ],
        ..ListSpec::default()
    }
    .shared()
    .map_err(|_| Error::Internal)
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
#[derive(Clone)]
struct Retention {
    company: i64,
    artifact: Option<String>,
}
struct Records {
    training: Arc<Training>,
    task: Task,
    list: ListState,
    columns: BTreeMap<&'static str, ListColumns>,
    companies: Vec<bilibi_people_workforce::CompanySummary>,
    company: Option<i64>,
    flags: HashMap<&'static str, bool>,
    sessions: Vec<Session>,
    session: Option<i64>,
    history: Vec<AttendanceFact>,
    selected_fact: Option<i64>,
    evidence: Vec<Evidence>,
    holds: Vec<PurgeHold>,
    attendance_employees: Vec<EmployeeOption>,
    passport_employees: Vec<bilibi_people_workforce::Employee>,
    employee: Option<i64>,
    passport: Option<Passport>,
    document: Option<String>,
    uploads: Option<Uploads>,
    pending: Option<Retention>,
    draft: HashMap<String, String>,
    summary: ErrorSummary,
    error: String,
    warning: String,
    flash: Option<(StatusKind, String)>,
    clock: Option<bilibi_datetime::DisplayContext>,
    instance_notice: String,
}
impl Records {
    fn new(training: Arc<Training>, task: Task, query: &HashMap<String, String>) -> DbResult<Self> {
        let mut tables = BTreeMap::new();
        for (id, labels) in [
            (
                "sessions",
                vec![
                    ("name", "Session"),
                    ("starts", "Starts"),
                    ("ends", "Ends"),
                    ("capacity", "Capacity"),
                    ("action", "Action"),
                ],
            ),
            (
                "history",
                vec![
                    ("employee", "Employee"),
                    ("revision", "Revision"),
                    ("status", "Attendance"),
                    ("reason", "Reason"),
                    ("actor", "Recorded by"),
                    ("time", "Recorded"),
                    ("evidence", "Evidence"),
                ],
            ),
            ("evidence", vec![("document", "Document")]),
            (
                "holds",
                vec![
                    ("document", "Document"),
                    ("reason", "Reason"),
                    ("attempts", "Attempts"),
                    ("action", "Action"),
                ],
            ),
            (
                "passport",
                vec![
                    ("course", "Course"),
                    ("session", "Session"),
                    ("completed", "Completed"),
                    ("attendance", "Attendance"),
                    ("evidence", "Evidence"),
                ],
            ),
        ] {
            tables.insert(id, columns(id, &labels)?);
        }
        Ok(Self {
            training,
            task,
            list: ListState::parse(query, spec()?).map_err(|_| Error::Internal)?,
            columns: tables,
            companies: vec![],
            company: None,
            flags: HashMap::new(),
            sessions: vec![],
            session: None,
            history: vec![],
            selected_fact: None,
            evidence: vec![],
            holds: vec![],
            attendance_employees: vec![],
            passport_employees: vec![],
            employee: None,
            passport: None,
            document: None,
            uploads: None,
            pending: None,
            draft: HashMap::new(),
            summary: ErrorSummary::new("records-errors"),
            error: String::new(),
            warning: String::new(),
            flash: None,
            clock: None,
            instance_notice: String::new(),
        })
    }
    fn can(&self, cap: &str) -> bool {
        self.flags.get(cap).copied().unwrap_or(false)
    }
    fn filter_id(&self, key: &str) -> Option<i64> {
        params::positive_id(self.list.filters.get(key).map(String::as_str))
    }
    fn clear_selection(&mut self) {
        self.selected_fact = None;
        self.document = None;
        self.pending = None;
        self.draft.clear();
        self.summary = ErrorSummary::new("records-errors");
        if let Some(uploads) = &mut self.uploads {
            uploads.clear("evidence");
        }
    }
    async fn load(&mut self, scope: &Scope) -> DbResult<()> {
        self.error.clear();
        self.warning.clear();
        self.flags.clear();
        self.sessions.clear();
        self.history.clear();
        self.evidence.clear();
        self.holds.clear();
        self.attendance_employees.clear();
        self.passport_employees.clear();
        self.passport = None;
        let client = self.training.db.get().await?;
        self.companies =
            authorization::selectable_companies(&**client, scope, self.task.capability())
                .await
                .unwrap_or_default()
                .into_iter()
                .filter(|c| !c.archived)
                .collect();
        let requested = self
            .list
            .filters
            .get("company_id")
            .filter(|s| !s.is_empty());
        self.company = if requested.is_some() {
            self.filter_id("company_id")
                .filter(|id| self.companies.iter().any(|c| c.id == *id))
        } else {
            self.companies.first().map(|c| c.id)
        };
        let Some(company) = self.company else {
            self.error = "No company is available. Ask an operator to check your access.".into();
            self.clear_selection();
            self.session = None;
            self.employee = None;
            return Ok(());
        };
        self.list
            .filters
            .insert("company_id".into(), company.to_string());
        for cap in [
            RECORDS_WORKSPACE_VIEW,
            RECORDS_VIEW,
            RECORDS_MANAGE,
            EVIDENCE_MANAGE,
            RETENTION_MANAGE,
            PASSPORT_MY_VIEW,
            PASSPORT_TEAM_VIEW,
            PASSPORT_GENERATE,
            LEARNING_VIEW,
        ] {
            self.flags
                .insert(cap, self.training.allowed(scope, company, cap).await);
        }
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
        self.instance_notice = bilibi_system::Instance::resolve(&self.training.settings, &**client)
            .await?
            .notice();
        if self.uploads.is_none() {
            let max = self
                .training
                .settings
                .get_global(&**client, "artifacts.max_bytes")
                .await?
                .as_u64()
                .unwrap_or(1);
            self.uploads = Some(Uploads::new([
                UploadSpec::new("evidence", max).accept([".pdf"])
            ]));
        }
        drop(client);
        if self.task == Task::Records {
            if self.can(RECORDS_VIEW) {
                match self.training.sessions(scope, company).await {
                    Ok(rows) => self.sessions = rows,
                    Err(e) => self.error = e.to_string(),
                }
                self.session = if self
                    .list
                    .filters
                    .get("session_id")
                    .is_some_and(|s| !s.is_empty())
                {
                    self.filter_id("session_id")
                        .filter(|id| self.sessions.iter().any(|s| s.id == *id))
                } else {
                    self.sessions.first().map(|s| s.id)
                };
                if let Some(session) = self.session {
                    self.list
                        .filters
                        .insert("session_id".into(), session.to_string());
                    match self.training.history(scope, company, session).await {
                        Ok(rows) => self.history = rows,
                        Err(e) => self.error = e.to_string(),
                    }
                    if self.can(RECORDS_MANAGE) {
                        match self.training.participation_employees(scope, company).await {
                            Ok(rows) => self.attendance_employees = rows,
                            Err(e) => self.error = e.to_string(),
                        }
                    }
                }
                if let Some(fact) = self
                    .selected_fact
                    .filter(|id| self.history.iter().any(|f| f.id == *id))
                {
                    match self.training.evidence(scope, company, fact).await {
                        Ok(rows) => self.evidence = rows,
                        Err(e) => self.error = e.to_string(),
                    }
                } else {
                    self.selected_fact = None;
                    if let Some(u) = &mut self.uploads {
                        u.clear("evidence");
                    }
                }
            } else {
                self.session = None;
                self.selected_fact = None;
            }
            if self.can(RETENTION_MANAGE) {
                match self.training.purge_holds(scope, company).await {
                    Ok(rows) => self.holds = rows,
                    Err(e) => self.error = e.to_string(),
                }
            }
            let page = self.list.paginate(&self.history);
            self.list.page = page.counts.page;
        } else {
            match self
                .training
                .passport_employees(scope, company, self.task.audience())
                .await
            {
                Ok(directory) => {
                    self.warning = directory.workforce_warning().unwrap_or_default();
                    self.passport_employees = directory.employees;
                }
                Err(e) => self.error = e.to_string(),
            }
            self.employee = if self
                .list
                .filters
                .get("employee_id")
                .is_some_and(|s| !s.is_empty())
            {
                self.filter_id("employee_id").filter(|id| {
                    self.passport_employees
                        .iter()
                        .any(|e| e.reference.stable_id == id.to_string())
                })
            } else {
                self.passport_employees
                    .first()
                    .and_then(|e| e.reference.stable_id.parse().ok())
            };
            if let Some(employee) = self.employee {
                self.list
                    .filters
                    .insert("employee_id".into(), employee.to_string());
                match self
                    .training
                    .passport(
                        scope,
                        company,
                        self.task.audience(),
                        employee,
                        &self.list.to_params(),
                    )
                    .await
                {
                    Ok(passport) => {
                        self.list.page = passport.page.page as u64;
                        self.passport = Some(passport);
                    }
                    Err(e) => self.error = e.to_string(),
                }
            } else if self.error.is_empty() && self.filter_id("employee_id").is_some() {
                self.error = "That employee is not available in this view.".into();
            }
            if self.passport.is_none() {
                self.document = None;
            }
        }
        Ok(())
    }
    fn fail(&mut self, message: impl Into<String>) {
        self.flash = Some((StatusKind::Error, message.into()));
    }
    fn finish<T>(&mut self, result: crate::Result<T>, message: &str) -> bool {
        match result {
            Ok(_) => {
                self.flash = Some((StatusKind::Success, message.into()));
                self.summary = ErrorSummary::new("records-errors");
                true
            }
            Err(e) => {
                self.summary = ErrorSummary::new("records-errors");
                if let TrainingError::Invalid(errors) = &e {
                    for error in errors {
                        self.summary
                            .push(format!("record-{}", error.field), error.message.clone());
                    }
                }
                self.fail(e.to_string());
                false
            }
        }
    }
    async fn event(&mut self, scope: &Scope, payload: &Value) -> DbResult<()> {
        let event = posted(payload, "type");
        if let Some(table) = event.strip_prefix("grid-") {
            if let Some(columns) = self.columns.get_mut(table) {
                columns.handle(&params::string_values(payload));
            }
            return Ok(());
        }
        match event.as_str() {
            "clear-flash" => {
                self.flash = None;
                return Ok(());
            }
            "page" => {
                self.list
                    .put_page(payload.get("page").and_then(Value::as_str));
                return Ok(());
            }
            "cancel_retention" => {
                self.pending = None;
                return Ok(());
            }
            "select_company" | "filter" | "filters" => {
                let values = params::form_values(payload, "filters");
                let changed_company = values.get("company_id").is_some_and(|id| {
                    Some(id.as_str()) != self.list.filters.get("company_id").map(String::as_str)
                });
                self.list.apply_filters(&values);
                if changed_company {
                    self.list.filters.insert("session_id".into(), String::new());
                    self.list
                        .filters
                        .insert("employee_id".into(), String::new());
                }
                self.clear_selection();
                return Ok(());
            }
            "select_session" => {
                if self.task == Task::Records {
                    let id = params::positive_id(payload.get("value").and_then(Value::as_str));
                    if let Some(id) = id.filter(|id| self.sessions.iter().any(|s| s.id == *id)) {
                        self.list
                            .filters
                            .insert("session_id".into(), id.to_string());
                        self.list.page = 1;
                        self.clear_selection();
                    }
                }
                return Ok(());
            }
            "select_fact" => {
                let id = params::positive_id(payload.get("value").and_then(Value::as_str));
                self.clear_selection();
                self.selected_fact = id.filter(|id| self.history.iter().any(|f| f.id == *id));
                return Ok(());
            }
            "validate_upload" => return Ok(()),
            _ => {}
        }
        let Some(company) = self.company else {
            self.fail(NO_WRITE);
            return Ok(());
        };
        let required = match event.as_str() {
            "record" if self.task == Task::Records => RECORDS_MANAGE,
            "upload" if self.task == Task::Records => EVIDENCE_MANAGE,
            "purge" | "retry_purge" | "confirm_retention" if self.task == Task::Records => {
                RETENTION_MANAGE
            }
            "generate" if self.task != Task::Records => PASSPORT_GENERATE,
            _ => {
                self.fail("That action is not available in this view.");
                return Ok(());
            }
        };
        if !self
            .training
            .allowed(scope, company, self.task.capability())
            .await
            || !self.training.allowed(scope, company, required).await
        {
            self.pending = None;
            self.fail(if event == "generate" {
                NO_GENERATE
            } else {
                NO_WRITE
            });
            return Ok(());
        }
        if matches!(event.as_str(), "record" | "upload")
            && !self.training.allowed(scope, company, RECORDS_VIEW).await
        {
            self.fail(NO_WRITE);
            return Ok(());
        }
        match event.as_str() {
            "record" => {
                self.draft = params::form_values(payload, "record");
                // Retain the full allowed reason, beyond form_values' generic 1 KiB bound.
                self.draft
                    .insert("reason".into(), posted(payload, "record[reason]"));
                let employee =
                    params::positive_id(self.draft.get("employee_id").map(String::as_str));
                let status = match self.draft.get("status").map(String::as_str) {
                    Some("confirmed") => Some(AttendanceStatus::Confirmed),
                    Some("absent") => Some(AttendanceStatus::Absent),
                    _ => None,
                };
                if let (Some(session_id), Some(employee_id), Some(status)) =
                    (self.session, employee, status)
                {
                    let result = self
                        .training
                        .record(
                            scope,
                            company,
                            &NewAttendance {
                                session_id,
                                employee_id,
                                status,
                                reason: self.value("reason"),
                                import_key: self.value("import_key"),
                            },
                        )
                        .await;
                    if self.finish(result, "Attendance recorded.") {
                        self.draft.clear();
                    }
                } else {
                    self.fail(TrainingError::InvalidRecord.to_string());
                }
            }
            "upload" => {
                if let Some(fact) = self
                    .selected_fact
                    .filter(|id| self.history.iter().any(|f| f.id == *id))
                {
                    let files = self
                        .uploads
                        .as_mut()
                        .map(|u| u.consume("evidence"))
                        .unwrap_or_default();
                    if let [file] = files.as_slice() {
                        let result = self
                            .training
                            .attach(scope, company, fact, &file.bytes)
                            .await;
                        self.finish(result, "Evidence added.");
                    } else {
                        self.fail("Choose an evidence PDF first.");
                    }
                } else {
                    self.fail("Select an attendance fact and obtain evidence management access.");
                }
            }
            "purge" => {
                self.pending = Some(Retention {
                    company,
                    artifact: None,
                })
            }
            "retry_purge" => {
                let id = posted(payload, "value");
                if self.holds.iter().any(|h| h.metadata.id == id) {
                    self.pending = Some(Retention {
                        company,
                        artifact: Some(id),
                    });
                } else {
                    self.fail("That document is not available in retention holds.");
                }
            }
            "confirm_retention" => {
                if let Some(pending) = self.pending.take().filter(|p| p.company == company) {
                    if let Some(id) = pending.artifact {
                        let result = self.training.retry_purge(scope, company, &id).await;
                        self.finish(result, "Document purged.");
                    } else {
                        match self.training.purge(scope, company).await {
                            Ok(result) if !result.failed.is_empty() => self.fail("Some documents could not be purged. Check retention holds and retry."),
                            result => { self.finish(result, "Retention run complete."); },
                        }
                    }
                }
            }
            "generate" => {
                if let Some(employee) = self.employee.filter(|_| {
                    self.passport
                        .as_ref()
                        .is_some_and(|p| p.freshness == Freshness::Current)
                }) {
                    match self
                        .training
                        .generate_passport(scope, company, self.task.audience(), employee)
                        .await
                    {
                        Ok(document) => {
                            self.document = Some(document.id);
                            self.flash = Some((StatusKind::Success, "Passport generated.".into()));
                        }
                        Err(e) => self.fail(e.to_string()),
                    }
                } else {
                    self.fail(NO_GENERATE);
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn value(&self, key: &str) -> String {
        self.draft.get(key).cloned().unwrap_or_default()
    }
    fn instant(&self, _id: &str, value: &str) -> DbResult<bilibi_ui::Cell> {
        let Some(display) = &self.clock else {
            return Ok(bilibi_ui::Cell::text(value));
        };
        let instant = value.parse::<Timestamp>().map_err(|_| Error::Invalid)?;
        let r = display.render(
            instant,
            bilibi_datetime::Format::DateTime,
            bilibi_datetime::Precision::Minute,
        );
        Ok(bilibi_ui::Cell::instant(bilibi_ui::Instant::new(
            &r.iso,
            &r.text,
            &r.company,
            &r.utc,
            r.mode.as_str(),
        )))
    }
    fn table(
        &self,
        key: &str,
        id: &str,
        caption: &str,
        rows: Vec<FlexRow>,
        empty: EmptyState,
    ) -> DbResult<Html> {
        let columns = &self.columns[key];
        let mut table = FlexTable::new(id, columns.flex_columns(), rows);
        table.event = match key {
            "sessions" => "grid-sessions",
            "history" => "grid-history",
            "evidence" => "grid-evidence",
            "holds" => "grid-holds",
            _ => "grid-passport",
        };
        table.caption = caption;
        table.zoom = columns.zoom;
        table.suggestions = columns.flex_suggestions();
        table.add_query = &columns.add_query;
        table.empty = Some(TableEmpty::State(empty));
        render(&table)
    }
    fn tabs(&self) -> DbResult<Html> {
        let mut links = vec![];
        for (task, label) in [
            (Task::My, "My passport"),
            (Task::Team, "Team passports"),
            (Task::Records, "Attendance & evidence"),
        ] {
            if self.can(task.capability()) {
                links.push((
                    params::address(
                        task.path(),
                        &HashMap::from([(
                            "company_id".into(),
                            self.company.map(|c| c.to_string()).unwrap_or_default(),
                        )]),
                    ),
                    label,
                    task == self.task,
                ));
            }
        }
        if self.task != Task::Records && self.can(LEARNING_VIEW) {
            links.push((
                "/people/training/my".into(),
                "Learning requests & evaluations",
                false,
            ));
        }
        render(&Tabs {
            id: "training-records-tabs",
            label: "Training records",
            tabs: links
                .iter()
                .map(|(path, label, current)| Tab::link(path, label, *current))
                .collect(),
            ..Tabs::default()
        })
    }
    fn toolbar(&self) -> DbResult<Html> {
        if self.task == Task::Records && !self.can(RECORDS_VIEW) {
            return Ok(Html::EMPTY);
        }
        let mut controls = vec![ToolbarControl::select(
            "records-company",
            "company_id",
            "Company",
            &self.company.map(|c| c.to_string()).unwrap_or_default(),
            self.companies
                .iter()
                .map(|c| Choice::new(&c.name, c.id.to_string()))
                .collect(),
        )];
        if self.task == Task::Records && self.can(RECORDS_VIEW) {
            controls.push(ToolbarControl::select(
                "records-session",
                "session_id",
                "Session",
                &self.session.map(|c| c.to_string()).unwrap_or_default(),
                self.sessions
                    .iter()
                    .map(|s| Choice::new(&s.name, s.id.to_string()))
                    .collect(),
            ));
        } else if self.task != Task::Records {
            controls.push(ToolbarControl::select(
                "passport-employee",
                "employee_id",
                "Employee",
                &self.employee.map(|c| c.to_string()).unwrap_or_default(),
                self.passport_employees
                    .iter()
                    .map(|e| Choice::new(&e.display_name, &e.reference.stable_id))
                    .collect(),
            ));
        }
        render(&FilterToolbar {
            id: if self.task == Task::Records {
                "records-filters"
            } else {
                "passport-filters"
            },
            event: "filter",
            controls,
            class: "",
            target: "",
        })
    }
    fn attendance(&mut self) -> DbResult<Html> {
        let session_rows = self
            .sessions
            .iter()
            .map(|s| {
                Ok(FlexRow::new(s.id.to_string())
                    .cell("name", bilibi_ui::Cell::text(&s.name))
                    .cell(
                        "starts",
                        self.instant(&format!("session-start-{}", s.id), &s.starts_at.to_string())?,
                    )
                    .cell(
                        "ends",
                        self.instant(&format!("session-end-{}", s.id), &s.ends_at.to_string())?,
                    )
                    .cell("capacity", bilibi_ui::Cell::text(s.capacity.to_string()))
                    .slot(
                        "action",
                        button(
                            "select_session",
                            if self.session == Some(s.id) {
                                "Selected session"
                            } else {
                                "View attendance"
                            },
                            &s.id.to_string(),
                        )?,
                    ))
            })
            .collect::<DbResult<Vec<_>>>()?;
        let sessions = self.table(
            "sessions",
            "records-sessions",
            "Sessions",
            session_rows,
            EmptyState::missing(
                "No sessions yet",
                "A training operator can add sessions before recording attendance.",
            ),
        )?;
        let page = self.list.paginate(&self.history);
        let rows = page
            .entries
            .iter()
            .map(|fact| {
                let mut actor = fact.actor_user_id.to_string();
                if let Some(id) = fact.impersonator_id {
                    actor.push_str(&format!(" via {id}"));
                }
                Ok(FlexRow::new(fact.id.to_string())
                    .cell(
                        "employee",
                        bilibi_ui::Cell::text(fact.employee_id.to_string()),
                    )
                    .cell("revision", bilibi_ui::Cell::text(fact.revision.to_string()))
                    .cell("status", bilibi_ui::Cell::text(fact.status.as_str()))
                    .cell("reason", bilibi_ui::Cell::text(&fact.reason))
                    .cell("actor", bilibi_ui::Cell::text(actor))
                    .cell(
                        "time",
                        self.instant(&format!("fact-time-{}", fact.id), &fact.inserted_at)?,
                    )
                    .slot(
                        "evidence",
                        button("select_fact", "View evidence", &fact.id.to_string())?,
                    ))
            })
            .collect::<DbResult<Vec<_>>>()?;
        let history = self.table(
            "history",
            "participation-history",
            "Attendance revisions",
            rows,
            EmptyState::missing(
                "No attendance yet",
                "Confirmed attendance and corrections appear here as history.",
            ),
        )?;
        let pagination = render(&Pagination::new(
            "records-pagination",
            page.counts,
            &[25, 50, 100, 300],
        ))?;
        let entry = if self.can(RECORDS_MANAGE) && self.session.is_some() {
            if self.attendance_employees.is_empty() {
                render(&EmptyState::missing(
                    "No current employees",
                    "Ask an operator to check the workforce before confirming attendance.",
                ))?
            } else {
                let mut fields = vec![
                    Field::in_form("record", "employee_id")
                        .id("record-employee_id")
                        .label("Employee")
                        .kind(InputKind::Select)
                        .value(self.value("employee_id"))
                        .options(
                            self.attendance_employees
                                .iter()
                                .map(|e| Choice::new(&e.name, e.id.to_string()))
                                .collect(),
                        )
                        .required(),
                    Field::in_form("record", "status")
                        .id("record-status")
                        .label("Attendance")
                        .kind(InputKind::Select)
                        .value(
                            self.draft
                                .get("status")
                                .map(String::as_str)
                                .unwrap_or("confirmed"),
                        )
                        .options(vec![
                            Choice::new("Confirmed", "confirmed"),
                            Choice::new("Absent", "absent"),
                        ])
                        .required(),
                    Field::in_form("record", "reason")
                        .id("record-reason")
                        .label("Confirmation or correction reason")
                        .value(self.value("reason"))
                        .required()
                        .attr("maxlength", "2000"),
                    Field::in_form("record", "import_key")
                        .id("record-import_key")
                        .label("Record key")
                        .value(self.value("import_key"))
                        .required()
                        .attr("maxlength", "160"),
                ];
                for field in &mut fields {
                    for item in self.summary.items.iter().filter(|e| e.target == field.id) {
                        field.errors.push(item.message.clone());
                    }
                }
                render(&FieldsView {
                    id: "participation-form",
                    event: "record",
                    fields,
                    actions: submit("Record attendance")?,
                })?
            }
        } else {
            Html::EMPTY
        };
        let evidence = if self.selected_fact.is_some() {
            let company = self.company.ok_or(Error::Invalid)?;
            let rows = self
                .evidence
                .iter()
                .map(|d| {
                    Ok(FlexRow::new(d.id.to_string()).slot(
                        "document",
                        link(
                            &format!("/people/training/evidence/{company}/{}", d.artifact_id),
                            "Download evidence PDF",
                        )?,
                    ))
                })
                .collect::<DbResult<Vec<_>>>()?;
            let table = self.table(
                "evidence",
                "training-evidence",
                "Evidence",
                rows,
                EmptyState::missing(
                    "No evidence yet",
                    "An authorized operator can attach a PDF to this attendance fact.",
                ),
            )?;
            let upload = if self.can(EVIDENCE_MANAGE) {
                if let Some(uploads) = &self.uploads {
                    render(&bilibi_ui::Form {
                        id: "evidence-form",
                        event: "upload",
                        body: render(
                            &uploads
                                .input("evidence", "training-evidence-upload")
                                .label("Evidence PDF"),
                        )?,
                        actions: submit("Add evidence")?,
                        ..bilibi_ui::Form::default()
                    })?
                } else {
                    Html::EMPTY
                }
            } else {
                Html::EMPTY
            };
            render(&EvidenceView {
                table,
                upload,
                read_only: !self.can(EVIDENCE_MANAGE),
            })?
        } else {
            Html::EMPTY
        };
        render(&AttendanceView {
            sessions,
            history,
            pagination,
            entry,
            evidence,
            summary: render(&self.summary)?,
            limited: self.sessions.len() == 500,
            read_only: !self.can(RECORDS_MANAGE),
        })
    }
    fn retention(&self) -> DbResult<Html> {
        if self.task != Task::Records || !self.can(RETENTION_MANAGE) {
            return Ok(Html::EMPTY);
        }
        let rows = self
            .holds
            .iter()
            .map(|h| {
                Ok(FlexRow::new(&h.metadata.id)
                    .cell("document", bilibi_ui::Cell::text(&h.metadata.id))
                    .cell(
                        "reason",
                        bilibi_ui::Cell::text(h.last_error.as_deref().unwrap_or("")),
                    )
                    .cell("attempts", bilibi_ui::Cell::text(h.attempts.to_string()))
                    .slot(
                        "action",
                        button("retry_purge", "Retry purge", &h.metadata.id)?,
                    ))
            })
            .collect::<DbResult<Vec<_>>>()?;
        let table = self.table(
            "holds",
            "retention-holds",
            "Retention holds",
            rows,
            EmptyState::missing(
                "No retention holds",
                "Expired training documents are maintained by the document service.",
            ),
        )?;
        render(&RetentionView {
            table,
            action: button("purge", "Purge expired training documents", "")?,
        })
    }
    fn passport(&self) -> DbResult<Html> {
        let Some(passport) = &self.passport else {
            return if self.error.is_empty() {
                render(&EmptyState::missing(
                    "No team employees",
                    "Only employees currently reporting directly to your linked employee appear here.",
                ))
            } else {
                Ok(Html::EMPTY)
            };
        };
        let company = self.company.ok_or(Error::Invalid)?;
        let rows = passport
            .page
            .entries
            .iter()
            .map(|r| {
                let evidence = if r.evidence.is_empty() {
                    Html::text("No evidence on this revision")
                } else {
                    r.evidence
                        .iter()
                        .map(|d| {
                            link(
                                &format!(
                                    "{}/evidence/{company}/{}",
                                    self.task.path(),
                                    d.artifact_id
                                ),
                                "Download evidence PDF",
                            )
                        })
                        .collect::<DbResult<Html>>()?
                };
                Ok(FlexRow::new(r.fact_id.to_string())
                    .cell("course", bilibi_ui::Cell::text(&r.course))
                    .cell("session", bilibi_ui::Cell::text(&r.session))
                    .cell(
                        "completed",
                        self.instant(&format!("passport-date-{}", r.fact_id), &r.ends_at)?,
                    )
                    .cell(
                        "attendance",
                        bilibi_ui::Cell::text(format!("{} (revision {})", r.status, r.revision)),
                    )
                    .slot("evidence", evidence))
            })
            .collect::<DbResult<Vec<_>>>()?;
        let table = self.table("passport", "passport-records", "Training passport", rows, EmptyState::missing("No training records yet", "An authorized training operator can confirm session attendance and attach evidence."))?;
        let counts = bilibi_ui::PageCounts {
            page: passport.page.page as u64,
            page_size: passport.page.page_size as u64,
            total_entries: passport.page.total_entries as u64,
            total_pages: passport.page.total_pages as u64,
        };
        let can_generate = passport.freshness == Freshness::Current && self.can(PASSPORT_GENERATE);
        render(&PassportView {
            employee: &passport.employee.display_name,
            table,
            pagination: render(&Pagination::new(
                "passport-pagination",
                counts,
                &[25, 50, 100, 300],
            ))?,
            generate: if can_generate {
                render(&Button {
                    event: "generate",
                    working: "Generating…",
                    body: Html::text("Generate passport PDF"),
                    ..Button::default()
                })?
            } else {
                Html::EMPTY
            },
            can_generate,
            document: if let Some(id) = &self.document {
                link(
                    &format!("{}/document/{company}/{id}", self.task.path()),
                    "Download generated passport PDF",
                )?
            } else {
                Html::EMPTY
            },
        })
    }
    fn render(&mut self) -> DbResult<String> {
        let body = if self.task == Task::Records {
            if self.can(RECORDS_VIEW) {
                self.attendance()?
            } else {
                render(&EmptyState::missing(
                    "Choose a training records task",
                    "Use your authorized My or Team passport above. Company-wide attendance and evidence need separate record-view access.",
                ))?
            }
        } else {
            self.passport()?
        };
        let confirmation = if let Some(pending) = &self.pending {
            render(&ConfirmDialog {
                id: "training-retention-confirm",
                consequence: if pending.artifact.is_some() {
                    "Permanently purge this training document?"
                } else {
                    "Permanently purge expired training documents?"
                },
                detail: &format!(
                    "{} Document bytes cannot be recovered. Attendance history and document provenance remain.",
                    self.instance_notice
                ),
                confirm: if pending.artifact.is_some() {
                    "Retry purge"
                } else {
                    "Purge expired training documents"
                },
                working: "Purging…",
                confirm_event: "confirm_retention",
                cancel_event: "cancel_retention",
            })?
        } else {
            Html::EMPTY
        };
        let flash = match &self.flash {
            Some((kind, message)) => FlashMap {
                info: if *kind == StatusKind::Info {
                    message
                } else {
                    ""
                },
                success: if *kind == StatusKind::Success {
                    message
                } else {
                    ""
                },
                warning: if *kind == StatusKind::Warning {
                    message
                } else {
                    ""
                },
                error: if *kind == StatusKind::Error {
                    message
                } else {
                    ""
                },
            },
            None => NO_FLASH,
        };
        RecordsView {
            flash,
            tabs: self.tabs()?,
            toolbar: self.toolbar()?,
            error: &self.error,
            warning: &self.warning,
            body,
            retention: self.retention()?,
            confirmation,
        }
        .render()
        .map_err(|_| Error::Internal)
    }
}
impl LiveState<Scope, Error> for Records {
    fn uploads(&mut self) -> Option<&mut Uploads> {
        self.uploads.as_mut()
    }
    fn update<'a>(&'a mut self, req: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            self.load(req.scope).await?;
            match req.input {
                LiveInput::Refresh => {}
                LiveInput::Params(query) => {
                    self.list = ListState::parse(query, spec()?).map_err(|_| Error::Internal)?;
                    self.clear_selection();
                }
                LiveInput::Event(payload) => self.event(req.scope, payload).await?,
            }
            self.load(req.scope).await?;
            Ok(LiveBody {
                html: self.render()?,
                query: self.list.to_params(),
                rows: self.history.len()
                    + self.passport.as_ref().map_or(0, |p| p.page.entries.len()),
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
        .to_owned()
}
fn render(template: &impl Template) -> DbResult<Html> {
    Html::render(template).map_err(|_| Error::Internal)
}
fn button(event: &str, label: &str, value: &str) -> DbResult<Html> {
    render(&Button {
        event,
        name: "value",
        value,
        body: Html::text(label),
        ..Button::default()
    })
}
fn submit(label: &str) -> DbResult<Html> {
    render(&Button {
        button_type: "submit",
        variant: ButtonVariant::Primary,
        body: Html::text(label),
        ..Button::default()
    })
}
fn link(href: &str, label: &str) -> DbResult<Html> {
    render(&ActionLink {
        href,
        body: Html::text(label),
        ..ActionLink::default()
    })
}
#[derive(Template)]
#[template(path = "records.html")]
struct RecordsView<'a> {
    flash: FlashMap<'a>,
    tabs: Html,
    toolbar: Html,
    error: &'a str,
    warning: &'a str,
    body: Html,
    retention: Html,
    confirmation: Html,
}
#[derive(Template)]
#[template(path = "learning_fields.html")]
struct FieldsView<'a> {
    id: &'a str,
    event: &'a str,
    fields: Vec<Field>,
    actions: Html,
}
#[derive(Template)]
#[template(path = "records_attendance.html")]
struct AttendanceView {
    sessions: Html,
    history: Html,
    pagination: Html,
    summary: Html,
    entry: Html,
    evidence: Html,
    limited: bool,
    read_only: bool,
}
#[derive(Template)]
#[template(path = "records_evidence.html")]
struct EvidenceView {
    table: Html,
    upload: Html,
    read_only: bool,
}
#[derive(Template)]
#[template(path = "records_retention.html")]
struct RetentionView {
    table: Html,
    action: Html,
}
#[derive(Template)]
#[template(path = "records_passport.html")]
struct PassportView<'a> {
    employee: &'a str,
    table: Html,
    pagination: Html,
    generate: Html,
    document: Html,
    can_generate: bool,
}
