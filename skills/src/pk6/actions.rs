//! Mirrors Bilimbi Skills.Web.ActionsLive: proposal, independent approval,
//! owner progress and append-only history. Confirmation retains its own target.
use super::*;
use crate::{
    ActionGroup, ActionInput, ActionPlan, ActionSource, ActionType, ActionView, AssessableEmployee,
    ScoreView, Skill, ACTIONS_APPROVE, ACTIONS_MANAGE, ACTIONS_UPDATE, ACTIONS_VIEW,
};
use askama::Template;
use bilibi_ui::{
    page::{LiveBody, LiveFuture, LiveInput, LiveRequest},
    Tab, Tabs,
};

struct Pending {
    company: i64,
    command: Command,
    detail: String,
}
enum Command {
    Propose(Box<ActionInput>),
    Approve(i64),
    Start(i64),
    Hold(i64, String),
    Complete(i64, String, jiff::civil::Date),
    Cancel(i64, String),
    Link(i64, i64),
}
impl Command {
    fn capability(&self) -> &'static str {
        match self {
            Self::Propose(_) => ACTIONS_MANAGE,
            Self::Approve(_) => ACTIONS_APPROVE,
            _ => ACTIONS_UPDATE,
        }
    }
    fn verb(&self) -> &'static str {
        match self {
            Self::Propose(_) => "Propose action",
            Self::Approve(_) => "Approve",
            Self::Start(_) => "Start",
            Self::Hold(..) => "Hold",
            Self::Complete(..) => "Complete intervention",
            Self::Cancel(..) => "Cancel action",
            Self::Link(..) => "Close against reassessment",
        }
    }
    fn success(&self) -> &'static str {
        match self {
            Self::Propose(_) => "Development action proposed.",
            Self::Approve(_) => "Action approved.",
            Self::Start(_) => "Action started.",
            Self::Hold(..) => "Action put on hold.",
            Self::Complete(..) => "Intervention recorded; a reassessment is now due.",
            Self::Cancel(..) => "Action cancelled.",
            Self::Link(..) => "Action closed against the reassessment.",
        }
    }
}
pub(super) struct Actions {
    core: Arc<Core>,
    company: Company,
    group: ActionGroup,
    owned: bool,
    rows: Vec<ActionView>,
    types: Vec<ActionType>,
    people: Vec<AssessableEmployee>,
    skills: Vec<Skill>,
    gaps: Vec<ScoreView>,
    table: Table,
    history_table: Table,
    history: Vec<crate::ActionEvent>,
    history_id: Option<i64>,
    pending: Option<Pending>,
    can_propose: bool,
    can_approve: bool,
    can_update: bool,
    linked: Option<i64>,
    available: bool,
    flash: Flash,
    draft: HashMap<String, String>,
    errors: Vec<crate::FieldError>,
    request_key: String,
}
impl Actions {
    pub(super) fn new(core: Arc<Core>, query: &HashMap<String, String>) -> Result<Self> {
        Ok(Self {
            core,
            company: Company::new(query),
            group: if query.get("group").is_some_and(|g| g == "closed") {
                ActionGroup::Closed
            } else {
                ActionGroup::Open
            },
            owned: query.get("audience").is_some_and(|g| g == "owned"),
            rows: vec![],
            types: vec![],
            people: vec![],
            skills: vec![],
            gaps: vec![],
            table: Table::new(
                "actions",
                "actions_grid",
                &[
                    ("employee", "Employee"),
                    ("skill", "Skill"),
                    ("type", "Action type"),
                    ("levels", "Levels"),
                    ("status", "Status"),
                    ("objective", "Objective"),
                    ("owner", "Owner"),
                    ("coordinator", "Coordinator"),
                    ("dates", "Start and due"),
                    ("priority", "Priority"),
                    ("controls", "Actions"),
                ],
            )?,
            history_table: Table::new(
                "action-history",
                "history_grid",
                &[
                    ("event", "Event"),
                    ("status", "Status"),
                    ("comment", "Comment"),
                    ("evidence", "Evidence"),
                    ("actor", "Account"),
                    ("at", "Recorded at"),
                ],
            )?,
            history: vec![],
            history_id: None,
            pending: None,
            can_propose: false,
            can_approve: false,
            can_update: false,
            linked: None,
            available: false,
            flash: Flash::default(),
            draft: HashMap::new(),
            errors: vec![],
            request_key: key(),
        })
    }
    pub(super) async fn load(&mut self, scope: &Scope, changed: bool) -> Result<()> {
        self.company.load(&self.core, scope, ACTIONS_VIEW).await?;
        self.rows.clear();
        self.types.clear();
        self.people.clear();
        self.skills.clear();
        self.gaps.clear();
        self.available = false;
        self.can_propose = false;
        self.can_approve = false;
        self.can_update = false;
        self.linked = None;
        if let Some(c) = self.company.selected.as_ref() {
            self.can_propose = self
                .company
                .allowed(&self.core, scope, ACTIONS_MANAGE)
                .await;
            self.can_approve = self
                .company
                .allowed(&self.core, scope, ACTIONS_APPROVE)
                .await;
            self.can_update = self
                .company
                .allowed(&self.core, scope, ACTIONS_UPDATE)
                .await;
            let client = self.core.db.get().await?;
            self.linked = self
                .core
                .skills
                .workforce
                .linked_employee(&**client, scope, c.id)
                .await
                .unwrap_or(None);
            drop(client);
            let result = if self.owned {
                self.core
                    .skills
                    .owned_actions(scope, c.id, self.group)
                    .await
            } else {
                self.core.skills.list_actions(scope, c.id, self.group).await
            };
            match result {
                Ok(rows) => {
                    self.rows = rows;
                    self.available = true;
                    self.types = self
                        .core
                        .skills
                        .list_action_types(scope, c.id)
                        .await
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|t| t.active)
                        .collect();
                    if self.can_propose {
                        self.people = self
                            .core
                            .skills
                            .action_people(scope, c.id)
                            .await
                            .unwrap_or_default();
                        self.skills = self
                            .core
                            .skills
                            .list_skills(scope, c.id)
                            .await
                            .unwrap_or_default()
                            .into_iter()
                            .filter(|s| s.active)
                            .collect();
                        self.gaps = self.core.skills.gaps(scope, c.id).await.unwrap_or_default();
                    }
                }
                Err(e) => self.flash.error(&message(&e)),
            }
            self.history = if let Some(id) = self.history_id {
                self.core
                    .skills
                    .action_events(scope, c.id, id)
                    .await
                    .unwrap_or_default()
            } else {
                vec![]
            };
        }
        if !self.available {
            self.pending = None;
            self.history.clear();
        }
        if changed {
            self.table.load(&self.rows, |r| r.action.id.to_string());
            self.history_table.load(&self.history, |r| r.id.to_string());
        }
        Ok(())
    }
    fn progressable(&self, a: &crate::Action) -> bool {
        self.can_propose || (self.can_update && self.linked == Some(a.owner_employee_id))
    }
    fn query(&self) -> HashMap<String, String> {
        let mut q = self.company.query();
        if self.group == ActionGroup::Closed {
            q.insert("group".into(), "closed".into());
        }
        if self.owned {
            q.insert("audience".into(), "owned".into());
        }
        q
    }
    async fn authorize(&mut self, scope: &Scope, cap: &str) -> bool {
        let ok = self.company.allowed(&self.core, scope, ACTIONS_VIEW).await
            && (self.company.allowed(&self.core, scope, cap).await
                || (cap == ACTIONS_UPDATE
                    && self
                        .company
                        .allowed(&self.core, scope, ACTIONS_MANAGE)
                        .await));
        if !ok {
            self.pending = None;
            self.flash.error("You cannot do that for this company.");
        }
        ok
    }
    async fn event(&mut self, scope: &Scope, payload: &Value) -> Result<bool> {
        let event = text(payload, "type");
        match event {
            "clear-flash" => self.flash.clear(payload),
            "actions_grid" => self.table.event(payload),
            "history_grid" => self.history_table.event(payload),
            "dismiss_confirm" => self.pending = None,
            "select_company" => {
                self.company.id = id(payload, "company_id");
                self.pending = None;
                self.history_id = None;
                self.draft.clear();
                self.request_key = key();
                return Ok(true);
            }
            "show_history" => {
                self.load(scope, false).await?;
                self.history_id =
                    id(payload, "id").filter(|id| self.rows.iter().any(|r| r.action.id == *id));
                return Ok(true);
            }
            "confirm" => {
                if let Some(p) = self.pending.take() {
                    if self.company.id != Some(p.company)
                        || !self.authorize(scope, p.command.capability()).await
                    {
                        return Ok(true);
                    }
                    let s = &self.core.skills;
                    let result = match &p.command {
                        Command::Propose(a) => s.propose_action(scope, p.company, a).await,
                        Command::Approve(id) => s.approve_action(scope, p.company, *id).await,
                        Command::Start(id) => s.start_action(scope, p.company, *id).await,
                        Command::Hold(id, reason) => {
                            s.hold_action(scope, p.company, *id, reason).await
                        }
                        Command::Complete(id, evidence, due) => {
                            s.complete_action(scope, p.company, *id, evidence, *due)
                                .await
                        }
                        Command::Cancel(id, reason) => {
                            s.cancel_action(scope, p.company, *id, reason).await
                        }
                        Command::Link(id, assessment) => {
                            s.link_action_reassessment(scope, p.company, *id, *assessment)
                                .await
                        }
                    };
                    if matches!(p.command, Command::Propose(_)) {
                        self.errors = match &result {
                            Err(SkillsError::Invalid(errors)) => errors.clone(),
                            _ => vec![],
                        };
                    }
                    if self.flash.result(result, p.command.success())
                        && matches!(p.command, Command::Propose(_))
                    {
                        self.draft.clear();
                        self.request_key = key();
                    }
                }
                return Ok(true);
            }
            "create_action" => {
                if !self.authorize(scope, ACTIONS_MANAGE).await {
                    return Ok(true);
                }
                // Known fields only, preserving the 2,000-character evidence limit.
                for name in ACTION_FIELDS {
                    self.draft.insert(
                        (*name).into(),
                        text(payload, &format!("action[{name}]"))
                            .chars()
                            .take(2001)
                            .collect(),
                    );
                }
                match self.proposal() {
                    Ok(a) => {
                        self.pending = Some(Pending {
                            company: self.company.id.unwrap_or_default(),
                            detail: a.plan.objective.clone(),
                            command: Command::Propose(Box::new(a)),
                        });
                    }
                    Err(e) => self.flash.error(&message(&e)),
                }
            }
            "approve_action"
            | "start_action"
            | "hold_action"
            | "complete_action"
            | "cancel_action"
            | "link_action_reassessment" => {
                let cap = if event == "approve_action" {
                    ACTIONS_APPROVE
                } else {
                    ACTIONS_UPDATE
                };
                if !self.authorize(scope, cap).await {
                    return Ok(true);
                }
                self.load(scope, false).await?;
                let target = id(payload, "id");
                let Some(view) = self.rows.iter().find(|r| Some(r.action.id) == target) else {
                    self.flash.error("That record is not available to you.");
                    return Ok(false);
                };
                let a = &view.action;
                let aid = a.id;
                if event != "approve_action" && !self.progressable(a) {
                    self.flash.error("You cannot do that for this company.");
                    return Ok(false);
                }
                let command = match event {
                    "approve_action" => Command::Approve(aid),
                    "start_action" => Command::Start(aid),
                    "hold_action" | "cancel_action" => {
                        let reason = text(payload, "reason").trim();
                        if reason.is_empty() {
                            self.flash.error("A reason is required.");
                            return Ok(false);
                        }
                        if reason.chars().count() > 2000 {
                            self.flash.error("Check the entered values.");
                            return Ok(false);
                        }
                        if event == "hold_action" {
                            Command::Hold(aid, reason.into())
                        } else {
                            Command::Cancel(aid, reason.into())
                        }
                    }
                    "complete_action" => {
                        let evidence = text(payload, "complete[evidence]").trim();
                        if evidence.is_empty() {
                            self.flash.error("Evidence is required.");
                            return Ok(false);
                        }
                        let Ok(Some(due)) = date(text(payload, "complete[reassessment_due_on]"))
                        else {
                            self.flash.error("Choose a valid date.");
                            return Ok(false);
                        };
                        Command::Complete(aid, evidence.chars().take(2001).collect(), due)
                    }
                    _ => {
                        let Some(assessment) = id(payload, "assessment_id")
                            .filter(|id| view.reassessments.iter().any(|a| a.id == *id))
                        else {
                            self.flash.error(
                                "That is not a finalized reassessment after the intervention.",
                            );
                            return Ok(false);
                        };
                        Command::Link(aid, assessment)
                    }
                };
                self.pending = Some(Pending {
                    company: self.company.id.unwrap_or_default(),
                    detail: format!(
                        "{} · {} · {}",
                        a.employee_name,
                        view.skill_name.as_deref().unwrap_or_default(),
                        a.objective
                    ),
                    command,
                });
            }
            _ => self.flash.error("That could not be done."),
        }
        Ok(false)
    }
    fn proposal(&self) -> crate::Result<ActionInput> {
        let v = |k: &str| self.draft.get(k).map(String::as_str).unwrap_or_default();
        let required_id =
            |k: &str| params::positive_id(Some(v(k))).ok_or(SkillsError::InvalidAction);
        let source = if let Some(id) = params::positive_id(Some(v("assessment_id"))) {
            ActionSource::Assessment(id)
        } else {
            ActionSource::Manual {
                employee_id: required_id("employee_id")?,
                skill_id: required_id("skill_id")?,
                starting_level: v("starting_level")
                    .parse()
                    .map_err(|_| SkillsError::InvalidAction)?,
                target_level: v("target_level")
                    .parse()
                    .map_err(|_| SkillsError::InvalidAction)?,
                criticality: v("criticality").into(),
                mandatory: false,
                reason: v("manual_reason").into(),
            }
        };
        let plan = ActionPlan {
            action_type_id: required_id("action_type_id")?,
            owner_employee_id: required_id("owner_employee_id")?,
            coordinator_employee_id: required_id("coordinator_employee_id")?,
            provider_employee_id: params::positive_id(Some(v("provider_employee_id"))),
            provider_name: optional(v("provider_name")),
            objective: v("objective").into(),
            intervention: v("intervention").into(),
            expected_evidence: v("expected_evidence").into(),
            next_steps: None,
            start_on: date(v("start_on"))?,
            due_on: date(v("due_on"))?,
        };
        Ok(ActionInput {
            source,
            plan,
            request_key: self.request_key.clone(),
        })
    }
    fn proposal_form(&self) -> Result<Html> {
        if !self.can_propose {
            return Ok(Html::EMPTY);
        }
        if self.types.is_empty() {
            return section(
                "action-proposal",
                "Propose an action",
                render(
                    &EmptyState::missing(
                        "Add an action type under Skills policy before proposing an action.",
                        "",
                    )
                    .id("action-types-empty"),
                )?,
            );
        }
        if self.people.is_empty() {
            return Ok(Html::EMPTY);
        }
        let people = || {
            self.people
                .iter()
                .map(|p| (p.name.clone(), p.id.to_string()))
                .collect::<Vec<_>>()
        };
        let mut gaps = vec![("Manual (state the gap below)".into(), String::new())];
        gaps.extend(self.gaps.iter().map(|g| {
            (
                format!(
                    "{} · {} · level {} of {}",
                    g.employee_name,
                    g.skill_name.as_deref().unwrap_or_default(),
                    g.score.current_level,
                    g.score.required_level
                ),
                g.score.assessment_id.to_string(),
            )
        }));
        let mut providers = vec![("No internal provider".into(), String::new())];
        providers.extend(people());
        let f = |key: &str, label: &str| field(&format!("action[{key}]"), label);
        let mut fields = vec![
            select("action[assessment_id]", "Assessment gap", gaps),
            select(
                "action[action_type_id]",
                "Action type",
                self.types
                    .iter()
                    .map(|t| (t.name.clone(), t.id.to_string()))
                    .collect(),
            ),
            select("action[employee_id]", "Employee (manual)", people()),
            select(
                "action[skill_id]",
                "Skill (manual)",
                self.skills
                    .iter()
                    .map(|s| (s.name.clone(), s.id.to_string()))
                    .collect(),
            ),
            f("starting_level", "Starting level (manual)")
                .kind(InputKind::Number)
                .attr("min", "0")
                .attr("max", "20"),
            f("target_level", "Target level (manual)")
                .kind(InputKind::Number)
                .attr("min", "0")
                .attr("max", "20"),
            select(
                "action[criticality]",
                "Criticality (manual)",
                crate::CRITICALITIES
                    .iter()
                    .map(|c| (label(c), (*c).into()))
                    .collect(),
            ),
            f("manual_reason", "Reason for a manual action").attr("maxlength", "1000"),
            f("objective", "Objective")
                .required()
                .attr("maxlength", "2000"),
            f("intervention", "Intervention")
                .required()
                .attr("maxlength", "2000"),
            f("expected_evidence", "Expected evidence")
                .required()
                .attr("maxlength", "2000"),
            select("action[owner_employee_id]", "Owner", people()),
            select("action[coordinator_employee_id]", "Coordinator", people()),
            select(
                "action[provider_employee_id]",
                "Trainer or coach",
                providers,
            ),
            f("provider_name", "External provider").attr("maxlength", "160"),
            f("start_on", "Start")
                .kind(InputKind::Date)
                .value(crate::assessments::today().to_string()),
            f("due_on", "Due").kind(InputKind::Date).required(),
        ];
        for f in &mut fields {
            let key = f
                .name
                .strip_prefix("action[")
                .and_then(|n| n.strip_suffix(']'))
                .unwrap_or_default();
            if let Some(v) = self.draft.get(key) {
                f.value.clone_from(v);
            }
            f.errors = self
                .errors
                .iter()
                .filter(|e| e.field == key)
                .map(|e| e.message.clone())
                .collect();
        }
        section(
            "action-proposal",
            "Propose an action",
            form("action-form", "create_action", fields, "Propose action")?,
        )
    }
    fn controls(&self, v: &ActionView) -> Result<Html> {
        let a = &v.action;
        let mut h = Html::EMPTY;
        if self.can_approve && a.status == "proposed" {
            h.push(&button("approve_action", "Approve", a.id)?);
        }
        if self.progressable(a) {
            if ["not_started", "scheduled", "on_hold"].contains(&a.status.as_str()) {
                h.push(&button("start_action", "Start", a.id)?);
            }
            if ["not_started", "scheduled", "in_progress"].contains(&a.status.as_str()) {
                h.push(&form(
                    &format!("hold-form-{}", a.id),
                    "hold_action",
                    vec![
                        hidden("id", a.id),
                        field("reason", "Reason for hold")
                            .id(format!("hold-reason-{}", a.id))
                            .required()
                            .attr("maxlength", "2000"),
                    ],
                    "Hold",
                )?);
            }
            if ["not_started", "scheduled", "in_progress", "on_hold"].contains(&a.status.as_str()) {
                h.push(&form(
                    &format!("complete-form-{}", a.id),
                    "complete_action",
                    vec![
                        hidden("id", a.id),
                        field("complete[evidence]", "Completion evidence")
                            .id(format!("completion-evidence-{}", a.id))
                            .required()
                            .attr("maxlength", "2000"),
                        field("complete[reassessment_due_on]", "Reassessment due")
                            .id(format!("reassessment-due-{}", a.id))
                            .kind(InputKind::Date)
                            .required(),
                    ],
                    "Complete intervention",
                )?);
            }
            if a.status == "pending_reassessment" {
                h.push(&form(
                    &format!("link-form-{}", a.id),
                    "link_action_reassessment",
                    vec![
                        hidden("id", a.id),
                        select(
                            "assessment_id",
                            "Finalized reassessment",
                            v.reassessments
                                .iter()
                                .map(|r| {
                                    (
                                        format!(
                                            "{} · level {}",
                                            r.assessed_on
                                                .map(|d| d.to_string())
                                                .unwrap_or_default(),
                                            r.assessed_level
                                        ),
                                        r.id.to_string(),
                                    )
                                })
                                .collect(),
                        )
                        .id(format!("reassessment-{}", a.id)),
                    ],
                    "Close against reassessment",
                )?);
            }
            if !["completed", "cancelled"].contains(&a.status.as_str()) {
                h.push(&form(
                    &format!("cancel-form-{}", a.id),
                    "cancel_action",
                    vec![
                        hidden("id", a.id),
                        field("reason", "Reason for cancelling")
                            .id(format!("cancel-reason-{}", a.id))
                            .required()
                            .attr("maxlength", "2000"),
                    ],
                    "Cancel",
                )?);
            }
        }
        h.push(&button("show_history", "History", a.id)?);
        Ok(h)
    }
    pub(super) fn render(&self) -> Result<String> {
        let mut body = Html::EMPTY;
        if self.company.selected.is_none() {
            body = render(
                &EmptyState::missing(
                    "No active company is available for development actions.",
                    "",
                )
                .id("actions-no-company"),
            )?;
        } else if !self.available {
            body=render(&EmptyState::missing("Development actions are unavailable while this company's workforce is not current.","").id("actions-unavailable"))?;
        } else {
            let mut open = self.query();
            open.remove("group");
            let mut closed = open.clone();
            closed.insert("group".into(), "closed".into());
            let open = params::address("/people/skills/actions", &open);
            let closed = params::address("/people/skills/actions", &closed);
            body.push(&render(&Tabs {
                id: "actions-group",
                label: "Action status",
                tabs: vec![
                    Tab::patch(&open, "Open", self.group == ActionGroup::Open),
                    Tab::patch(&closed, "Closed", self.group == ActionGroup::Closed),
                ],
                ..Tabs::default()
            })?);
            let mut all = self.query();
            all.remove("audience");
            let mut owned = all.clone();
            owned.insert("audience".into(), "owned".into());
            let all = params::address("/people/skills/actions", &all);
            let owned = params::address("/people/skills/actions", &owned);
            if self.can_update {
                body.push(&render(&Tabs {
                    id: "actions-audience",
                    label: "Action ownership",
                    tabs: vec![
                        Tab::patch(&all, "In reach", !self.owned),
                        Tab::patch(&owned, "Owned", self.owned),
                    ],
                    ..Tabs::default()
                })?);
            }
            let rows = self
                .rows
                .iter()
                .map(|v| {
                    let a = &v.action;
                    Ok(row("action", a.id)
                        .slot("employee", Html::text(&a.employee_name))
                        .slot(
                            "skill",
                            Html::text(v.skill_name.as_deref().unwrap_or_default()),
                        )
                        .slot(
                            "type",
                            Html::text(v.type_name.as_deref().unwrap_or_default()),
                        )
                        .slot(
                            "levels",
                            Html::text(&format!("{} to {}", a.starting_level, a.target_level)),
                        )
                        .slot(
                            "status",
                            Html::text(&if a.status == "completed" {
                                format!(
                                    "{} ({}, now level {})",
                                    label(&a.status),
                                    label(&a.closure),
                                    a.post_level.unwrap_or_default()
                                )
                            } else {
                                label(&a.status)
                            }),
                        )
                        .slot("objective", Html::text(&a.objective))
                        .slot("owner", Html::text(&v.owner_name))
                        .slot("coordinator", Html::text(&v.coordinator_name))
                        .slot(
                            "dates",
                            Html::text(&format!(
                                "{} to {}",
                                a.start_on.map(|d| d.to_string()).unwrap_or_default(),
                                a.due_on.map(|d| d.to_string()).unwrap_or_default()
                            )),
                        )
                        .slot("priority", Html::text(&a.priority_explanation))
                        .slot("controls", self.controls(v)?))
                })
                .collect::<Result<Vec<_>>>()?;
            body.push(&section(
                "actions-section",
                "Actions",
                self.table.render(
                    "Actions",
                    rows,
                    if self.group == ActionGroup::Open {
                        "No open development actions."
                    } else {
                        "No closed development actions."
                    },
                )?,
            )?);
            if let Some(aid) = self.history_id {
                let rows = self
                    .history
                    .iter()
                    .map(|e| {
                        row("action-event", e.id)
                            .slot("event", Html::text(&label(&e.event_type)))
                            .slot(
                                "status",
                                Html::text(e.to_status.as_deref().unwrap_or_default()),
                            )
                            .slot(
                                "comment",
                                Html::text(e.comment.as_deref().unwrap_or_default()),
                            )
                            .slot(
                                "evidence",
                                Html::text(e.evidence.as_deref().unwrap_or_default()),
                            )
                            .slot("actor", Html::text(&e.actor_user_id.to_string()))
                            .slot(
                                "at",
                                Html::text(
                                    &e.inserted_at.map(|d| d.to_string()).unwrap_or_default(),
                                ),
                            )
                    })
                    .collect();
                body.push(&section(
                    &format!("action-history-{aid}"),
                    "History",
                    self.history_table
                        .render("Action history", rows, "No action history.")?,
                )?);
            }
            body.push(&self.proposal_form()?);
        }
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
            page_id: "actions-page",
            title: "Development actions",
            subtitle: self
                .company
                .selected
                .as_ref()
                .map(|c| format!("{} · Closing skill gaps with owned, evidenced work", c.name))
                .unwrap_or_default(),
            has_company: !self.company.companies.is_empty(),
            company_form: "actions-company-form",
            company: self.company.field("actions"),
            body,
            confirm: confirmation,
        }
        .render()
        .map_err(|_| Error::Internal)
    }
}
const ACTION_FIELDS: &[&str] = &[
    "assessment_id",
    "action_type_id",
    "employee_id",
    "skill_id",
    "starting_level",
    "target_level",
    "criticality",
    "manual_reason",
    "objective",
    "intervention",
    "expected_evidence",
    "owner_employee_id",
    "coordinator_employee_id",
    "provider_employee_id",
    "provider_name",
    "start_on",
    "due_on",
];
impl LiveState<Scope, Error> for Actions {
    fn update<'a>(&'a mut self, request: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            let changed = match request.input {
                LiveInput::Event(p) => self.event(request.scope, p).await?,
                LiveInput::Params(q) => {
                    self.company.id = params::positive_id(q.get("company_id").map(String::as_str));
                    self.group = if q.get("group").is_some_and(|g| g == "closed") {
                        ActionGroup::Closed
                    } else {
                        ActionGroup::Open
                    };
                    self.owned = q.get("audience").is_some_and(|a| a == "owned");
                    self.pending = None;
                    self.history_id = None;
                    true
                }
                LiveInput::Refresh => true,
            };
            self.load(request.scope, changed).await?;
            Ok(LiveBody {
                html: self.render()?,
                query: self.query(),
                rows: self.rows.len(),
                ..LiveBody::default()
            })
        })
    }
}
