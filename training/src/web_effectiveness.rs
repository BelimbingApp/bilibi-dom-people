//! Bilimbi Training.Web.EffectivenessLive: HOD answers, frozen HR summaries and policy.
use crate::{web_evaluation::*, *};
use askama::Template;
use bilibi_database::{Error, Result as DbResult};
use bilibi_menu::{MenuItem, Requirement};
use bilibi_module_registry::{
    contributions::{Access, Method, PageWrites, Pending, Route, PAGE_WRITES, ROUTES},
    Contribution, Provenance,
};
use bilibi_ui::page::Page;
use bilibi_ui::{page::*, params, *};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

pub(crate) const PAGES: &str = "people/training.effectiveness_pages";
const PROVENANCE: Provenance = Provenance {
    module: MODULE,
    source: file!(),
    line: line!(),
};
inventory::submit! {Contribution::new(ROUTES,"people/training.effectiveness",PROVENANCE,&Route{method:Method::Get,path:"/people/training/effectiveness",service:PAGES,operation:"effectiveness",access:Access::Capability(EFFECTIVENESS_VIEW),framed:true})}
inventory::submit! {Contribution::new(PAGE_WRITES,PAGES,PROVENANCE,&PageWrites{service:PAGES,contracts:&[],pending:Pending::Every{reason:"Evaluation policy, reviews, reminders and frozen periods call the Training facade; operation contracts are not yet reviewed."}})}
bilibi_menu::contribute!(
    "people/training",
    MenuItem::page(
        "people.development.effectiveness",
        "Effectiveness",
        "/people/training/effectiveness"
    )
    .icon("chart-bar")
    .under("people.development")
    .requires(Requirement::Capability(EFFECTIVENESS_VIEW))
    .order(90)
);
pub(crate) struct Pages(pub Arc<Training>);
impl Page<Scope, Error> for Pages {
    fn title(&self, _: &str) -> Option<&'static str> {
        Some("Effectiveness")
    }
    fn render<'a>(&'a self, req: PageRequest<'a, Scope>) -> PageFuture<'a, Error> {
        Box::pin(async move {
            let mut state = Effectiveness::new(self.0.clone(), req.query)?;
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
            Ok(Some(
                Box::new(Effectiveness::new(self.0.clone(), req.query)?)
                    as Box<dyn LiveState<Scope, Error>>,
            ))
        })
    }
}
enum PendingAction {
    Policy(NewEvaluationPolicy),
    Prepare(i64),
    Reminders,
    Freeze,
}
struct Effectiveness {
    training: Arc<Training>,
    list: ListState,
    company: Option<i64>,
    companies: Vec<bilibi_people_workforce::CompanySummary>,
    reviews: Reviews,
    policy_access: bool,
    summary_access: bool,
    reminder_access: bool,
    policies: Vec<EvaluationPolicy>,
    summaries: Vec<EffectivenessSummary>,
    settings: EvaluationSettings,
    settings_draft: HashMap<String, String>,
    period_start: Option<String>,
    policy_open: bool,
    criterion_counts: [usize; 2],
    policy_draft: HashMap<String, String>,
    errors: ErrorSummary,
    pending: Option<PendingAction>,
    columns: BTreeMap<String, ListColumns>,
    error: Option<String>,
    flash: Option<(StatusKind, String)>,
}
impl Effectiveness {
    fn new(training: Arc<Training>, query: &HashMap<String, String>) -> DbResult<Self> {
        Ok(Self {
            training,
            list: ListState::parse(query, spec(&["company_id"], "")?)
                .map_err(|_| Error::Invalid)?,
            company: None,
            companies: vec![],
            reviews: Reviews::new(EvaluationKind::Effectiveness, query)?,
            policy_access: false,
            summary_access: false,
            reminder_access: false,
            policies: vec![],
            summaries: vec![],
            settings: EvaluationSettings {
                evaluation_days: None,
                checkpoints: None,
                reminder_days: None,
                minimum_cohort: None,
                report_months: None,
                report_grace_days: None,
            },
            settings_draft: HashMap::new(),
            period_start: None,
            policy_open: false,
            criterion_counts: [1, 1],
            policy_draft: HashMap::new(),
            errors: ErrorSummary::new("evaluation-policy-errors"),
            pending: None,
            columns: BTreeMap::from([
                (
                    "evaluation-policies".into(),
                    columns(
                        "evaluation-policies",
                        &[
                            ("version", "Version"),
                            ("period", "Effective period"),
                            ("criteria", "Criteria"),
                            ("reason", "Reason"),
                        ],
                    )?,
                ),
                (
                    "effectiveness-summary".into(),
                    columns(
                        "effectiveness-summary",
                        &[
                            ("period", "Reporting period"),
                            ("review", "Review"),
                            ("criterion", "Criterion"),
                            ("answered", "Known answers"),
                            ("mean", "Mean"),
                        ],
                    )?,
                ),
            ]),
            error: None,
            flash: None,
        })
    }
    fn reset(&mut self) {
        self.policy_open = false;
        self.pending = None;
        self.policy_draft.clear();
        self.settings_draft.clear();
        self.reviews.reset();
    }
    fn fail(&mut self, message: impl Into<String>) {
        self.flash = Some((StatusKind::Error, message.into()));
    }
    fn finish<T>(&mut self, result: crate::Result<T>, message: &str) -> bool {
        match result {
            Ok(_) => {
                self.flash = Some((StatusKind::Success, message.into()));
                true
            }
            Err(e) => {
                self.fail(e.to_string());
                false
            }
        }
    }
    async fn load(&mut self, scope: &Scope) -> DbResult<()> {
        let client = self.training.db.get().await?;
        self.companies = authorization::selectable_companies(&**client, scope, EFFECTIVENESS_VIEW)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|c| !c.archived)
            .collect();
        let selected = self
            .list
            .filters
            .get("company_id")
            .filter(|s| !s.is_empty());
        let company = if let Some(id) = selected {
            params::positive_id(Some(id)).filter(|id| self.companies.iter().any(|c| c.id == *id))
        } else {
            self.companies.first().map(|c| c.id)
        };
        if self.company != company {
            self.reset();
        }
        self.company = company;
        self.error = None;
        self.policies.clear();
        self.summaries.clear();
        self.period_start = None;
        self.policy_access = false;
        self.summary_access = false;
        self.reminder_access = false;
        if let Some(company) = company {
            self.list
                .filters
                .insert("company_id".into(), company.to_string());
            self.policy_access = self
                .training
                .allowed(scope, company, EVALUATION_POLICY_MANAGE)
                .await;
            self.summary_access = self
                .training
                .allowed(scope, company, EFFECTIVENESS_SUMMARY_VIEW)
                .await;
            self.reminder_access = self
                .training
                .allowed(scope, company, EVALUATION_REMINDERS_MANAGE)
                .await;
            if self.policy_access {
                match self.training.evaluation_settings(scope, company).await {
                    Ok(s) => self.settings = s,
                    Err(e) => self.error = Some(e.to_string()),
                }
                match self.training.evaluation_policies(scope, company).await {
                    Ok(p) => self.policies = p,
                    Err(e) => self.error = Some(e.to_string()),
                }
                self.period_start = self
                    .training
                    .effectiveness_period_start(scope, company)
                    .await
                    .ok()
                    .flatten();
            } else {
                self.policy_open = false;
                if matches!(self.pending, Some(PendingAction::Policy(_))) {
                    self.pending = None;
                }
            }
            if self.summary_access {
                match self.training.effectiveness_summary(scope, company).await {
                    Ok(s) => self.summaries = s,
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
            if !self.reminder_access && !matches!(self.pending, Some(PendingAction::Policy(_))) {
                self.pending = None;
            }
        } else {
            self.error = Some(
                "No company is available. Ask an operator to check your company access.".into(),
            );
        }
        self.reviews.load(&self.training, scope, company).await;
        Ok(())
    }
    async fn today(&self, scope: &Scope, company: i64) -> crate::Result<Date> {
        let client = self.training.db.get().await.map_err(Error::from)?;
        let zone = self.training.zone(&**client, scope, company).await?;
        Ok(Timestamp::now()
            .to_zoned(
                jiff::tz::TimeZone::get(&zone).map_err(|_| TrainingError::InvalidTimeZoneOrTime)?,
            )
            .date())
    }
    async fn event(&mut self, scope: &Scope, payload: &Value) -> DbResult<()> {
        let event = payload.get("type").and_then(Value::as_str).unwrap_or("");
        if event == "clear-flash" {
            if self
                .flash
                .as_ref()
                .is_some_and(|(k, _)| Some(k.name()) == payload.get("key").and_then(Value::as_str))
            {
                self.flash = None;
            }
            return Ok(());
        }
        if self.reviews.handles(event) {
            if let Some(message) = self.reviews.event(&self.training, scope, payload).await? {
                self.flash = Some(message);
            }
            return Ok(());
        }
        if let Some(key) = event.strip_prefix("table-") {
            if let Some(columns) = self.columns.get_mut(key) {
                columns.handle(&params::string_values(payload));
            }
            return Ok(());
        }
        if matches!(event, "select_company" | "filters") {
            self.list
                .apply_filters(&params::form_values(payload, "filters"));
            self.reviews.list.page = 1;
            self.reset();
            return Ok(());
        }
        if event == "close_policy" {
            self.policy_open = false;
            return Ok(());
        }
        if event == "cancel_confirm" {
            self.pending = None;
            return Ok(());
        }
        let cap = match event {
            "open_policy" | "add_criterion" | "save_policy" | "confirm_policy"
            | "save_settings" => EVALUATION_POLICY_MANAGE,
            "prepare_reviews" | "run_reminders" | "freeze_summaries" | "confirm_maintenance" => {
                EVALUATION_REMINDERS_MANAGE
            }
            _ => return Ok(()),
        };
        let refusal = if cap == EVALUATION_POLICY_MANAGE {
            "You cannot publish this company's policy."
        } else {
            "You cannot prepare this company's reminders."
        };
        let Some(company) = self.company else {
            self.fail(refusal);
            return Ok(());
        };
        if !self
            .training
            .allowed(scope, company, EFFECTIVENESS_VIEW)
            .await
            || !self.training.allowed(scope, company, cap).await
        {
            self.reset();
            self.fail(refusal);
            return Ok(());
        }
        match event {
            "open_policy" => {
                self.policy_open = true;
                self.criterion_counts = [1, 1];
                self.policy_draft.clear();
                self.errors = ErrorSummary::new("evaluation-policy-errors");
            }
            "add_criterion" => {
                if self.policy_open {
                    match posted(payload, "value").as_str() {
                        "criteria" => {
                            self.criterion_counts[0] = (self.criterion_counts[0] + 1).min(50)
                        }
                        "effectiveness_criteria" => {
                            self.criterion_counts[1] = (self.criterion_counts[1] + 1).min(50)
                        }
                        _ => {}
                    }
                }
            }
            "save_settings" => {
                self.settings_draft = draft(payload, "settings[");
                let number = |key| {
                    posted(payload, &format!("settings[{key}]"))
                        .parse::<i32>()
                        .ok()
                };
                let checkpoints = posted(payload, "settings[checkpoints]")
                    .split(',')
                    .map(|s| s.trim().parse::<i32>())
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .ok();
                let settings = EvaluationSettings {
                    evaluation_days: number("evaluation_days"),
                    checkpoints,
                    reminder_days: number("reminder_days"),
                    minimum_cohort: number("minimum_cohort"),
                    report_months: number("report_months"),
                    report_grace_days: number("report_grace_days"),
                };
                let result = self
                    .training
                    .put_evaluation_settings(scope, company, &settings)
                    .await;
                if self.finish(result, "Evaluation settings saved.") {
                    self.settings_draft.clear();
                    self.errors = ErrorSummary::new("evaluation-policy-errors");
                } else {
                    self.errors = ErrorSummary::new("evaluation-policy-errors");
                    if let Some((_, message)) = &self.flash {
                        self.errors
                            .push("settings-evaluation_days", message.clone());
                    }
                }
            }
            "save_policy" => {
                if !self.policy_open {
                    return Ok(());
                }
                self.policy_draft = draft(payload, "entry[");
                match self.policy_input() {
                    Ok(policy) => {
                        self.pending = Some(PendingAction::Policy(policy));
                        self.policy_open = false;
                    }
                    Err(e) => {
                        self.fail(e.to_string());
                        self.errors = ErrorSummary::new("evaluation-policy-errors");
                        self.errors.push("criteria-1-code", e.to_string());
                    }
                }
            }
            "confirm_policy" => {
                if let Some(PendingAction::Policy(policy)) = self.pending.take() {
                    let result = self
                        .training
                        .publish_evaluation_policy(scope, company, &policy)
                        .await;
                    if !self.finish(result, "Policy version published.") {
                        self.policy_open = true;
                        self.errors = ErrorSummary::new("evaluation-policy-errors");
                        if let Some((_, message)) = &self.flash {
                            self.errors.push("policy-effective_from", message.clone());
                        }
                    }
                }
            }
            "prepare_reviews" => {
                if let Some(id) = params::positive_id(Some(&posted(payload, "entry[session_id]"))) {
                    self.pending = Some(PendingAction::Prepare(id));
                } else {
                    self.fail("Choose a session in this company.");
                }
            }
            "run_reminders" => self.pending = Some(PendingAction::Reminders),
            "freeze_summaries" => self.pending = Some(PendingAction::Freeze),
            "confirm_maintenance" => {
                let Some(pending) = self.pending.take() else {
                    return Ok(());
                };
                match pending {
                    PendingAction::Prepare(session) => {
                        let result = self
                            .training
                            .prepare_evaluation_reviews(scope, company, session, Timestamp::now())
                            .await;
                        let message=match &result{Ok(r)if r.unknown>0=>format!("Session reviews prepared. {} attendee(s) are no longer current employees and were skipped.",r.unknown),_=>"Session reviews prepared.".into()};
                        self.finish(result, &message);
                    }
                    PendingAction::Reminders => {
                        let result = match self.today(scope, company).await {
                            Ok(today) => {
                                self.training
                                    .evaluation_reminders(scope, company, today)
                                    .await
                            }
                            Err(e) => Err(e),
                        };
                        self.finish(result, "Due reminder worklist refreshed.");
                    }
                    PendingAction::Freeze => {
                        let result = match self.today(scope, company).await {
                            Ok(today) => {
                                self.training
                                    .freeze_effectiveness_summaries(scope, company, today)
                                    .await
                            }
                            Err(e) => Err(e),
                        };
                        self.finish(result, "Closed reporting periods frozen.");
                    }
                    PendingAction::Policy(policy) => {
                        self.pending = Some(PendingAction::Policy(policy))
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn policy_input(&self) -> crate::Result<NewEvaluationPolicy> {
        let get = |key: &str| {
            self.policy_draft
                .get(&format!("entry[{key}]"))
                .cloned()
                .unwrap_or_default()
        };
        let criteria = |kind: &str, count: usize| -> crate::Result<Vec<EvaluationCriterion>> {
            (1..=count)
                .map(|i| {
                    Ok(EvaluationCriterion {
                        code: get(&format!("{kind}][{i}][code")),
                        label: get(&format!("{kind}][{i}][label")),
                        minimum: get(&format!("{kind}][{i}][minimum"))
                            .parse()
                            .map_err(|_| TrainingError::InvalidCriteria)?,
                        maximum: get(&format!("{kind}][{i}][maximum"))
                            .parse()
                            .map_err(|_| TrainingError::InvalidCriteria)?,
                    })
                })
                .collect()
        };
        Ok(NewEvaluationPolicy {
            effective_from: get("effective_from"),
            effective_to: get("effective_to"),
            criteria: criteria("criteria", self.criterion_counts[0])?,
            effectiveness_criteria: criteria("effectiveness_criteria", self.criterion_counts[1])?,
            reason: get("reason"),
        })
    }
    fn settings_form(&self) -> DbResult<Html> {
        let settings = &self.settings;
        let value = |key: &str, stored: Option<i32>, proposed: i32| {
            self.settings_draft
                .get(&format!("settings[{key}]"))
                .cloned()
                .unwrap_or_else(|| stored.unwrap_or(proposed).to_string())
        };
        let number =
            |key: &str, label: &str, stored: Option<i32>, proposed: i32, min: &str, max: &str| {
                Field::in_form("settings", key)
                    .id(format!("settings-{key}"))
                    .kind(InputKind::Number)
                    .label(label)
                    .required()
                    .attr("min", min)
                    .attr("max", max)
                    .value(value(key, stored, proposed))
            };
        let checkpoints = self
            .settings_draft
            .get("settings[checkpoints]")
            .cloned()
            .unwrap_or_else(|| {
                settings
                    .checkpoints
                    .clone()
                    .unwrap_or_else(|| vec![30, 90])
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            });
        let fields = vec![
            number(
                "evaluation_days",
                "Evaluation due days",
                settings.evaluation_days,
                7,
                "0",
                "3650",
            ),
            Field::in_form("settings", "checkpoints")
                .id("settings-checkpoints")
                .label("Effectiveness checkpoints (days after a session, comma-separated)")
                .required()
                .value(checkpoints),
            number(
                "reminder_days",
                "Reminder lead days",
                settings.reminder_days,
                3,
                "0",
                "3650",
            ),
            number(
                "minimum_cohort",
                "Disclosure minimum",
                settings.minimum_cohort,
                5,
                "2",
                "1000",
            ),
            Field::in_form("settings", "report_months")
                .id("settings-report_months")
                .kind(InputKind::Select)
                .label("Reporting period months")
                .required()
                .value(value("report_months", settings.report_months, 3))
                .options(
                    [1, 2, 3, 4, 6, 12]
                        .into_iter()
                        .map(|n| Choice::new(n.to_string(), n.to_string()))
                        .collect(),
                ),
            number(
                "report_grace_days",
                "Answer grace days",
                settings.report_grace_days,
                14,
                "0",
                "3650",
            ),
        ];
        render(&SettingsBody {
            summary: &self.errors,
            form: form(
                "evaluation-settings-form",
                "save_settings",
                fields,
                submit("Save settings")?,
            )?,
            period: self.period_start.as_deref().unwrap_or(""),
        })
    }
    fn policy_dialog(&self) -> DbResult<Html> {
        if !self.policy_open {
            return Ok(Html::EMPTY);
        }
        let field = |name: &str, id: &str, label: &str, kind: InputKind| {
            Field::named(name)
                .id(id)
                .label(label)
                .kind(kind)
                .required()
                .value(self.policy_draft.get(name).cloned().unwrap_or_default())
        };
        let dates = vec![
            field(
                "entry[effective_from]",
                "policy-effective_from",
                "Effective from",
                InputKind::Date,
            ),
            field(
                "entry[effective_to]",
                "policy-effective_to",
                "Effective to",
                InputKind::Date,
            ),
        ];
        let mut groups = vec![];
        for (index, kind, label) in [
            (0, "criteria", "Employee evaluation"),
            (1, "effectiveness_criteria", "HOD effectiveness"),
        ] {
            let mut fields = vec![];
            for i in 1..=self.criterion_counts[index] {
                for (key, label, input) in [
                    ("code", "Criterion code", InputKind::Text),
                    ("label", "Criterion label", InputKind::Text),
                    ("minimum", "Minimum score", InputKind::Number),
                    ("maximum", "Maximum score", InputKind::Number),
                ] {
                    let f = field(
                        &format!("entry[{kind}][{i}][{key}]"),
                        &format!("{kind}-{i}-{key}"),
                        label,
                        input,
                    );
                    fields.push(if matches!(key, "minimum" | "maximum") {
                        f.attr("min", "-10000").attr("max", "10000")
                    } else {
                        f.attr("maxlength", if key == "code" { "80" } else { "160" })
                    });
                }
            }
            groups.push((
                label,
                fields,
                if self.criterion_counts[index] < 50 {
                    button("add_criterion", "Add criterion", kind)?
                } else {
                    Html::EMPTY
                },
            ));
        }
        let reason = field(
            "entry[reason]",
            "policy-reason",
            "Publication reason",
            InputKind::Textarea,
        )
        .attr("maxlength", "4000");
        let body = render(&PolicyBody {
            dates,
            groups,
            reason,
            summary: &self.errors,
            actions: submit("Review publication")?,
        })?;
        render(&Modal {
            id: "evaluation-policy-modal",
            title: "Publish criteria",
            cancel_event: "close_policy",
            body,
            flash: Some(flash(&self.flash)),
            ..Modal::default()
        })
    }
    fn summary_table(&self) -> DbResult<Html> {
        let mut rows = vec![];
        for period in &self.summaries {
            let label = format!("{} – {}", period.period_start, period.period_end);
            if period.status == "suppressed" {
                rows.push(FlexRow::new(format!("summary-{}",period.period_start)).cell("period",Cell::text(&label)).cell("mean",Cell::text("Summary suppressed: the cohort is below this company's disclosure minimum.")));
                continue;
            }
            for (g, group) in period.groups.items.iter().enumerate() {
                let review = format!(
                    "{}-day review · criteria version {}",
                    group.checkpoint_days, group.policy_version
                );
                if group.status == "suppressed" {
                    rows.push(
                        FlexRow::new(format!("{}-{g}", period.id))
                            .cell("period", Cell::text(&label))
                            .cell("review", Cell::text(&review))
                            .cell("mean", Cell::text("Summary suppressed for this cohort.")),
                    );
                } else {
                    for (c, criterion) in group.criteria.iter().enumerate() {
                        rows.push(
                            FlexRow::new(format!("{}-{g}-{c}", period.id))
                                .cell("period", Cell::text(&label))
                                .cell("review", Cell::text(&review))
                                .cell("criterion", Cell::text(&criterion.label))
                                .cell(
                                    "answered",
                                    Cell::text(
                                        criterion
                                            .answered
                                            .map(|n| n.to_string())
                                            .unwrap_or_else(|| "Suppressed".into()),
                                    ),
                                )
                                .cell(
                                    "mean",
                                    Cell::text(
                                        criterion
                                            .mean
                                            .as_deref()
                                            .unwrap_or("Suppressed — insufficient known data"),
                                    ),
                                ),
                        );
                    }
                }
            }
        }
        table("effectiveness-summary",&self.columns["effectiveness-summary"],rows,"table-effectiveness-summary","No frozen reporting period","A period's summary appears once it ends, its answer grace window passes and an operator freezes it.")
    }
    fn render(&self) -> DbResult<String> {
        let mut body = self.reviews.render(flash(&self.flash))?;
        if self.summary_access {
            body.push(&render(&Section{id:"effectiveness-summary-section",title:"Summary",description:"Fixed company reporting periods, frozen after the answer grace window; small cohorts and insufficient known answers are suppressed.",body:self.summary_table()?})?);
        }
        if self.policy_access {
            let rows = self
                .policies
                .iter()
                .map(|p| {
                    FlexRow::new(p.id.to_string())
                        .cell("version", Cell::text(p.version.to_string()))
                        .cell(
                            "period",
                            Cell::text(format!("{} – {}", p.effective_from, p.effective_to)),
                        )
                        .cell(
                            "criteria",
                            Cell::text(
                                p.criteria
                                    .items
                                    .iter()
                                    .chain(p.effectiveness_criteria.items.iter())
                                    .map(|c| c.label.as_str())
                                    .collect::<Vec<_>>()
                                    .join(" · "),
                            ),
                        )
                        .cell("reason", Cell::text(&p.reason))
                })
                .collect();
            let mut content = render(&Card {
                body: self.settings_form()?,
                ..Card::default()
            })?;
            content.push(&table(
                "evaluation-policies",
                &self.columns["evaluation-policies"],
                rows,
                "table-evaluation-policies",
                "No evaluation policy",
                "Configure company settings, then publish effective-dated criteria.",
            )?);
            body.push(&render(&Section{id:"evaluation-policy-section",title:"Published criteria",description:"Versions and answers are permanent. Publish another effective period to change criteria.",body:content})?);
        }
        if self.reminder_access {
            let mut content = form(
                "prepare-review-form",
                "prepare_reviews",
                vec![Field::in_form("entry", "session_id")
                    .id("prepare-session")
                    .kind(InputKind::Number)
                    .label("Completed session")
                    .required()
                    .attr("min", "1")],
                submit("Prepare reviews")?,
            )?;
            content.push(&button("run_reminders", "Refresh reminders", "")?);
            content.push(&button("freeze_summaries", "Freeze closed periods", "")?);
            body.push(&render(&Section{id:"effectiveness-maintenance",title:"Due reminders",description:"Prepare session reviews, refresh the durable worklist and freeze closed reporting periods. This does not send email.",body:render(&Card { body: content, ..Card::default() })?})?);
        }
        if !self.reviews.enabled
            && !self.summary_access
            && !self.policy_access
            && !self.reminder_access
            && self.error.is_none()
        {
            body.push(&render(&EmptyState::missing("No review tasks available","Ask an operator to grant HOD review or HR summary access. Employees answer their own evaluations under My learning."))?);
        }
        body.push(&self.policy_dialog()?);
        if let Some(pending) = &self.pending {
            let (id,consequence,detail,confirm,working,event)=match pending{
                PendingAction::Policy(_)=>("evaluation-policy-confirm","This criteria version will be published permanently.","It applies to sessions ending in its effective period. Settings are captured at publication.","Publish","Publishing…","confirm_policy"),
                PendingAction::Prepare(_)=>("evaluation-prepare-confirm","Prepare reviews for this completed session?","Reviews preserve the published criteria version and checkpoints.","Prepare reviews","Preparing…","confirm_maintenance"),
                PendingAction::Reminders=>("evaluation-reminders-confirm","Refresh the due reminder worklist?","Available reminders are recorded permanently. This does not send email.","Refresh reminders","Refreshing…","confirm_maintenance"),
                PendingAction::Freeze=>("effectiveness-freeze-confirm","Freeze closed reporting periods permanently?","Summaries preserve the current disclosure minimum. Later answers will not change a frozen summary.","Freeze closed periods","Freezing…","confirm_maintenance"),
            };
            let detail = if let PendingAction::Prepare(session) = pending {
                format!("Completed session {session}. {detail}")
            } else {
                detail.into()
            };
            body.push(&render(&ConfirmDialog {
                id,
                consequence,
                detail: &detail,
                confirm,
                working,
                confirm_event: event,
                cancel_event: "cancel_confirm",
            })?);
        }
        let toolbar = render(&FilterToolbar {
            id: "effectiveness-filters",
            event: "select_company",
            controls: vec![ToolbarControl::select(
                "effectiveness-company",
                "company_id",
                "Company",
                &self.company.map(|n| n.to_string()).unwrap_or_default(),
                self.companies
                    .iter()
                    .map(|c| Choice::new(&c.name, c.id.to_string()))
                    .collect(),
            )],
            class: "",
            target: "",
        })?;
        EffectivenessView {
            flash: flash(&self.flash),
            actions: if self.policy_access {
                button("open_policy", "Publish criteria", "")?
            } else {
                Html::EMPTY
            },
            toolbar,
            error: self.error.as_deref().unwrap_or(""),
            body,
        }
        .render()
        .map_err(|_| Error::Internal)
    }
}
impl LiveState<Scope, Error> for Effectiveness {
    fn update<'a>(&'a mut self, req: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            self.load(req.scope).await?;
            match req.input {
                LiveInput::Refresh => {}
                LiveInput::Params(query) => {
                    self.list = ListState::parse(query, spec(&["company_id"], "")?)
                        .map_err(|_| Error::Invalid)?;
                    self.reviews = Reviews::new(EvaluationKind::Effectiveness, query)?;
                    self.reset();
                }
                LiveInput::Event(payload) => self.event(req.scope, payload).await?,
            }
            self.load(req.scope).await?;
            let mut query = self.list.to_params();
            query.extend(self.reviews.list.to_params());
            if let Some(company) = self.company {
                query.insert("company_id".into(), company.to_string());
            }
            Ok(LiveBody {
                html: self.render()?,
                query,
                ..LiveBody::default()
            })
        })
    }
}
#[derive(Template)]
#[template(path = "effectiveness.html")]
struct EffectivenessView<'a> {
    flash: FlashMap<'a>,
    actions: Html,
    toolbar: Html,
    error: &'a str,
    body: Html,
}
#[derive(Template)]
#[template(path = "evaluation_settings.html")]
struct SettingsBody<'a> {
    summary: &'a ErrorSummary,
    form: Html,
    period: &'a str,
}
#[derive(Template)]
#[template(path = "evaluation_policy.html")]
struct PolicyBody<'a> {
    dates: Vec<Field>,
    groups: Vec<(&'a str, Vec<Field>, Html)>,
    reason: Field,
    summary: &'a ErrorSummary,
    actions: Html,
}
