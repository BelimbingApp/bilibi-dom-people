//! Bilimbi Payroll.Web.RunsLive: frozen inputs, exact results and final decisions.
mod view;
use super::*;
use crate::{NewContribution, RunOutput, APPROVE};
use bilibi_ui::{Choice, ErrorSummary};
use rust_decimal::Decimal;

pub(crate) const PAGES: &str = "people/payroll.run_pages";
const MANAGE_REFUSED: &str = "You cannot change this company's payroll runs.";
const APPROVE_REFUSED: &str = "You cannot approve this company's payroll runs.";
pub(crate) struct Runs(Arc<Core>);
impl Runs {
    pub(crate) fn new(db: Db, payroll: Arc<Payroll>) -> Self {
        Self(Arc::new(Core {
            db,
            display: bilibi_datetime::DisplayPolicy::new(payroll.settings.clone()),
            payroll,
        }))
    }
}
impl Page<Scope, Error> for Runs {
    fn title(&self, operation: &str) -> Option<&'static str> {
        (operation == "runs").then_some("Payroll runs")
    }
    fn render<'a>(&'a self, request: PageRequest<'a, Scope>) -> PageFuture<'a, Error> {
        Box::pin(async move {
            let mut state = RunState::new(self.0.clone(), request.query)?;
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
            Ok(Some(
                Box::new(RunState::new(self.0.clone(), request.query)?)
                    as Box<dyn LiveState<Scope, Error>>,
            ))
        })
    }
}
struct Confirmation {
    company: i64,
    run: i64,
    action: Action,
}
enum Action {
    Lock,
    Calculate,
    Decide { outcome: String, reason: String },
}
struct RunState {
    core: Arc<Core>,
    company_id: Option<i64>,
    run_id: Option<i64>,
    companies: Vec<CompanySummary>,
    setup: Option<Setup>,
    output: Option<RunOutput>,
    can_manage: bool,
    can_approve: bool,
    flash: Flash,
    drafts: HashMap<String, Draft>,
    tables: HashMap<&'static str, Table>,
    pending: Option<Confirmation>,
}
impl RunState {
    fn new(core: Arc<Core>, query: &HashMap<String, String>) -> Result<Self> {
        let mut tables = HashMap::new();
        for (key, columns) in [
            (
                "runs",
                vec![
                    ("run", "Run"),
                    ("period", "Period"),
                    ("currency", "Currency"),
                    ("locked", "Locked"),
                ],
            ),
            (
                "contributions",
                vec![
                    ("key", "Contribution key"),
                    ("employee", "Employee"),
                    ("item", "Pay item"),
                    ("date", "Date"),
                    ("direction", "Direction"),
                    ("units", "Units"),
                    ("evidence", "Source evidence reference"),
                ],
            ),
            (
                "results",
                vec![
                    ("employee", "Employee"),
                    ("item", "Pay item"),
                    ("direction", "Direction"),
                    ("units", "Units"),
                    ("rate", "Rate"),
                    ("amount", "Amount"),
                ],
            ),
            (
                "totals",
                vec![
                    ("employee", "Employee"),
                    ("earning", "Earnings"),
                    ("deduction", "Deductions"),
                    ("net", "Net"),
                    ("employer", "Employer contribution"),
                    ("actions", "Actions"),
                ],
            ),
            (
                "documents",
                vec![
                    ("kind", "Document"),
                    ("employee", "Employee"),
                    ("download", "Download"),
                ],
            ),
        ] {
            tables.insert(key, Table::new(&format!("payroll-run-{key}"), &columns)?);
        }
        Ok(Self {
            core,
            company_id: params::positive_id(query.get("company_id").map(String::as_str)),
            run_id: params::positive_id(query.get("run_id").map(String::as_str)),
            companies: vec![],
            setup: None,
            output: None,
            can_manage: false,
            can_approve: false,
            flash: Flash::default(),
            drafts: HashMap::new(),
            tables,
            pending: None,
        })
    }
    async fn load(&mut self, scope: &Scope) -> Result<()> {
        let c = self.core.db.get().await?;
        self.companies =
            match authorization::company_choices(&**c, scope, VIEW, self.company_id).await {
                Ok(companies) => companies,
                Err(bilibi_people_workforce::WorkforceError::Failed(e)) => return Err(e),
                Err(_) => vec![],
            };
        drop(c);
        if self.company_id.is_none() {
            self.company_id = self.companies.first().map(|c| c.id);
        }
        self.setup = None;
        self.output = None;
        self.can_manage = false;
        self.can_approve = false;
        if let Some(company) = self
            .company_id
            .filter(|id| self.companies.iter().any(|c| c.id == *id))
        {
            self.setup = self.core.payroll.setup(scope, company).await.ok();
            if self.setup.is_some() {
                self.can_manage = self.core.payroll.allowed(scope, company, MANAGE).await;
                self.can_approve = self.core.payroll.allowed(scope, company, APPROVE).await;
                if let Some(run) = self.run_id {
                    self.output = self.core.payroll.run_output(scope, company, run).await.ok();
                }
            }
        }
        if self.output.is_none() {
            self.pending = None;
        }
        Ok(())
    }
    async fn authorized(&mut self, scope: &Scope, approve: bool) -> Option<(i64, i64)> {
        if let Some(company) = self.company_id {
            if self
                .core
                .payroll
                .allowed(scope, company, if approve { APPROVE } else { MANAGE })
                .await
            {
                if let Some(run) = self.run_id {
                    return Some((company, run));
                }
                self.flash.error("Choose a frozen payroll run.");
                return None;
            }
        }
        self.pending = None;
        self.flash.error(if approve {
            APPROVE_REFUSED
        } else {
            MANAGE_REFUSED
        });
        None
    }
    fn outcome(&mut self, event: &str, result: crate::Result<()>) {
        match result {
            Ok(()) => {
                self.drafts.remove(event);
                self.flash.success("Payroll action completed.");
            }
            Err(PayrollError::Invalid(errors)) => {
                self.drafts.entry(event.into()).or_default().errors = errors;
                self.flash
                    .error("Check the contribution fields and exact decimal units.");
            }
            Err(PayrollError::Unauthorized) => self.flash.error(if event == "decide" {
                APPROVE_REFUSED
            } else {
                MANAGE_REFUSED
            }),
            Err(PayrollError::Failed(_) | PayrollError::ArtifactFailed(_)) => self
                .flash
                .error("Payroll action could not be completed. Try again."),
            Err(error) => self.flash.error(&error.to_string()),
        }
    }
    async fn event(&mut self, scope: &Scope, payload: &Value) -> Result<()> {
        let event = text(payload, "type");
        match event.as_str() {
            "clear-flash" => {
                self.flash.dismiss(payload);
                return Ok(());
            }
            "cancel_action" => {
                self.pending = None;
                return Ok(());
            }
            "select_company" => {
                self.load(scope).await?;
                let selected =
                    params::positive_id(payload.get("company_id").and_then(Value::as_str));
                if self.companies.iter().any(|c| Some(c.id) == selected) {
                    self.company_id = selected;
                    self.run_id = None;
                    self.pending = None;
                    self.drafts.clear();
                }
                return Ok(());
            }
            "select_run" => {
                self.load(scope).await?;
                let selected = params::positive_id(payload.get("run_id").and_then(Value::as_str));
                if self
                    .setup
                    .as_ref()
                    .is_some_and(|s| s.runs.iter().any(|r| Some(r.id) == selected))
                {
                    self.run_id = selected;
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
        let approve = match event.as_str() {
            "decide" => true,
            "confirm_action" => matches!(
                self.pending.as_ref().map(|p| &p.action),
                Some(Action::Decide { .. })
            ),
            "intake" | "intake_mapped" | "intake_attendance" | "prepare_lock" | "calculate"
            | "generate_report" | "generate_payslip" => false,
            _ => return Err(Error::Invalid),
        };
        let Some((company, run)) = self.authorized(scope, approve).await else {
            return Ok(());
        };
        match event.as_str() {
            "prepare_lock" | "calculate" | "decide" => {
                self.load(scope).await?;
                let action = match event.as_str() {
                    "prepare_lock"
                        if self
                            .output
                            .as_ref()
                            .is_some_and(|o| o.run.locked_at.is_none()) =>
                    {
                        Some(Action::Lock)
                    }
                    "calculate"
                        if self.output.as_ref().is_some_and(|o| {
                            o.run.locked_at.is_some() && o.calculation.is_none()
                        }) =>
                    {
                        Some(Action::Calculate)
                    }
                    "decide"
                        if self
                            .output
                            .as_ref()
                            .is_some_and(|o| o.calculation.is_some() && o.decision.is_none()) =>
                    {
                        let draft = Draft::read(payload, "decision");
                        let outcome = draft.get("outcome").to_owned();
                        let reason = draft.get("reason").trim().to_owned();
                        self.drafts.insert("decide".into(), draft);
                        if !matches!(outcome.as_str(), "approved" | "rejected")
                            || reason.is_empty()
                            || reason.chars().count() > 500
                        {
                            self.outcome("decide", Err(PayrollError::InvalidDecision));
                            None
                        } else {
                            Some(Action::Decide { outcome, reason })
                        }
                    }
                    _ => {
                        self.outcome(&event, Err(PayrollError::RunUnavailable));
                        None
                    }
                };
                self.pending = action.map(|action| Confirmation {
                    company,
                    run,
                    action,
                });
            }
            "confirm_action" => {
                let result = match self.pending.take() {
                    Some(selected) if selected.company == company && selected.run == run => {
                        match selected.action {
                            Action::Lock => self
                                .core
                                .payroll
                                .lock_run(scope, company, run)
                                .await
                                .map(|_| ()),
                            Action::Calculate => self
                                .core
                                .payroll
                                .calculate(scope, company, run)
                                .await
                                .map(|_| ()),
                            Action::Decide { outcome, reason } => self
                                .core
                                .payroll
                                .decide_run(scope, company, run, &outcome, &reason)
                                .await
                                .map(|_| ()),
                        }
                    }
                    _ => Err(PayrollError::RunUnavailable),
                };
                self.outcome(if approve { "decide" } else { &event }, result);
            }
            "intake" | "intake_mapped" | "intake_attendance" => {
                self.drafts
                    .insert(event.clone(), Draft::read(payload, "input"));
                let result = self.intake(scope, company, run, &event).await;
                self.outcome(&event, result);
            }
            "generate_report" | "generate_payslip" => {
                let employee = params::positive_id(payload.get("employee").and_then(Value::as_str));
                let result = if event == "generate_payslip" && employee.is_none() {
                    Err(PayrollError::InvalidDocument)
                } else {
                    self.core
                        .payroll
                        .generate_document(
                            scope,
                            company,
                            run,
                            if event == "generate_report" {
                                "report"
                            } else {
                                "payslip"
                            },
                            if event == "generate_report" {
                                None
                            } else {
                                employee
                            },
                        )
                        .await
                        .map(|_| ())
                };
                self.outcome(&event, result);
            }
            _ => {}
        }
        Ok(())
    }
    async fn intake(
        &self,
        scope: &Scope,
        company: i64,
        run: i64,
        event: &str,
    ) -> crate::Result<()> {
        let draft = &self.drafts[event];
        let units = Decimal::from_str_exact(draft.get("units")).map_err(|_| {
            PayrollError::Invalid(vec![crate::FieldError::new(
                "units",
                "must be an exact decimal",
            )])
        })?;
        let input = NewContribution {
            employee_id: draft.id("employee_id").ok_or_else(|| {
                PayrollError::Invalid(vec![crate::FieldError::new(
                    "employee_id",
                    "must be a positive employee ID",
                )])
            })?,
            item_id: draft.id("item_id").unwrap_or_default(),
            source_key: draft.get("source_key").into(),
            evidence: draft.get("evidence").into(),
            on_date: draft.date("on_date")?,
            units,
            direction: draft.get("direction").into(),
        };
        match event {
            "intake_mapped" => {
                self.core
                    .payroll
                    .intake_mapped(
                        scope,
                        company,
                        run,
                        draft.get("source_kind"),
                        draft.get("mapping_source"),
                        &input,
                    )
                    .await
            }
            "intake_attendance" => {
                self.core
                    .payroll
                    .intake_attendance_allowance(
                        scope,
                        company,
                        run,
                        draft.get("mapping_source"),
                        &input,
                    )
                    .await
            }
            _ => self.core.payroll.intake(scope, company, run, &input).await,
        }
        .map(|_| ())
    }
    fn field(&self, event: &str, namespace: &str, key: &str, label: &str) -> Field {
        let mut field = Field::in_form(namespace, key)
            .id(if matches!(event, "intake" | "decide") {
                format!("{namespace}_{key}")
            } else {
                format!("{event}_{key}")
            })
            .label(label)
            .wrapper_class("mb-0");
        if let Some(draft) = self.drafts.get(event) {
            field = field.value(draft.get(key));
            for error in &draft.errors {
                if error.field == key {
                    field = field.error(bilibi_ui::error_sentence(label, &error.message));
                }
            }
        }
        field
    }
    fn select(
        &self,
        event: &str,
        namespace: &str,
        key: &str,
        label: &str,
        options: Vec<Choice>,
    ) -> Field {
        self.field(event, namespace, key, label)
            .kind(InputKind::Select)
            .options(options)
            .required()
    }
    fn form(&self, id: &str, event: &str, fields: Vec<Field>, label: &str) -> Result<Html> {
        let mut body = Html::EMPTY;
        if let Some(draft) = self.drafts.get(event) {
            let mut summary = ErrorSummary::new(format!("{id}-errors"));
            for error in &draft.errors {
                let field_id = fields
                    .iter()
                    .find(|f| f.name.ends_with(&format!("[{}]", error.field)))
                    .map(|f| f.id.clone())
                    .unwrap_or_else(|| format!("{event}_{}", error.field));
                let label = fields
                    .iter()
                    .find(|f| f.id == field_id)
                    .map(|f| f.label.as_str())
                    .unwrap_or(error.field);
                summary.push(field_id, bilibi_ui::error_sentence(label, &error.message));
            }
            body.push(&render(&summary)?);
        }
        for field in fields {
            body.push(&render(&field)?);
        }
        render(&Form {
            id,
            event,
            body,
            actions: submit(label)?,
            class: "mt-3 grid gap-3 sm:grid-cols-3",
            change: false,
        })
    }
}
impl LiveState<Scope, Error> for RunState {
    fn update<'a>(&'a mut self, request: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            match request.input {
                LiveInput::Event(payload) => self.event(request.scope, payload).await?,
                LiveInput::Params(query) => {
                    self.company_id =
                        params::positive_id(query.get("company_id").map(String::as_str));
                    self.run_id = params::positive_id(query.get("run_id").map(String::as_str));
                    self.pending = None;
                    self.drafts.clear();
                }
                LiveInput::Refresh => {}
            }
            self.load(request.scope).await?;
            let mut query = HashMap::new();
            if let Some(id) = self.company_id {
                query.insert("company_id".into(), id.to_string());
            }
            if let Some(id) = self.run_id {
                query.insert("run_id".into(), id.to_string());
            }
            Ok(LiveBody {
                html: self.render()?,
                query,
                ..LiveBody::default()
            })
        })
    }
}
