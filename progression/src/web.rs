//! Bilimbi Progression.Web.PolicyLive and MyLive. Every business write goes
//! through Progression; view state stays in the shared list components.
use crate::{
    Criterion, DraftInput, MissingEvidence, PageOptions, PerformanceRule, Period, Policy,
    ProfilePin, Progression, ProgressionError, Standing, Status, POLICY_MANAGE, POLICY_VIEW,
    SELF_VIEW,
};
use askama::Template;
use bilibi_database::{Error, Result};
use bilibi_people_skills::{Profile, Skills};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::{
        LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState, MountFuture, Page, PageBody,
        PageFuture, PageRequest,
    },
    params, ActionLink, BuiltinColumn, Button, ButtonVariant, Card, Choice, ConfirmDialog,
    EmptyState, Fact, Facts, Field, FlashMap, FlexRow, FlexTable, Form, Html, InputKind,
    ListColumns, ListSpec, ListState, PageCounts, PageVariant, Pagination, SectionHeading,
    StatusKind, TableEmpty, NO_FLASH,
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

pub(crate) struct Pages {
    progression: Arc<Progression>,
    skills: Arc<Skills>,
}
impl Pages {
    pub(crate) fn new(progression: Arc<Progression>, skills: Arc<Skills>) -> Self {
        Self {
            progression,
            skills,
        }
    }
    fn state(&self, operation: &str, query: &HashMap<String, String>) -> Result<State> {
        State::new(
            self.progression.clone(),
            self.skills.clone(),
            operation == "my",
            query,
        )
    }
}
impl Page<Scope, Error> for Pages {
    fn title(&self, operation: &str) -> Option<&'static str> {
        Some(if operation == "my" {
            "My standing"
        } else {
            "Progression policies"
        })
    }
    fn render<'a>(&'a self, request: PageRequest<'a, Scope>) -> PageFuture<'a, Error> {
        Box::pin(async move {
            let mut state = self.state(request.operation, request.query)?;
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
            let state: Box<dyn LiveState<Scope, Error>> =
                Box::new(self.state(request.operation, request.query)?);
            Ok(Some(state))
        })
    }
}
fn spec() -> Result<Arc<ListSpec>> {
    ListSpec {
        page_sizes: vec![10, 25, 50, 100],
        default_page_size: 25,
        page_size_aliases: vec!["perPage".into()],
        invalid_page_size: bilibi_ui::list_state::InvalidPageSize::Default,
        omit_blank: vec!["search".into()],
        ..ListSpec::default()
    }
    .shared()
    .map_err(|_| Error::Internal)
}
struct State {
    progression: Arc<Progression>,
    skills: Arc<Skills>,
    my: bool,
    list: ListState,
    tables: BTreeMap<String, ListColumns>,
    rows: Vec<Policy>,
    total: u64,
    profiles: Vec<Profile>,
    selected: Option<Policy>,
    pending: Option<Policy>,
    values: HashMap<String, String>,
    can_manage: bool,
    can_progression: bool,
    can_performance: bool,
    standing: Option<Standing>,
    unavailable: Option<String>,
    flash: Option<(StatusKind, String)>,
}
impl State {
    fn new(
        progression: Arc<Progression>,
        skills: Arc<Skills>,
        my: bool,
        query: &HashMap<String, String>,
    ) -> Result<Self> {
        let mut state = Self {
            progression,
            skills,
            my,
            list: ListState::parse(query, spec()?).map_err(|_| Error::Internal)?,
            tables: BTreeMap::new(),
            rows: vec![],
            total: 0,
            profiles: vec![],
            selected: None,
            pending: None,
            values: HashMap::from([
                ("performance".into(), "false".into()),
                ("missing_evidence".into(), "unknown".into()),
            ]),
            can_manage: false,
            can_progression: false,
            can_performance: false,
            standing: None,
            unavailable: None,
            flash: None,
        };
        state.ensure_table(
            "progression-policies",
            &[
                ("policy", "Policy"),
                ("version", "Version"),
                ("effective", "Effective from"),
                ("status", "Status"),
                ("criteria", "Criteria"),
            ],
        )?;
        Ok(state)
    }
    fn ensure_table(&mut self, id: &str, columns: &[(&str, &str)]) -> Result<()> {
        if !self.tables.contains_key(id) {
            self.tables.insert(
                id.into(),
                ListColumns::mount(
                    id,
                    columns
                        .iter()
                        .map(|(id, label)| BuiltinColumn::new(id, label))
                        .collect(),
                )
                .map_err(|_| Error::Internal)?,
            );
        }
        Ok(())
    }
    async fn load(&mut self, scope: &Scope) -> Result<()> {
        self.unavailable = None;
        self.standing = None;
        self.rows.clear();
        self.total = 0;
        self.profiles.clear();
        let Some(company) = scope.actor().organisation().map(|id| id.get()) else {
            self.unavailable = Some(ProgressionError::EmployeeUnavailable.to_string());
            self.can_manage = false;
            self.can_progression = false;
            self.can_performance = false;
            self.selected = None;
            self.pending = None;
            return Ok(());
        };
        if self.my {
            self.can_progression = self.progression.allowed(scope, company, SELF_VIEW).await;
            self.can_performance = self
                .progression
                .allowed(scope, company, bilibi_people_performance::SELF_VIEW)
                .await;
            if self.can_progression {
                match self.progression.explain(scope, company).await {
                    Ok(standing) => {
                        for e in &standing.explanations {
                            self.ensure_table(
                                &format!("progression-explanation-{}", e.policy.id),
                                &[
                                    ("criterion", "Criterion"),
                                    ("result", "Result"),
                                    ("evidence", "Evidence"),
                                ],
                            )?;
                        }
                        self.standing = Some(standing);
                    }
                    Err(error) => self.unavailable = Some(message(&error)),
                }
            }
        } else {
            self.can_manage = self
                .progression
                .allowed(scope, company, POLICY_MANAGE)
                .await;
            match self
                .progression
                .policies(
                    scope,
                    company,
                    PageOptions {
                        page: self.list.page as i64,
                        page_size: self.list.page_size as i64,
                    },
                )
                .await
            {
                Ok(page) => {
                    self.rows = page.rows;
                    self.total = page.total as u64;
                    self.list.page = page.page as u64;
                    if let Some(selected) = &mut self.selected {
                        if let Some(row) = self.rows.iter().find(|r| r.id == selected.id) {
                            *selected = row.clone();
                        }
                    }
                }
                Err(error) => {
                    self.unavailable = Some(message(&error));
                    self.selected = None;
                    self.pending = None;
                }
            }
            if self.can_manage && self.unavailable.is_none() {
                // Skills retains its own read permission. A policy with only
                // performance criteria is still possible without catalog access.
                self.profiles = self
                    .skills
                    .list_profiles(scope, company)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|p| p.status == "published")
                    .collect();
            } else {
                self.pending = None;
            }
            if let Some(selected) = &self.selected {
                self.ensure_table(
                    &format!("policy-criteria-{}", selected.id),
                    &[
                        ("skill", "Skill criterion"),
                        ("level", "Required level"),
                        ("profile", "Profile version"),
                    ],
                )?;
            }
        }
        Ok(())
    }
    fn fail(&mut self, error: ProgressionError) {
        self.flash = Some((StatusKind::Error, message(&error)));
    }
    async fn event(&mut self, scope: &Scope, payload: &Value) -> Result<()> {
        let event = payload.get("type").and_then(Value::as_str).unwrap_or("");
        if let Some(table) = event.strip_prefix("grid-") {
            if let Some(columns) = self.tables.get_mut(table) {
                columns.handle(&params::string_values(payload));
            }
            return Ok(());
        }
        if matches!(event, "draft" | "request_publish" | "confirm_publish") {
            let permitted = if let Some(company) = scope.actor().organisation() {
                !self.my
                    && self.unavailable.is_none()
                    && self
                        .progression
                        .allowed(scope, company.get(), POLICY_MANAGE)
                        .await
                    && self
                        .progression
                        .allowed(scope, company.get(), POLICY_VIEW)
                        .await
            } else {
                false
            };
            if !permitted {
                self.pending = None;
                self.fail(ProgressionError::Unauthorized);
                return Ok(());
            }
        }
        match event {
            "clear-flash" | "clear_error" => self.flash = None,
            "filter" | "filters" => {
                self.pending = None;
                self.list
                    .apply_filters(&params::form_values(payload, "filters"));
            }
            "paginate" | "page" => {
                self.pending = None;
                self.list
                    .put_page(payload.get("page").and_then(Value::as_str));
            }
            "select" if !self.my => {
                let id = event_id(payload);
                self.selected = self.rows.iter().find(|p| Some(p.id) == id).cloned();
            }
            "cancel_publish" => self.pending = None,
            "request_publish" => {
                self.pending = self
                    .rows
                    .iter()
                    .find(|p| Some(p.id) == event_id(payload) && p.status == "draft")
                    .cloned();
            }
            "confirm_publish" => {
                // The client chooses neither company nor record at confirmation.
                if let Some(policy) = self.pending.take() {
                    let company = scope.actor().organisation().ok_or(Error::Invalid)?.get();
                    match self.progression.publish(scope, company, policy.id).await {
                        Ok(_) => {
                            self.flash = Some((StatusKind::Success, "Policy published.".into()))
                        }
                        Err(error) => self.fail(error),
                    }
                }
            }
            "draft" => {
                self.values = params::form_values(payload, "policy");
                let company = scope.actor().organisation().ok_or(Error::Invalid)?.get();
                match self.draft_input() {
                    Ok(input) => match self.progression.draft(scope, company, &input).await {
                        Ok(_) => {
                            self.values.clear();
                            self.values
                                .insert("missing_evidence".into(), "unknown".into());
                            self.flash =
                                Some((StatusKind::Success, "Policy draft recorded.".into()));
                        }
                        Err(error) => self.fail(error),
                    },
                    Err(error) => self.fail(error),
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn value(&self, key: &str) -> &str {
        self.values.get(key).map_or("", String::as_str)
    }
    fn draft_input(&self) -> crate::Result<DraftInput> {
        let profile = if self.value("profile_id").is_empty() {
            None
        } else {
            let id = params::positive_id(Some(self.value("profile_id")))
                .ok_or(ProgressionError::ProfileUnavailable)?;
            let profile = self
                .profiles
                .iter()
                .find(|p| p.id == id)
                .ok_or(ProgressionError::ProfileUnavailable)?;
            Some(ProfilePin {
                id: profile.id,
                version: profile.version,
            })
        };
        let performance = if self.value("performance") == "true" {
            Some(PerformanceRule {
                periods: vec![Period {
                    start: self.value("period_start").into(),
                    end: self.value("period_end").into(),
                }],
                accepted_outcomes: self
                    .value("outcomes")
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect(),
                missing_evidence: match self.value("missing_evidence") {
                    "unknown" => MissingEvidence::Unknown,
                    "not_met" => MissingEvidence::NotMet,
                    _ => return Err(ProgressionError::InvalidRules),
                },
            })
        } else {
            None
        };
        Ok(DraftInput {
            code: self.value("code").into(),
            name: self.value("name").into(),
            version: self
                .value("version")
                .parse()
                .map_err(|_| ProgressionError::InvalidPolicy)?,
            effective_from: self.value("effective_from").into(),
            profile,
            performance,
        })
    }
    fn table(
        &self,
        id: &str,
        rows: Vec<FlexRow>,
        empty: EmptyState,
        actions: bool,
    ) -> Result<Html> {
        let columns = self.tables.get(id).ok_or(Error::Internal)?;
        let event = format!("grid-{id}");
        let mut table = FlexTable::new(id, columns.flex_columns(), rows);
        table.event = &event;
        table.zoom = columns.zoom;
        table.suggestions = columns.flex_suggestions();
        table.add_query = &columns.add_query;
        table.actions = actions;
        table.empty = Some(TableEmpty::State(empty));
        render(&table)
    }
    fn policies(&self) -> Result<Html> {
        let rows = self
            .rows
            .iter()
            .map(|p| {
                let mut actions = action("select", "Read", p.id)?;
                if self.can_manage && p.status == "draft" {
                    actions.push(&action("request_publish", "Publish", p.id)?);
                }
                let mut row = FlexRow::new(format!("policy-{}", p.id))
                    .slot("policy", Html::text(&format!("{} · {}", p.name, p.code)))
                    .slot("version", Html::text(&p.version.to_string()))
                    .slot("effective", Html::text(&p.effective_from))
                    .slot("status", Html::text(&p.status))
                    .slot(
                        "criteria",
                        Html::text(&format!(
                            "{} skill requirements{}",
                            p.rules.competence.len(),
                            if p.rules.performance.is_some() {
                                "; performance required"
                            } else {
                                ""
                            }
                        )),
                    );
                row.actions = actions;
                Ok(row)
            })
            .collect::<Result<Vec<_>>>()?;
        let mut body = self.table(
            "progression-policies",
            rows,
            EmptyState::missing(
                "No progression policies",
                "An authorized policy operator must draft and publish company criteria.",
            )
            .id("progression-empty"),
            true,
        )?;
        let mut pagination = Pagination::new(
            "progression-pagination",
            PageCounts::new(self.list.page, self.list.page_size, self.total),
            &[10, 25, 50, 100],
        );
        pagination.page_event = "paginate";
        pagination.filters_event = "filter";
        body.push(&render(&pagination)?);
        if let Some(p) = &self.selected {
            body.push(&self.detail(p)?);
        }
        Ok(body)
    }
    fn detail(&self, policy: &Policy) -> Result<Html> {
        let publication = match policy.published_at.as_deref().and_then(|s| s.parse().ok()) {
            Some(instant) => {
                let r = bilibi_datetime::DisplayContext::anonymous().render(
                    instant,
                    bilibi_datetime::Format::DateTime,
                    bilibi_datetime::Precision::Second,
                );
                render(&Publication {
                    value: bilibi_ui::Instant::new(
                        &r.iso,
                        &r.text,
                        &r.company,
                        &r.utc,
                        r.mode.as_str(),
                    ),
                })?
            }
            None => Html::text("Draft"),
        };
        let mut body = facts(vec![
            (
                "Version",
                Html::text(&format!("{} · {}", policy.code, policy.version)),
            ),
            ("Effective from", Html::text(&policy.effective_from)),
            ("Publication", publication),
        ])?;
        let rows = policy
            .rules
            .competence
            .iter()
            .enumerate()
            .map(|(i, r)| {
                FlexRow::new(i.to_string())
                    .slot("skill", Html::text(&r.code))
                    .slot("level", Html::text(&r.required_level.to_string()))
                    .slot(
                        "profile",
                        Html::text(&format!("{} · {}", r.profile_id, r.profile_version)),
                    )
            })
            .collect();
        body.push(&self.table(
            &format!("policy-criteria-{}", policy.id),
            rows,
            EmptyState::missing(
                "No skill criteria",
                "This policy declares performance criteria only.",
            ),
            false,
        )?);
        if let Some(rule) = &policy.rules.performance {
            body.push(&facts(vec![
                (
                    "Performance periods",
                    Html::text(
                        &rule
                            .periods
                            .iter()
                            .map(|p| format!("{} – {}", p.start, p.end))
                            .collect::<Vec<_>>()
                            .join("; "),
                    ),
                ),
                (
                    "Accepted outcomes",
                    Html::text(&rule.accepted_outcomes.join(", ")),
                ),
                (
                    "Missing released evidence",
                    Html::text(match rule.missing_evidence {
                        MissingEvidence::Unknown => "unknown",
                        MissingEvidence::NotMet => "not_met",
                    }),
                ),
            ])?);
        }
        card("progression-detail", &policy.name, "", body)
    }
    fn draft_form(&self) -> Result<Html> {
        let field = |key: &str, label: &str, kind: InputKind| {
            Field::in_form("policy", key)
                .label(label)
                .kind(kind)
                .value(self.value(key))
        };
        let fields = vec![
            field("code", "Policy code", InputKind::Text).required(),
            field("name", "Policy name", InputKind::Text).required(),
            field("version", "Version", InputKind::Number)
                .required()
                .attr("min", "1"),
            field("effective_from", "Effective from", InputKind::Date).required(),
            field(
                "profile_id",
                "Published competency profile",
                InputKind::Select,
            )
            .prompt("No competency criteria")
            .options(
                self.profiles
                    .iter()
                    .map(|p| {
                        Choice::new(
                            format!("{} · version {}", p.name, p.version),
                            p.id.to_string(),
                        )
                    })
                    .collect(),
            ),
            field(
                "performance",
                "Require released performance evidence",
                InputKind::Checkbox,
            )
            .checked(self.value("performance") == "true"),
            field("period_start", "Performance period start", InputKind::Date),
            field("period_end", "Performance period end", InputKind::Date),
            field(
                "outcomes",
                "Accepted performance outcomes (separated by commas)",
                InputKind::Text,
            ),
            field(
                "missing_evidence",
                "Missing performance evidence",
                InputKind::Select,
            )
            .options(vec![
                Choice::new("Unknown", "unknown"),
                Choice::new("Not met", "not_met"),
            ]),
        ];
        let mut body = Html::EMPTY;
        for field in fields {
            body.push(&render(&field)?);
        }
        let form = render(&Form {
            id: "progression-draft",
            event: "draft",
            body,
            actions: render(&Button {
                button_type: "submit",
                variant: ButtonVariant::Primary,
                working: "Recording…",
                body: Html::text("Record draft"),
                ..Button::default()
            })?,
            ..Form::default()
        })?;
        card(
            "progression-draft-section",
            "Draft a policy version",
            "",
            form,
        )
    }
    fn my_standing(&self) -> Result<Html> {
        let mut body = Html::EMPTY;
        if self.can_performance {
            body.push(&card(
                "my-standing-performance",
                "My performance",
                "Communicated targets, released reviews and your responses.",
                render(&ActionLink {
                    href: "/people/performance/my",
                    body: Html::text("Open my performance"),
                    ..ActionLink::default()
                })?,
            )?);
        }
        if self.can_progression {
            if let Some(reason) = &self.unavailable {
                body.push(&render(
                    &EmptyState::missing("Your progression explanation is unavailable", reason)
                        .id("my-progression-unavailable"),
                )?);
            }
            if let Some(standing) = &self.standing {
                for e in &standing.explanations {
                    let mut content = facts(vec![
                        (
                            "Policy version",
                            Html::text(&format!("{} · {}", e.policy.code, e.policy.version)),
                        ),
                        ("Effective from", Html::text(&e.policy.effective_from)),
                        ("Eligibility criteria", Html::text(label(e.status))),
                    ])?;
                    let rows = e
                        .rules
                        .iter()
                        .enumerate()
                        .map(|(i, r)| {
                            let (code, evidence) = match r {
                                Criterion::Skill {
                                    code,
                                    required_level,
                                    observed_level,
                                    ..
                                } => (
                                    code,
                                    format!(
                                        "Required level {required_level} · {}",
                                        observed_level.map_or_else(
                                            || "current assessment unavailable".into(),
                                            |level| format!("observed {level}")
                                        )
                                    ),
                                ),
                                Criterion::Performance {
                                    code,
                                    review_id: Some(id),
                                    review_version,
                                    period_start,
                                    period_end,
                                    supersedes_id,
                                    ..
                                } => (
                                    code,
                                    format!(
                                        "Released review {id} · version {} · {} – {}{}",
                                        review_version.unwrap_or_default(),
                                        period_start.as_deref().unwrap_or(""),
                                        period_end.as_deref().unwrap_or(""),
                                        if supersedes_id.is_some() {
                                            " · corrected evidence requires reevaluation"
                                        } else {
                                            ""
                                        }
                                    ),
                                ),
                                Criterion::Performance { code, .. } => (
                                    code,
                                    "No released review in the policy's declared period.".into(),
                                ),
                            };
                            FlexRow::new(i.to_string())
                                .slot("criterion", Html::text(code))
                                .slot("result", Html::text(label(r.status())))
                                .slot("evidence", Html::text(&evidence))
                        })
                        .collect();
                    content.push(&self.table(
                        &format!("progression-explanation-{}", e.policy.id),
                        rows,
                        EmptyState::missing("No criteria", ""),
                        false,
                    )?);
                    body.push(&card(
                        &format!("my-progression-{}", e.policy.id),
                        &e.policy.name,
                        "",
                        content,
                    )?);
                }
            }
        }
        Ok(body)
    }
    fn flash(&self) -> FlashMap<'_> {
        let mut flash = NO_FLASH;
        if let Some((kind, text)) = &self.flash {
            match kind {
                StatusKind::Success => flash.success = text,
                _ => flash.error = text,
            }
        }
        flash
    }
    fn render(&self) -> Result<String> {
        let mut body = if self.my {
            self.my_standing()?
        } else {
            let mut body = if let Some(reason) = &self.unavailable {
                render(
                    &EmptyState::missing("Policies unavailable", reason)
                        .id("progression-unavailable"),
                )?
            } else {
                self.policies()?
            };
            if !self.can_manage {
                body.push(&render(
                    &EmptyState::missing(
                        "Policy history is read-only",
                        "A policy operator with manage permission can create and publish versions.",
                    )
                    .id("progression-read-only"),
                )?);
            } else if self.unavailable.is_none() {
                body.push(&self.draft_form()?);
            }
            body
        };
        if self.my && !self.can_progression && !self.can_performance {
            body.push(&render(&EmptyState::missing("Your standing is unavailable", "You cannot do that for this company. Check your progression and evidence permissions."))?);
        }
        let confirmation = if let Some(policy) = &self.pending {
            render(&ConfirmDialog {
                id: "progression-confirm", consequence: "Confirm publication",
                detail: &format!("Publish {} version {}, effective {}? This version and its criteria become permanent. The latest effective publication will apply to employees.", policy.name, policy.version, policy.effective_from),
                confirm: "Confirm publication", working: "Publishing…", confirm_event: "confirm_publish", cancel_event: "cancel_publish",
            })?
        } else {
            Html::EMPTY
        };
        View {
            id: if self.my { "my-standing" } else { "progression-page" },
            title: if self.my { "My standing" } else { "Progression policies" },
            subtitle: if self.my { "" } else { "Published criteria explain eligibility. They do not authorize a promotion or change pay." },
            variant: if self.my { PageVariant::Detail } else { PageVariant::List },
            flash: self.flash(), body, confirmation,
        }.render().map_err(|_| Error::Internal)
    }
}
impl LiveState<Scope, Error> for State {
    fn update<'a>(&'a mut self, request: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            if matches!(request.input, LiveInput::Event(_)) {
                self.load(request.scope).await?;
            }
            match request.input {
                LiveInput::Params(query) => {
                    self.pending = None;
                    self.list = ListState::parse(query, spec()?).map_err(|_| Error::Internal)?;
                }
                LiveInput::Event(payload) => self.event(request.scope, payload).await?,
                LiveInput::Refresh => {}
            }
            self.load(request.scope).await?;
            Ok(LiveBody {
                html: self.render()?,
                query: if self.my {
                    HashMap::new()
                } else {
                    self.list.to_params()
                },
                rows: self.rows.len(),
                ..LiveBody::default()
            })
        })
    }
}
#[derive(Template)]
#[template(path = "page.html")]
struct View<'a> {
    id: &'a str,
    title: &'a str,
    subtitle: &'a str,
    variant: PageVariant,
    flash: FlashMap<'a>,
    body: Html,
    confirmation: Html,
}
#[derive(Template)]
#[template(
    source = "{% import \"flex_table.html\" as table %}{% call table::instant(id=\"progression-publication-time\", i=value) %}{% endcall %}",
    ext = "html"
)]
struct Publication {
    value: bilibi_ui::Instant,
}

fn render(template: &impl Template) -> Result<Html> {
    Html::render(template).map_err(|_| Error::Internal)
}
fn action(event: &str, label: &str, id: i64) -> Result<Html> {
    render(&Button {
        button_type: "button",
        event,
        name: "id",
        value: &id.to_string(),
        body: Html::text(label),
        ..Button::default()
    })
}
fn card(id: &str, title: &str, description: &str, mut body: Html) -> Result<Html> {
    let mut content = render(&SectionHeading {
        title,
        description,
        ..SectionHeading::default()
    })?;
    content.push(&body);
    body = content;
    render(&Card {
        id,
        body,
        ..Card::default()
    })
}
fn facts(items: Vec<(&str, Html)>) -> Result<Html> {
    render(&Facts {
        id: "",
        items: items
            .into_iter()
            .map(|(title, value)| Fact {
                title,
                value,
                ..Fact::default()
            })
            .collect(),
    })
}
fn event_id(payload: &Value) -> Option<i64> {
    params::positive_id(payload.get("id").and_then(Value::as_str))
}
fn label(status: Status) -> &'static str {
    match status {
        Status::Met => "Met",
        Status::NotMet => "Not met",
        Status::Unknown => "Unknown",
    }
}
fn message(error: &ProgressionError) -> String {
    match error {
        ProgressionError::Failed(_) => {
            "The record is unavailable. Check your company, employee link and evidence permissions."
                .into()
        }
        other => other.to_string(),
    }
}
