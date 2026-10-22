//! Bilimbi Progression.Web.PolicyLive and MyLive. Every business write goes
//! through Progression; view state stays in the shared list components.
use crate::{
    Criterion, DraftInput, MissingEvidence, PageOptions, PerformanceRule, Period, Policy,
    ProfilePin, Progression, ProgressionError, Standing, Status, POLICY_MANAGE, POLICY_VIEW,
    SELF_VIEW,
};
mod render;
use bilibi_database::{Error, Result};
use bilibi_people_skills::{Profile, Skills};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::{
        LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState, MountFuture, Page, PageBody,
        PageFuture, PageRequest,
    },
    params, ActionLink, BuiltinColumn, Button, ButtonVariant, Card, Choice, ConfirmDialog,
    EmptyState, ErrorSummary, Fact, Facts, Field, FlashMap, FlexRow, FlexTable, Form, Html,
    InputKind, ListColumns, ListSpec, ListState, PageCounts, PageVariant, Pagination,
    SectionHeading, StatusKind, TableEmpty, NO_FLASH,
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

pub(crate) struct Pages {
    display: Arc<bilibi_datetime::DisplayPolicy>,
    progression: Arc<Progression>,
    skills: Arc<Skills>,
}
impl Pages {
    pub(crate) fn new(
        progression: Arc<Progression>,
        skills: Arc<Skills>,
        display: Arc<bilibi_datetime::DisplayPolicy>,
    ) -> Self {
        Self {
            progression,
            skills,
            display,
        }
    }
    fn state(&self, operation: &str, query: &HashMap<String, String>) -> Result<State> {
        State::new(
            self.progression.clone(),
            self.skills.clone(),
            self.display.clone(),
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
    display: Arc<bilibi_datetime::DisplayPolicy>,
    clock: bilibi_datetime::DisplayContext,
    progression: Arc<Progression>,
    skills: Arc<Skills>,
    my: bool,
    company: Option<i64>,
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
    draft_error: Option<String>,
}
impl State {
    fn new(
        progression: Arc<Progression>,
        skills: Arc<Skills>,
        display: Arc<bilibi_datetime::DisplayPolicy>,
        my: bool,
        query: &HashMap<String, String>,
    ) -> Result<Self> {
        let mut state = Self {
            display,
            clock: bilibi_datetime::DisplayContext::anonymous(),
            progression,
            skills,
            my,
            company: None,
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
            draft_error: None,
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
        // Keep an old page bound to its original company. An account relink
        // must not expose the former company's selected policy or retarget a form.
        if self.company.is_some_and(|bound| bound != company) {
            self.unavailable = Some(ProgressionError::Unauthorized.to_string());
            self.can_manage = false;
            self.can_progression = false;
            self.can_performance = false;
            self.selected = None;
            self.pending = None;
            return Ok(());
        }
        self.company = Some(company);
        let client = self.progression.db.get().await?;
        self.clock = self
            .display
            .display(&**client, scope, scope.actor().organisation(), "en")
            .await?;
        drop(client);
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
            "clear-flash" | "clear_error" => {
                self.flash = None;
                self.draft_error = None;
            }
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
                self.draft_error = None;
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
                        Err(error) => {
                            self.draft_error = Some(message(&error));
                            self.fail(error);
                        }
                    },
                    Err(error) => {
                        self.draft_error = Some(message(&error));
                        self.fail(error);
                    }
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

fn event_id(payload: &Value) -> Option<i64> {
    params::positive_id(payload.get("id").and_then(Value::as_str))
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
