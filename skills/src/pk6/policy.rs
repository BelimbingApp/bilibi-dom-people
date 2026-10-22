//! Bilimbi Skills.Web.PolicyLive: settings, action types and reminder runs.
//! The brief also exposes the pending reassessments and delivery ledger here.
use super::*;
use crate::{
    AssessmentInput, DueReminder, NewActionType, PolicyChanges, ReassessmentView, Reminder,
    ASSESSMENTS_SUBMIT, CATALOG_MANAGE, POLICY_MANAGE, REASSESSMENTS_EXECUTE, REASSESSMENTS_SUBMIT,
    REMINDERS_SEND,
};
use askama::Template;
use bilibi_ui::page::{LiveBody, LiveFuture, LiveInput, LiveRequest};

struct Pending {
    company: i64,
    command: Command,
    detail: String,
}
enum Command {
    Toggle(i64, bool),
    Cancel(i64),
    Perform(i64, AssessmentInput),
    Run,
    Retry,
}
impl Command {
    fn cap(&self) -> &'static str {
        match self {
            Self::Toggle(..) => CATALOG_MANAGE,
            Self::Cancel(_) => REASSESSMENTS_SUBMIT,
            Self::Perform(..) => REASSESSMENTS_EXECUTE,
            _ => REMINDERS_SEND,
        }
    }
    fn verb(&self) -> &'static str {
        match self {
            Self::Toggle(_, true) => "Reactivate",
            Self::Toggle(_, false) => "Deactivate",
            Self::Cancel(_) => "Cancel request",
            Self::Perform(..) => "Perform reassessment",
            Self::Run => "Send due reminders",
            Self::Retry => "Retry failed reminders",
        }
    }
}
pub(super) struct PolicyPage {
    core: Arc<Core>,
    company: Company,
    available: bool,
    policy: Option<crate::Policy>,
    types: Vec<crate::ActionType>,
    requests: Vec<ReassessmentView>,
    due: Vec<DueReminder>,
    ledger: Vec<Reminder>,
    types_table: Table,
    requests_table: Table,
    due_table: Table,
    ledger_table: Table,
    can_types: bool,
    can_remind: bool,
    can_requests: bool,
    can_perform: bool,
    pending: Option<Pending>,
    flash: Flash,
    policy_draft: HashMap<String, String>,
    type_draft: HashMap<String, String>,
}
impl PolicyPage {
    pub(super) fn new(core: Arc<Core>, query: &HashMap<String, String>) -> Result<Self> {
        Ok(Self {
            core,
            company: Company::new(query),
            available: false,
            policy: None,
            types: vec![],
            requests: vec![],
            due: vec![],
            ledger: vec![],
            types_table: Table::new(
                "action-types",
                "types_grid",
                &[
                    ("name", "Name"),
                    ("code", "Code"),
                    ("provider", "Provider"),
                    ("state", "Status"),
                    ("controls", "Actions"),
                ],
            )?,
            requests_table: Table::new(
                "reassessment-requests",
                "requests_grid",
                &[
                    ("employee", "Employee"),
                    ("skill", "Skill"),
                    ("reason", "Reason"),
                    ("due", "Due"),
                    ("controls", "Actions"),
                ],
            )?,
            due_table: Table::new(
                "reminders-due",
                "due_grid",
                &[
                    ("rule", "Rule"),
                    ("employee", "Employee"),
                    ("skill", "Skill"),
                    ("action", "Action"),
                    ("due", "Due"),
                    ("recipients", "Recipients"),
                ],
            )?,
            ledger_table: Table::new(
                "reminders-ledger",
                "ledger_grid",
                &[
                    ("rule", "Rule"),
                    ("employee", "Employee"),
                    ("skill", "Skill"),
                    ("action", "Action"),
                    ("period", "Period"),
                    ("recipient", "Recipient account"),
                    ("state", "Status"),
                    ("failure", "Failure"),
                    ("sent", "Sent at"),
                ],
            )?,
            can_types: false,
            can_remind: false,
            can_requests: false,
            can_perform: false,
            pending: None,
            flash: Flash::default(),
            policy_draft: HashMap::new(),
            type_draft: HashMap::new(),
        })
    }
    pub(super) async fn load(&mut self, scope: &Scope, changed: bool) -> Result<()> {
        self.company.load(&self.core, scope, POLICY_MANAGE).await?;
        self.available = false;
        self.policy = None;
        self.types.clear();
        self.requests.clear();
        self.due.clear();
        self.ledger.clear();
        self.can_types = false;
        self.can_remind = false;
        self.can_requests = false;
        self.can_perform = false;
        if let Some(c) = self.company.selected.as_ref() {
            match self.core.skills.policy(scope, c.id).await {
                Ok(p) => {
                    self.policy = Some(p);
                    self.available = true;
                }
                Err(e) => self.flash.error(&message(&e)),
            }
            if self.available {
                self.can_types = self
                    .company
                    .allowed(&self.core, scope, CATALOG_MANAGE)
                    .await;
                self.can_remind = self
                    .company
                    .allowed(&self.core, scope, REMINDERS_SEND)
                    .await;
                self.can_requests = self
                    .company
                    .allowed(&self.core, scope, REASSESSMENTS_SUBMIT)
                    .await
                    || self
                        .company
                        .allowed(&self.core, scope, REASSESSMENTS_EXECUTE)
                        .await;
                self.can_perform = self
                    .company
                    .allowed(&self.core, scope, REASSESSMENTS_EXECUTE)
                    .await
                    && self
                        .company
                        .allowed(&self.core, scope, ASSESSMENTS_SUBMIT)
                        .await;
                self.types = self
                    .core
                    .skills
                    .list_action_types(scope, c.id)
                    .await
                    .unwrap_or_default();
                if self.can_requests {
                    self.requests = self
                        .core
                        .skills
                        .pending_reassessments(scope, c.id)
                        .await
                        .unwrap_or_default();
                }
                if self.can_remind {
                    self.due = self
                        .core
                        .skills
                        .due_reminders(scope, c.id, None)
                        .await
                        .unwrap_or_default();
                    self.ledger = self
                        .core
                        .skills
                        .reminder_ledger(scope, c.id, 300)
                        .await
                        .unwrap_or_default();
                }
            }
        }
        if !self.available {
            self.pending = None;
        }
        if changed {
            self.types_table.load(&self.types, |t| t.id.to_string());
            self.requests_table
                .load(&self.requests, |r| r.request.id.to_string());
            self.due_table.load(&self.due, |r| {
                format!("{}-{}-{:?}", r.rule, r.skill_id, r.employee_id)
            });
            self.ledger_table.load(&self.ledger, |r| r.id.to_string());
        }
        Ok(())
    }
    async fn authorize(&mut self, scope: &Scope, cap: &str) -> bool {
        let ok = self.company.allowed(&self.core, scope, POLICY_MANAGE).await
            && (self.company.allowed(&self.core, scope, cap).await
                || (cap == REASSESSMENTS_SUBMIT
                    && self
                        .company
                        .allowed(&self.core, scope, REASSESSMENTS_EXECUTE)
                        .await));
        if !ok {
            self.pending = None;
            self.flash
                .error("You no longer have permission to change this company's skills policy.");
        }
        ok
    }
    async fn event(&mut self, scope: &Scope, payload: &Value) -> Result<bool> {
        match text(payload, "type") {
            "clear-flash" => self.flash.clear(payload),
            "types_grid" => self.types_table.event(payload),
            "requests_grid" => self.requests_table.event(payload),
            "due_grid" => self.due_table.event(payload),
            "ledger_grid" => self.ledger_table.event(payload),
            "dismiss_confirm" => self.pending = None,
            "select_company" => {
                self.company.id = id(payload, "company_id");
                self.pending = None;
                self.policy_draft.clear();
                self.type_draft.clear();
                return Ok(true);
            }
            "save_policy" => {
                if !self.authorize(scope, POLICY_MANAGE).await {
                    return Ok(true);
                }
                for (name, _, _, _) in FIELDS {
                    self.policy_draft.insert(
                        (*name).into(),
                        text(payload, &format!("policy[{name}]"))
                            .chars()
                            .take(32)
                            .collect(),
                    );
                }
                let values = self.policy_changes();
                let result = match values {
                    Ok(c) => {
                        self.core
                            .skills
                            .put_policy(scope, self.company.id.unwrap_or_default(), &c)
                            .await
                    }
                    Err(e) => Err(e),
                };
                if self.flash.result(result, "Skills policy saved.") {
                    self.policy_draft.clear();
                }
                return Ok(true);
            }
            "create_action_type" => {
                if !self.authorize(scope, CATALOG_MANAGE).await {
                    return Ok(true);
                }
                self.type_draft = HashMap::from([
                    (
                        "code".into(),
                        text(payload, "type[code]").chars().take(81).collect(),
                    ),
                    (
                        "name".into(),
                        text(payload, "type[name]").chars().take(161).collect(),
                    ),
                    (
                        "requires_provider".into(),
                        text(payload, "type[requires_provider]").into(),
                    ),
                ]);
                let input = NewActionType {
                    code: self.type_draft["code"].clone(),
                    name: self.type_draft["name"].clone(),
                    requires_provider: self.type_draft["requires_provider"] == "true",
                };
                if self.flash.result(
                    self.core
                        .skills
                        .create_action_type(scope, self.company.id.unwrap_or_default(), &input)
                        .await,
                    "Action type added.",
                ) {
                    self.type_draft.clear();
                }
                return Ok(true);
            }
            "toggle_action_type" => {
                if !self.authorize(scope, CATALOG_MANAGE).await {
                    return Ok(true);
                }
                self.load(scope, false).await?;
                let Some(t) = self.types.iter().find(|t| Some(t.id) == id(payload, "id")) else {
                    self.flash.error("That record is not available to you.");
                    return Ok(false);
                };
                self.pending = Some(Pending {
                    company: self.company.id.unwrap_or_default(),
                    command: Command::Toggle(t.id, !t.active),
                    detail: format!("{} · {}", t.name, t.code),
                });
            }
            "cancel_reassessment" | "perform_reassessment" => {
                let perform = text(payload, "type") == "perform_reassessment";
                if !self
                    .authorize(
                        scope,
                        if perform {
                            REASSESSMENTS_EXECUTE
                        } else {
                            REASSESSMENTS_SUBMIT
                        },
                    )
                    .await
                {
                    return Ok(true);
                }
                if perform && !self.authorize(scope, ASSESSMENTS_SUBMIT).await {
                    return Ok(true);
                }
                self.load(scope, false).await?;
                let Some(r) = self
                    .requests
                    .iter()
                    .find(|r| Some(r.request.id) == id(payload, "id"))
                else {
                    self.flash.error("That item is no longer pending.");
                    return Ok(false);
                };
                let command = if perform {
                    let Ok(level) = text(payload, "assessed_level").parse::<i32>() else {
                        self.flash.error("Check the entered values.");
                        return Ok(false);
                    };
                    let evidence = text(payload, "evidence");
                    if evidence.trim().is_empty() {
                        self.flash.error("Evidence is required.");
                        return Ok(false);
                    }
                    let Ok(assessed_on) = date(text(payload, "assessed_on")) else {
                        self.flash.error("Choose a valid date.");
                        return Ok(false);
                    };
                    Command::Perform(
                        r.request.id,
                        AssessmentInput {
                            assessed_level: level,
                            evidence: evidence.chars().take(2001).collect(),
                            assessed_on,
                            ..AssessmentInput::default()
                        },
                    )
                } else {
                    Command::Cancel(r.request.id)
                };
                self.pending = Some(Pending {
                    company: self.company.id.unwrap_or_default(),
                    command,
                    detail: format!(
                        "{} · {} · {}",
                        r.employee_name,
                        r.skill_name.as_deref().unwrap_or_default(),
                        r.request.reason
                    ),
                });
            }
            "run_reminders" | "retry_reminders" => {
                if !self.authorize(scope, REMINDERS_SEND).await {
                    return Ok(true);
                }
                let run = text(payload, "type") == "run_reminders";
                self.pending=Some(Pending {company:self.company.id.unwrap_or_default(),command:if run{Command::Run}else{Command::Retry},detail:if run{"Queue due reminders as your account. Sent reminders are not sent again in the same period."}else{"Retry eligible failed or interrupted reminders in the current period."}.into()});
            }
            "confirm" => {
                if let Some(p) = self.pending.take() {
                    if Some(p.company) != self.company.id
                        || !self.authorize(scope, p.command.cap()).await
                    {
                        return Ok(true);
                    }
                    match p.command {
                        Command::Toggle(id, active) => {
                            self.flash.result(
                                self.core
                                    .skills
                                    .set_action_type_active(scope, p.company, id, active)
                                    .await,
                                "Action type updated.",
                            );
                        }
                        Command::Cancel(id) => {
                            self.flash.result(
                                self.core
                                    .skills
                                    .cancel_reassessment(scope, p.company, id)
                                    .await,
                                "Reassessment request cancelled.",
                            );
                        }
                        Command::Perform(id, input) => {
                            if self.authorize(scope, ASSESSMENTS_SUBMIT).await {
                                self.flash.result(
                                    self.core
                                        .skills
                                        .perform_reassessment(scope, p.company, id, &input)
                                        .await,
                                    "Reassessment submitted.",
                                );
                            }
                        }
                        Command::Run => {
                            self.flash.result(
                                self.core.skills.enqueue_reminders(scope, p.company).await,
                                "Reminders queued.",
                            );
                        }
                        Command::Retry => {
                            self.flash.result(
                                self.core
                                    .skills
                                    .retry_reminders(scope, p.company, None)
                                    .await,
                                "Failed reminders retried.",
                            );
                        }
                    }
                }
                return Ok(true);
            }
            _ => self.flash.error("That could not be done."),
        }
        Ok(false)
    }
    fn policy_changes(&self) -> crate::Result<PolicyChanges> {
        let v = |key: &str| {
            self.policy_draft
                .get(key)
                .and_then(|v| v.trim().parse().ok())
                .ok_or(SkillsError::InvalidPolicy)
        };
        Ok(PolicyChanges {
            reassessment_due_days: Some(v("reassessment_due_days")?),
            default_reassessment_months: Some(v("default_reassessment_months")?),
            reminder_window_days: Some(v("reminder_window_days")?),
            backup_minimum: Some(v("backup_minimum")?),
            multiplier_critical: Some(v("multiplier_critical")?),
            multiplier_essential: Some(v("multiplier_essential")?),
            multiplier_development: Some(v("multiplier_development")?),
        })
    }
    fn policy_form(&self) -> Result<Html> {
        let Some(p) = &self.policy else {
            return Ok(Html::EMPTY);
        };
        let values = [
            p.reassessment_due_days,
            p.default_reassessment_months,
            p.reminder_window_days,
            p.backup_minimum,
            p.multiplier_critical,
            p.multiplier_essential,
            p.multiplier_development,
        ];
        let fields = FIELDS
            .iter()
            .zip(values)
            .map(|((key, label, min, max), value)| {
                let text = self
                    .policy_draft
                    .get(*key)
                    .cloned()
                    .unwrap_or_else(|| value.to_string());
                let mut f = field(&format!("policy[{key}]"), label)
                    .kind(InputKind::Number)
                    .attr("min", &min.to_string())
                    .attr("max", &max.to_string())
                    .required()
                    .value(&text);
                if self.policy_draft.contains_key(*key)
                    && !text
                        .parse::<i64>()
                        .is_ok_and(|v| (*min..=*max).contains(&v))
                {
                    f = f.error(format!("Enter a whole number from {min} to {max}."));
                }
                f
            })
            .collect();
        section(
            "skills-policy",
            "Policy",
            form("skills-policy-form", "save_policy", fields, "Save policy")?,
        )
    }
    fn types(&self) -> Result<Html> {
        let rows = self
            .types
            .iter()
            .map(|t| {
                Ok(row("action-type", t.id)
                    .slot("name", Html::text(&t.name))
                    .slot("code", Html::text(&t.code))
                    .slot(
                        "provider",
                        Html::text(if t.requires_provider {
                            "Needs a provider"
                        } else {
                            ""
                        }),
                    )
                    .slot(
                        "state",
                        Html::text(if t.active { "Active" } else { "Inactive" }),
                    )
                    .slot(
                        "controls",
                        if self.can_types {
                            button(
                                "toggle_action_type",
                                if t.active { "Deactivate" } else { "Reactivate" },
                                t.id,
                            )?
                        } else {
                            Html::EMPTY
                        },
                    ))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut body = self.types_table.render(
            "Development action types",
            rows,
            "No action types yet. Actions cannot be proposed until one is added.",
        )?;
        if self.can_types {
            body.push(&form(
                "action-type-form",
                "create_action_type",
                vec![
                    field("type[code]", "Type code")
                        .required()
                        .attr("maxlength", "80")
                        .value(self.type_draft.get("code").cloned().unwrap_or_default()),
                    field("type[name]", "Type name")
                        .required()
                        .attr("maxlength", "160")
                        .value(self.type_draft.get("name").cloned().unwrap_or_default()),
                    field("type[requires_provider]", "Needs a trainer or provider")
                        .kind(InputKind::Checkbox)
                        .checked(
                            self.type_draft
                                .get("requires_provider")
                                .is_some_and(|s| s == "true"),
                        ),
                ],
                "Add type",
            )?);
        }
        section("skills-action-types", "Development action types", body)
    }
    fn requests(&self) -> Result<Html> {
        if !self.can_requests {
            return Ok(Html::EMPTY);
        }
        let rows = self
            .requests
            .iter()
            .map(|r| {
                let mut controls = button("cancel_reassessment", "Cancel", r.request.id)?;
                if self.can_perform {
                    controls.push(&form(
                        &format!("reassessment-form-{}", r.request.id),
                        "perform_reassessment",
                        vec![
                            hidden("id", r.request.id),
                            field("assessed_level", "Assessed level")
                                .id(format!("reassessment-level-{}", r.request.id))
                                .kind(InputKind::Number)
                                .attr("min", "0")
                                .attr("max", "20")
                                .required(),
                            field("evidence", "Evidence")
                                .id(format!("reassessment-evidence-{}", r.request.id))
                                .attr("maxlength", "2000")
                                .required(),
                            field("assessed_on", "Assessed on")
                                .id(format!("reassessment-date-{}", r.request.id))
                                .kind(InputKind::Date)
                                .value(crate::assessments::today().to_string()),
                        ],
                        "Perform reassessment",
                    )?);
                }
                Ok(row("reassessment-request", r.request.id)
                    .slot("employee", Html::text(&r.employee_name))
                    .slot(
                        "skill",
                        Html::text(r.skill_name.as_deref().unwrap_or_default()),
                    )
                    .slot("reason", Html::text(&r.request.reason))
                    .slot(
                        "due",
                        Html::text(&r.request.due_on.map(|d| d.to_string()).unwrap_or_default()),
                    )
                    .slot("controls", controls))
            })
            .collect::<Result<Vec<_>>>()?;
        section(
            "skills-reassessments",
            "Reassessment requests",
            self.requests_table.render(
                "Reassessment requests",
                rows,
                "No reassessment requests are waiting.",
            )?,
        )
    }
    fn reminders(&self) -> Result<Html> {
        if !self.can_remind {
            return Ok(Html::EMPTY);
        }
        let rows = self
            .due
            .iter()
            .enumerate()
            .map(|(i, r)| {
                row("reminder-due", i as i64)
                    .slot("rule", Html::text(&label(&r.rule)))
                    .slot(
                        "employee",
                        Html::text(&r.employee_id.map(|id| id.to_string()).unwrap_or_default()),
                    )
                    .slot("skill", Html::text(&r.skill_id.to_string()))
                    .slot(
                        "action",
                        Html::text(&r.action_id.map(|id| id.to_string()).unwrap_or_default()),
                    )
                    .slot("due", Html::text(&r.due_on.to_string()))
                    .slot(
                        "recipients",
                        Html::text(&if r.recipients.is_empty() {
                            "no one to tell".into()
                        } else {
                            format!("{} to tell", r.recipients.len())
                        }),
                    )
            })
            .collect();
        let mut body = self
            .due_table
            .render("Due reminders", rows, "Nothing is due today.")?;
        body.push(&button("run_reminders", "Send due reminders", 0)?);
        body.push(&button("retry_reminders", "Retry failed reminders", 0)?);
        let mut result = section("skills-reminders", "Reminders", body)?;
        let rows = self
            .ledger
            .iter()
            .map(|r| {
                row("reminder", r.id)
                    .slot("rule", Html::text(&label(&r.rule)))
                    .slot(
                        "employee",
                        Html::text(&r.employee_id.map(|id| id.to_string()).unwrap_or_default()),
                    )
                    .slot("skill", Html::text(&r.skill_id.to_string()))
                    .slot(
                        "action",
                        Html::text(&r.action_id.map(|id| id.to_string()).unwrap_or_default()),
                    )
                    .slot("period", Html::text(&r.period_key))
                    .slot("recipient", Html::text(&r.recipient_user_id.to_string()))
                    .slot("state", Html::text(&label(&r.state)))
                    .slot(
                        "failure",
                        Html::text(r.failure.as_deref().unwrap_or_default()),
                    )
                    .slot(
                        "sent",
                        Html::text(&r.sent_at.map(|d| d.to_string()).unwrap_or_default()),
                    )
            })
            .collect();
        result.push(&section(
            "skills-reminder-ledger",
            "Reminder ledger",
            self.ledger_table.render(
                "Reminder ledger (latest 300)",
                rows,
                "No reminders have been recorded.",
            )?,
        )?);
        Ok(result)
    }
    pub(super) fn render(&self) -> Result<String> {
        let body = if self.company.selected.is_none() {
            render(
                &EmptyState::missing("No active company is available for skills policy.", "")
                    .id("skills-policy-no-company"),
            )?
        } else if !self.available {
            render(
                &EmptyState::missing(
                    "Skills policy is unavailable while this company's workforce is not current.",
                    "",
                )
                .id("skills-policy-unavailable"),
            )?
        } else {
            render(&PolicyLayout {
                policy: self.policy_form()?,
                types: self.types()?,
                requests: self.requests()?,
                reminders: self.reminders()?,
            })?
        };
        let confirmation = if let Some(p) = &self.pending {
            confirm(
                true,
                &self.company,
                &format!("{}?", p.command.verb()),
                &p.detail,
                p.command.verb(),
            )?
        } else {
            Html::EMPTY
        };
        View {
            flash: self.flash.map(),
            page_id: "skills-policy-page",
            title: "Skills policy",
            subtitle: self
                .company
                .selected
                .as_ref()
                .map(|c| format!("{} · Assessment, priority and reminder settings", c.name))
                .unwrap_or_default(),
            has_company: !self.company.companies.is_empty(),
            company_form: "skills-policy-company-form",
            company: self.company.field("skills-policy"),
            body,
            confirm: confirmation,
        }
        .render()
        .map_err(|_| Error::Internal)
    }
}
const FIELDS: &[(&str, &str, i64, i64)] = &[
    (
        "reassessment_due_days",
        "Days a reassessment request stays open",
        1,
        365,
    ),
    (
        "default_reassessment_months",
        "Default months between assessments",
        1,
        120,
    ),
    (
        "reminder_window_days",
        "Days before validity ends to remind",
        0,
        365,
    ),
    (
        "backup_minimum",
        "Holders needed to cover a critical skill",
        1,
        100,
    ),
    (
        "multiplier_critical",
        "Critical priority multiplier",
        0,
        100,
    ),
    (
        "multiplier_essential",
        "Essential priority multiplier",
        0,
        100,
    ),
    (
        "multiplier_development",
        "Development priority multiplier",
        0,
        100,
    ),
];
impl LiveState<Scope, Error> for PolicyPage {
    fn update<'a>(&'a mut self, request: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            let changed = match request.input {
                LiveInput::Event(p) => self.event(request.scope, p).await?,
                LiveInput::Params(q) => {
                    self.company.id = params::positive_id(q.get("company_id").map(String::as_str));
                    self.pending = None;
                    self.policy_draft.clear();
                    self.type_draft.clear();
                    true
                }
                LiveInput::Refresh => true,
            };
            self.load(request.scope, changed).await?;
            Ok(LiveBody {
                html: self.render()?,
                query: self.company.query(),
                rows: self.types.len() + self.requests.len() + self.ledger.len(),
                ..LiveBody::default()
            })
        })
    }
}

#[derive(askama::Template)]
#[template(path = "pk6_policy_layout.html")]
struct PolicyLayout {
    policy: Html,
    types: Html,
    requests: Html,
    reminders: Html,
}
