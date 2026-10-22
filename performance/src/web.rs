//! Bilimbi Performance.Web.{ReviewsLive, MyLive}: task tabs, immutable history
//! and bounded owner-provided choices. Every commit consumes server-held input.
use crate::*;
use askama::Template;
use bilibi_database::{Error, Result as UiResult};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::{
        LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState, MountFuture, Page as UiPage,
        PageBody, PageFuture, PageRequest,
    },
    params, BuiltinColumn, Button, ButtonVariant, Card, Choice, ConfirmDialog, EmptyState, Fact,
    Facts, Field, Filter, FlashMap, FlexRow, FlexTable, Fold, Form, Html, InputKind, ListColumns,
    ListSpec, ListState, PageCounts, Pagination, SectionHeading, StatusKind, Tab, TableEmpty, Tabs,
    NO_FLASH,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};

pub(crate) struct Pages(Arc<Performance>, Arc<bilibi_datetime::DisplayPolicy>);
impl Pages {
    pub(crate) fn new(
        performance: Arc<Performance>,
        display: Arc<bilibi_datetime::DisplayPolicy>,
    ) -> Self {
        Self(performance, display)
    }
}
impl UiPage<Scope, Error> for Pages {
    fn title(&self, operation: &str) -> Option<&'static str> {
        Some(if operation == "my" {
            "My performance"
        } else {
            "Performance reviews"
        })
    }
    fn render<'a>(&'a self, request: PageRequest<'a, Scope>) -> PageFuture<'a, Error> {
        Box::pin(async move {
            let mut state = State::new(
                self.0.clone(),
                self.1.clone(),
                request.operation == "my",
                request.query,
            )?;
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
                self.1.clone(),
                request.operation == "my",
                request.query,
            )?) as Box<dyn LiveState<Scope, Error>>))
        })
    }
}

// One vocabulary supplies detail labels and optional scalar columns.
bilibi_ui::record_fields! { mod review_fields {
    PERIOD: bilibi_ui::RecordField::new("period", "Period"),
    OUTCOME: bilibi_ui::RecordField::new("outcome", "Outcome"),
    RATIONALE: bilibi_ui::RecordField::new("rationale", "Rationale").excluded("Long review rationale stays in the detail."),
    CUTOFF: bilibi_ui::RecordField::new("cutoff", "Cutoff"),
    CORRECTION: bilibi_ui::RecordField::new("correction", "Correction").excluded("Long correction reasons stay in the detail."),
    RELEASED: bilibi_ui::RecordField::new("released", "Released"),
    EVIDENCE: bilibi_ui::RecordField::new("evidence", "Evidence").excluded("Pinned evidence is a nested collection."),
    TARGET: bilibi_ui::RecordField::new("target", "Target").excluded("Pinned targets are a nested collection."),
    RESPONSE: bilibi_ui::RecordField::new("response", "Response").excluded("Responses are a nested collection."),
} }

const TABLES: [(&str, &[(&str, &str)]); 7] = [
    (
        "performance-reviews",
        &[
            ("employee", "Employee"),
            ("period", "Period"),
            ("version", "Version"),
            ("state", "State"),
            ("outcome", "Outcome"),
        ],
    ),
    (
        "performance-release-queue",
        &[("employee", "Employee"), ("rationale", "Rationale")],
    ),
    (
        "performance-descriptions",
        &[
            ("code", "Code"),
            ("version", "Version"),
            ("purpose", "Purpose"),
            ("effective", "Effective"),
            ("state", "State"),
        ],
    ),
    (
        "performance-definitions",
        &[
            ("code", "Code"),
            ("version", "Version"),
            ("name", "Name"),
            ("measure", "Measure"),
        ],
    ),
    (
        "performance-targets",
        &[
            ("employee", "Employee"),
            ("target", "Target"),
            ("version", "Version"),
            ("state", "State"),
        ],
    ),
    (
        "performance-observations",
        &[
            ("source", "Source"),
            ("employee", "Employee"),
            ("evidence", "Evidence"),
            ("version", "Source version"),
        ],
    ),
    (
        "my-performance-targets",
        &[
            ("target", "Target"),
            ("period", "Period"),
            ("effective", "Effective from"),
            ("version", "Version"),
        ],
    ),
];
const PREFIXES: [&str; 7] = [
    "",
    "release_",
    "descriptions_",
    "definitions_",
    "targets_",
    "evidence_",
    "my_targets_",
];
struct List {
    state: ListState,
    columns: ListColumns,
}
impl List {
    fn new(index: usize, query: &HashMap<String, String>) -> UiResult<Self> {
        let spec = ListSpec {
            page_sizes: vec![10, 25, 50, 100],
            default_page_size: 25,
            param_prefix: PREFIXES[index].into(),
            invalid_page_size: bilibi_ui::list_state::InvalidPageSize::Default,
            filters: if index == 0 {
                BTreeMap::from([(
                    "tab".into(),
                    Filter::one_of(&["reviews", "descriptions", "kpis", "evidence"], "reviews"),
                )])
            } else {
                BTreeMap::new()
            },
            omit_blank: vec!["search".into()],
            ..ListSpec::default()
        }
        .shared()
        .map_err(|_| Error::Internal)?;
        Ok(Self {
            state: ListState::parse(query, spec).map_err(|_| Error::Internal)?,
            columns: ListColumns::mount(
                TABLES[index].0,
                TABLES[index]
                    .1
                    .iter()
                    .map(|(key, label)| BuiltinColumn::new(key, label))
                    .collect(),
            )
            .map(|columns| {
                if index <= 1 {
                    columns.with_record_fields(review_fields::ALL.iter().copied())
                } else {
                    columns
                }
            })
            .map_err(|_| Error::Internal)?,
        })
    }
}
#[derive(Clone)]
enum Command {
    Description(DescriptionInput),
    Definition(DefinitionInput),
    Target(TargetInput),
    AmendTarget(i64, TargetAmendment),
    Observation(ObservationInput),
    CorrectObservation(i64, ObservationCorrection),
    Review(ReviewInput),
    CorrectReview(i64, ReviewCorrection),
    ReviewTarget(i64, String),
    PublishDescription(i64),
    PublishTarget(i64),
    Release(i64),
    Respond(i64, String),
}
impl Command {
    fn capability(&self) -> &'static str {
        match self {
            Self::Description(_) | Self::PublishDescription(_) => DESCRIPTIONS_MANAGE,
            Self::Definition(_) | Self::Target(_) | Self::AmendTarget(..) => KPIS_SUBMIT,
            Self::ReviewTarget(..) => KPIS_REVIEW,
            Self::PublishTarget(_) => KPIS_APPROVE,
            Self::Observation(_)
            | Self::CorrectObservation(..)
            | Self::Review(_)
            | Self::CorrectReview(..) => REVIEWS_SUBMIT,
            Self::Release(_) => REVIEWS_APPROVE,
            Self::Respond(..) => SELF_VIEW,
        }
    }
    async fn commit(self, service: &Performance, scope: &Scope, company: i64) -> Result<()> {
        match self {
            Self::Description(input) => service
                .draft_description(scope, company, &input)
                .await
                .map(|_| ()),
            Self::Definition(input) => service.define_kpi(scope, company, &input).await.map(|_| ()),
            Self::Target(input) => service
                .propose_target(scope, company, &input)
                .await
                .map(|_| ()),
            Self::AmendTarget(id, input) => service
                .amend_target(scope, company, id, &input)
                .await
                .map(|_| ()),
            Self::Observation(input) => service
                .record_observation(scope, company, &input)
                .await
                .map(|_| ()),
            Self::CorrectObservation(id, input) => service
                .correct_observation(scope, company, id, &input)
                .await
                .map(|_| ()),
            Self::Review(input) => service
                .draft_review(scope, company, &input)
                .await
                .map(|_| ()),
            Self::CorrectReview(id, input) => service
                .correct_review(scope, company, id, &input)
                .await
                .map(|_| ()),
            Self::ReviewTarget(id, note) => service
                .review_target(scope, company, id, &note)
                .await
                .map(|_| ()),
            Self::PublishDescription(id) => service
                .publish_description(scope, company, id)
                .await
                .map(|_| ()),
            Self::PublishTarget(id) => service.publish_target(scope, company, id).await.map(|_| ()),
            Self::Release(id) => service.release_review(scope, company, id).await.map(|_| ()),
            Self::Respond(id, text) => service.respond(scope, company, id, &text).await.map(|_| ()),
        }
    }
}
struct Pending {
    command: Command,
    company: i64,
    detail: String,
}
struct State {
    service: Arc<Performance>,
    display_policy: Arc<bilibi_datetime::DisplayPolicy>,
    display: bilibi_datetime::DisplayContext,
    my: bool,
    company: i64,
    lists: Vec<List>,
    permissions: HashMap<&'static str, bool>,
    reviews: Vec<Review>,
    total: u64,
    planning: Option<PlanningRecords>,
    choices: Option<PlanningChoices>,
    mine: Option<MyRecords>,
    selected: Option<ReviewDetail>,
    pending: Option<Pending>,
    unavailable: Option<String>,
    flash: Option<(StatusKind, String)>,
    drafts: HashMap<String, Map<String, Value>>,
    errors: Vec<FieldError>,
    active_form: String,
}
impl State {
    fn new(
        service: Arc<Performance>,
        display_policy: Arc<bilibi_datetime::DisplayPolicy>,
        my: bool,
        query: &HashMap<String, String>,
    ) -> UiResult<Self> {
        Ok(Self {
            service,
            display_policy,
            display: bilibi_datetime::DisplayContext::anonymous(),
            my,
            company: 0,
            lists: (0..TABLES.len())
                .map(|i| List::new(i, query))
                .collect::<UiResult<_>>()?,
            permissions: HashMap::new(),
            reviews: vec![],
            total: 0,
            planning: None,
            choices: None,
            mine: None,
            selected: None,
            pending: None,
            unavailable: None,
            flash: None,
            drafts: HashMap::new(),
            errors: vec![],
            active_form: String::new(),
        })
    }
    fn tab(&self) -> &str {
        &self.lists[0].state.filters["tab"]
    }
    fn can(&self, capability: &str) -> bool {
        self.permissions.get(capability).copied().unwrap_or(false)
    }
    fn message(&mut self, kind: StatusKind, text: impl Into<String>) {
        self.flash = Some((kind, text.into()));
    }
    fn flash(&self) -> FlashMap<'_> {
        let mut map = NO_FLASH;
        if let Some((kind, text)) = &self.flash {
            if *kind == StatusKind::Success {
                map.success = text;
            } else {
                map.error = text;
            }
        }
        map
    }
    fn options(&self) -> PageOptions {
        PageOptions {
            page: self.lists[0].state.page as i64,
            page_size: self.lists[0].state.page_size as i64,
        }
    }
    async fn load(&mut self, scope: &Scope) -> UiResult<()> {
        // The current company belongs to the authenticated account; forms never choose it.
        self.company = match scope.actor().organisation() {
            Some(id) => id.get(),
            None => {
                let client = self.service.db.get().await?;
                bilibi_people_workforce::authorization::selectable_companies(
                    &**client,
                    scope,
                    if self.my { SELF_VIEW } else { VIEW },
                )
                .await
                .unwrap_or_default()
                .first()
                .map_or(0, |c| c.id)
            }
        };
        if self.company == 0 {
            self.unavailable = Some(PerformanceError::Unavailable.to_string());
            self.pending = None;
            self.selected = None;
            self.planning = None;
            self.choices = None;
            self.mine = None;
            self.reviews.clear();
            self.total = 0;
            return Ok(());
        }
        let client = self.service.db.get().await?;
        self.display = match self
            .display_policy
            .display(
                &**client,
                scope,
                Some(
                    bilibi_party_ids::OrganisationId::new(self.company)
                        .map_err(|_| Error::Invalid)?,
                ),
                self.display.locale,
            )
            .await
        {
            Ok(display) => display,
            Err(Error::Forbidden | Error::Unauthorized) => {
                self.unavailable = Some(PerformanceError::Unauthorized.to_string());
                self.pending = None;
                self.selected = None;
                self.planning = None;
                self.choices = None;
                self.mine = None;
                self.reviews.clear();
                self.total = 0;
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        drop(client);
        self.permissions.clear();
        for capability in CAPABILITIES {
            self.permissions.insert(
                capability,
                self.service.allowed(scope, self.company, capability).await,
            );
        }
        let result = if self.my {
            self.service
                .my_records(scope, self.company, self.options())
                .await
                .map(|records| {
                    self.total = records.reviews.total as u64;
                    self.lists[0].state.page = records.reviews.page as u64;
                    self.mine = Some(records);
                })
        } else {
            async {
                let page = self
                    .service
                    .reviews(scope, self.company, self.options())
                    .await?;
                let planning = self.service.planning_records(scope, self.company).await?;
                let choices = self.service.planning_choices(scope, self.company).await?;
                self.total = page.total as u64;
                self.lists[0].state.page = page.page as u64;
                self.reviews = page.rows;
                self.planning = Some(planning);
                self.choices = Some(choices);
                Ok(())
            }
            .await
        };
        match result {
            Ok(()) => self.unavailable = None,
            Err(error) => {
                self.unavailable = Some(error.to_string());
                self.reviews.clear();
                self.planning = None;
                self.choices = None;
                self.mine = None;
                self.total = 0;
                self.selected = None;
                self.pending = None;
            }
        }
        if let Some(p) = &self.planning {
            for (index, total) in [
                (1, p.drafts.len()),
                (2, p.descriptions.len()),
                (3, p.definitions.len()),
                (4, p.targets.len()),
                (5, p.observations.len()),
            ] {
                let list = &mut self.lists[index].state;
                list.page = list
                    .page
                    .min((total as u64).div_ceil(list.page_size).max(1));
            }
        }
        if let Some(m) = &self.mine {
            let list = &mut self.lists[6].state;
            list.page = list
                .page
                .min((m.targets.len() as u64).div_ceil(list.page_size).max(1));
        }
        if let Some(id) = self.selected.as_ref().map(|r| r.review.id) {
            self.selected = self.service.review(scope, self.company, id).await.ok();
        }
        Ok(())
    }
    fn query(&self) -> HashMap<String, String> {
        let mut query = HashMap::new();
        for (index, list) in self.lists.iter().enumerate() {
            if index == 0 || (!self.my && index < 6) || (self.my && index == 6) {
                query.extend(list.state.to_params());
            }
        }
        if self.my {
            query.remove("tab");
        }
        query
    }
    async fn permitted(&mut self, scope: &Scope, capability: &str) -> bool {
        let page_cap = if self.my { SELF_VIEW } else { VIEW };
        if !self.service.allowed(scope, self.company, page_cap).await
            || !self.service.allowed(scope, self.company, capability).await
        {
            self.pending = None;
            self.message(
                StatusKind::Error,
                PerformanceError::Unauthorized.to_string(),
            );
            false
        } else {
            true
        }
    }
    async fn event(&mut self, scope: &Scope, payload: &Value) -> UiResult<()> {
        let event = payload.get("type").and_then(Value::as_str).unwrap_or("");
        if event == "clear-flash" {
            self.flash = None;
            return Ok(());
        }
        if event == "cancel_publication" {
            self.pending = None;
            return Ok(());
        }
        // All write-shaped intents reauthorize before inspecting a posted selection.
        if event == "confirm_publication" {
            if let Some(pending) = self.pending.take() {
                if pending.company == self.company
                    && self.permitted(scope, pending.command.capability()).await
                {
                    let response = matches!(pending.command, Command::Respond(..));
                    match pending
                        .command
                        .commit(&self.service, scope, pending.company)
                        .await
                    {
                        Ok(()) => {
                            self.drafts.remove(&self.active_form);
                            self.errors.clear();
                            self.message(
                                StatusKind::Success,
                                if response {
                                    "Response recorded."
                                } else {
                                    "Record saved."
                                },
                            );
                        }
                        Err(error) => self.refused(error),
                    }
                }
            } else {
                self.message(StatusKind::Error, PerformanceError::NotFound.to_string());
            }
            return Ok(());
        }
        if let Some(cap) = event_capability(event) {
            if self.my != (event == "save_response") {
                self.message(StatusKind::Error, PerformanceError::NotFound.to_string());
                return Ok(());
            }
            if !self.permitted(scope, cap).await {
                return Ok(());
            }
            if self.pending.is_some() {
                return Ok(());
            }
            let result = self.prepare(event, payload);
            match result {
                Ok(command) => {
                    if let Command::Respond(id, text) = command {
                        match self.service.respond(scope, self.company, id, &text).await {
                            Ok(_) => {
                                self.drafts.remove(&self.active_form);
                                self.message(StatusKind::Success, "Response recorded.");
                            }
                            Err(error) => self.refused(error),
                        }
                        return Ok(());
                    }
                    let publication = matches!(
                        command,
                        Command::PublishDescription(_)
                            | Command::PublishTarget(_)
                            | Command::Release(_)
                    );
                    let detail = if publication {
                        "Corrections require a new version; existing history is retained.".into()
                    } else {
                        self.confirmation_detail()
                    };
                    self.pending = Some(Pending {
                        command,
                        company: self.company,
                        detail,
                    });
                }
                Err(error) => self.refused(error),
            }
            return Ok(());
        }
        if event == "select" {
            if let Some(id) = event_id(payload) {
                match self.service.review(scope, self.company, id).await {
                    Ok(record) => self.selected = Some(record),
                    Err(error) => self.refused(error),
                }
            }
            return Ok(());
        }
        for index in 0..self.lists.len() {
            if event == format!("grid_{index}") {
                self.lists[index]
                    .columns
                    .handle(&params::string_values(payload));
                return Ok(());
            }
            if event == format!("page_{index}") || (index == 0 && event == "paginate") {
                self.pending = None;
                self.selected = None;
                self.lists[index]
                    .state
                    .put_page(payload.get("page").and_then(Value::as_str));
                return Ok(());
            }
            if event == format!("filter_{index}") || (index == 0 && event == "filter") {
                self.pending = None;
                self.selected = None;
                self.lists[index]
                    .state
                    .apply_filters(&params::form_values(payload, "filters"));
                return Ok(());
            }
        }
        Ok(())
    }
    fn refused(&mut self, error: PerformanceError) {
        self.errors = if let PerformanceError::Invalid(errors) = &error {
            errors.clone()
        } else {
            vec![]
        };
        self.message(StatusKind::Error, error.to_string());
    }
    fn confirmation_detail(&self) -> String {
        self.drafts
            .get(&self.active_form)
            .map(|fields| {
                fields
                    .iter()
                    .filter(|(_, value)| !value.is_null())
                    .map(|(key, value)| {
                        let text = match value {
                            Value::String(text) => text.clone(),
                            value => value.to_string(),
                        };
                        format!("{}: {}", label(key), text)
                    })
                    .collect::<Vec<_>>()
                    .join(" · ")
            })
            .unwrap_or_default()
    }
    fn prepare(&mut self, event: &str, payload: &Value) -> Result<Command> {
        let id = event_id(payload);
        if matches!(
            event,
            "request_publish_description" | "request_publish_target" | "request_release"
        ) {
            let id = id.ok_or(PerformanceError::NotFound)?;
            let planning = self.planning.as_ref().ok_or(PerformanceError::NotFound)?;
            self.active_form.clear();
            return match event {
                "request_publish_description"
                    if planning
                        .descriptions
                        .iter()
                        .any(|r| r.id == id && r.status == "draft") =>
                {
                    Ok(Command::PublishDescription(id))
                }
                "request_publish_target"
                    if planning
                        .targets
                        .iter()
                        .any(|r| r.id == id && r.status == "reviewed" && !r.confidential) =>
                {
                    Ok(Command::PublishTarget(id))
                }
                "request_release" if planning.drafts.iter().any(|r| r.id == id) => {
                    Ok(Command::Release(id))
                }
                _ => Err(PerformanceError::NotFound),
            };
        }
        if event == "review_target"
            && !self.planning.as_ref().is_some_and(|p| {
                p.targets
                    .iter()
                    .any(|r| Some(r.id) == id && r.status == "proposed")
            })
        {
            return Err(PerformanceError::NotFound);
        }
        if event == "save_response"
            && !self.mine.as_ref().is_some_and(|m| {
                m.reviews
                    .rows
                    .iter()
                    .any(|r| Some(r.review.id) == id && r.responses.is_empty())
            })
        {
            return Err(PerformanceError::NotFound);
        }
        let form_id = match event {
            "create_description" => "description-form".into(),
            "create_definition" => "definition-form".into(),
            "create_target" => "target-form".into(),
            "create_observation" => "observation-form".into(),
            "create_review" => "review-form".into(),
            "review_target" => format!("target-review-{}", id.ok_or(PerformanceError::NotFound)?),
            "save_response" => format!("response-form-{}", id.ok_or(PerformanceError::NotFound)?),
            _ => return Err(PerformanceError::NotFound),
        };
        self.active_form = form_id.clone();
        self.errors.clear();
        // Performance narratives accept 8,000 characters. The generic form
        // helper keeps only 1 KiB, so read this fixed allowlist explicitly.
        let fields = fields_for(event);
        let mut values = Map::new();
        for key in fields {
            let name = format!("record[{key}]");
            let value = payload
                .get(&name)
                .or_else(|| payload.get(format!("{name}[]")));
            if matches!(*key, "observation_ids" | "target_ids") {
                let selected = match value {
                    None => vec![],
                    Some(Value::Array(items)) if items.len() <= 100 => items
                        .iter()
                        .map(|v| {
                            v.as_str()
                                .and_then(|s| s.parse::<i64>().ok())
                                .filter(|id| *id > 0)
                                .ok_or(PerformanceError::InvalidEvidence)
                        })
                        .collect::<Result<Vec<_>>>()?,
                    Some(Value::String(text)) if text.is_empty() => vec![],
                    _ => return Err(PerformanceError::InvalidEvidence),
                };
                values.insert((*key).into(), json!(selected));
            } else {
                let text = value.and_then(Value::as_str).unwrap_or("");
                if text.chars().count() > 8_000 {
                    return Err(PerformanceError::Invalid(vec![FieldError::new(
                        key,
                        "is too long",
                    )]));
                }
                values.insert((*key).into(), Value::String(text.into()));
            }
        }
        self.drafts.insert(form_id, values.clone());
        let prior = values
            .get("supersedes_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(|s| s.parse::<i64>().map_err(|_| PerformanceError::NotFound))
            .transpose()?;
        for key in [
            "version",
            "position_id",
            "position_version",
            "definition_id",
            "employee_id",
            "description_id",
            "precision",
        ] {
            if let Some(raw) = values.get(key).and_then(Value::as_str) {
                let number = if raw.is_empty() {
                    0
                } else {
                    raw.parse::<i64>().map_err(|_| {
                        PerformanceError::Invalid(vec![FieldError::new(key, "must be a number")])
                    })?
                };
                values.insert(key.into(), json!(number));
            }
        }
        if let Some(value) = values.get("confidential").and_then(Value::as_str) {
            values.insert("confidential".into(), json!(matches!(value, "true" | "on")));
        }
        for key in ["effective_to", "rubric"] {
            if values.get(key).and_then(Value::as_str) == Some("") {
                values.insert(key.into(), Value::Null);
            }
        }
        if let Some(cutoff) = values.get("cutoff_at").and_then(Value::as_str) {
            if cutoff.is_empty() && prior.is_some() {
                values.insert("cutoff_at".into(), Value::Null);
            } else {
                let cutoff = if cutoff.ends_with('Z') || cutoff.ends_with("+00:00") {
                    cutoff.to_owned()
                } else if cutoff.len() == 16 {
                    format!("{cutoff}:00Z")
                } else {
                    format!("{cutoff}Z")
                };
                values.insert("cutoff_at".into(), json!(cutoff));
            }
        }
        let text = |key: &str| {
            values
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned()
        };
        match event {
            "create_description" => {
                let profile = text("profile_id")
                    .parse::<i64>()
                    .ok()
                    .and_then(|id| self.choices.as_ref()?.profiles.iter().find(|p| p.id == id));
                values.insert(
                    "competency_links".into(),
                    profile.map_or(json!([]), |p| json!([{"id":p.id,"version":p.version}])),
                );
                Ok(Command::Description(decode(values)?))
            }
            "create_definition" => Ok(Command::Definition(decode(values)?)),
            "create_target" => {
                if let Some(id) = prior {
                    Ok(Command::AmendTarget(id, decode(values)?))
                } else {
                    Ok(Command::Target(decode(values)?))
                }
            }
            "create_observation" => {
                if let Some(id) = prior {
                    Ok(Command::CorrectObservation(id, decode(values)?))
                } else {
                    Ok(Command::Observation(decode(values)?))
                }
            }
            "create_review" => {
                if let Some(id) = prior {
                    Ok(Command::CorrectReview(id, decode(values)?))
                } else {
                    Ok(Command::Review(decode(values)?))
                }
            }
            "review_target" => {
                let id = id.ok_or(PerformanceError::NotFound)?;
                if !self.planning.as_ref().is_some_and(|p| {
                    p.targets
                        .iter()
                        .any(|r| r.id == id && r.status == "proposed")
                }) {
                    return Err(PerformanceError::NotFound);
                }
                Ok(Command::ReviewTarget(id, text("note")))
            }
            "save_response" => {
                let id = id.ok_or(PerformanceError::NotFound)?;
                if !self.mine.as_ref().is_some_and(|m| {
                    m.reviews
                        .rows
                        .iter()
                        .any(|r| r.review.id == id && r.responses.is_empty())
                }) {
                    return Err(PerformanceError::NotFound);
                }
                Ok(Command::Respond(id, text("response")))
            }
            _ => Err(PerformanceError::NotFound),
        }
    }
    fn employee(&self, id: i64) -> String {
        self.choices
            .as_ref()
            .and_then(|c| c.names.get(&id))
            .cloned()
            .unwrap_or_else(|| "Employee record unavailable".into())
    }
    fn table(
        &self,
        index: usize,
        mut rows: Vec<FlexRow>,
        title: &str,
        reason: &str,
        server_page: bool,
    ) -> UiResult<Html> {
        let host = &self.lists[index];
        let total = if server_page {
            self.total
        } else {
            rows.len() as u64
        };
        let page = host
            .state
            .page
            .min(total.div_ceil(host.state.page_size).max(1));
        if !server_page {
            rows = rows
                .into_iter()
                .skip(((page - 1) * host.state.page_size) as usize)
                .take(host.state.page_size as usize)
                .collect();
        }
        let mut table = FlexTable::new(TABLES[index].0, host.columns.flex_columns(), rows);
        let event = format!("grid_{index}");
        table.event = &event;
        table.zoom = host.columns.zoom;
        table.suggestions = host.columns.flex_suggestions();
        table.add_query = &host.columns.add_query;
        table.actions = matches!(index, 0 | 1 | 2 | 4);
        table.empty = Some(TableEmpty::State(EmptyState::missing(title, reason)));
        let mut body = render(&table)?;
        let page_event = format!("page_{index}");
        let filter_event = format!("filter_{index}");
        let pagination_id = format!("{}-pagination", TABLES[index].0);
        let mut pagination = Pagination::new(
            &pagination_id,
            PageCounts::new(page, host.state.page_size, total),
            &[10, 25, 50, 100],
        );
        pagination.page_event = &page_event;
        pagination.filters_event = &filter_event;
        body.push(&render(&pagination)?);
        Ok(body)
    }
    fn review_slots(&self, row: FlexRow, record: &Review, list: &str) -> UiResult<FlexRow> {
        Ok(row
            .slot(
                "period",
                Html::text(&format!("{} – {}", record.period_start, record.period_end)),
            )
            .slot("outcome", Html::text(&record.outcome))
            .slot(
                "cutoff",
                instant(
                    &self.display,
                    &format!("{list}-cutoff-{}", record.id),
                    &record.cutoff_at,
                )?,
            )
            .slot(
                "released",
                if let Some(at) = &record.released_at {
                    instant(&self.display, &format!("{list}-released-{}", record.id), at)?
                } else {
                    Html::text("Pending independent release")
                },
            ))
    }
    fn reviews_table(&self) -> UiResult<Html> {
        let rows = self
            .reviews
            .iter()
            .map(|r| {
                let mut row = FlexRow::new(format!("review-{}", r.id))
                    .slot("employee", Html::text(&self.employee(r.employee_id)))
                    .slot(
                        "period",
                        Html::text(&format!("{} – {}", r.period_start, r.period_end)),
                    )
                    .slot("version", Html::text(&r.version.to_string()))
                    .slot("state", Html::text(&r.status))
                    .slot("outcome", Html::text(&r.outcome));
                row = self.review_slots(row, r, "authored")?;
                row.actions = action("select", "Read", r.id)?;
                Ok(row)
            })
            .collect::<UiResult<Vec<_>>>()?;
        self.table(
            0,
            rows,
            "No reviews authored",
            "Record evidence and communicated targets before drafting a review.",
            true,
        )
    }
    fn planning_table(&self, index: usize) -> UiResult<Html> {
        let Some(p) = &self.planning else {
            return Ok(Html::EMPTY);
        };
        let (rows, title, reason) = match index {
            1 => (
                p.drafts
                    .iter()
                    .map(|r| {
                        let mut row = FlexRow::new(format!("release-{}", r.id))
                            .slot("employee", Html::text(&self.employee(r.employee_id)))
                            .slot("rationale", Html::text(&r.rationale));
                        row = self.review_slots(row, r, "release")?;
                        row.actions = action("select", "Read", r.id)?;
                        row.actions
                            .push(&action("request_release", "Release", r.id)?);
                        Ok(row)
                    })
                    .collect::<UiResult<Vec<_>>>()?,
                "No drafts awaiting release",
                "",
            ),
            2 => (
                p.descriptions
                    .iter()
                    .map(|r| {
                        let mut row = FlexRow::new(format!("description-{}", r.id))
                            .slot("code", Html::text(&r.code))
                            .slot("version", Html::text(&r.version.to_string()))
                            .slot("purpose", Html::text(&r.purpose))
                            .slot(
                                "effective",
                                Html::text(&format!(
                                    "{} – {}",
                                    r.effective_from,
                                    r.effective_to.as_deref().unwrap_or("Open")
                                )),
                            )
                            .slot("state", Html::text(&r.status));
                        if self.can(DESCRIPTIONS_MANAGE) && r.status == "draft" {
                            row.actions = action("request_publish_description", "Publish", r.id)?;
                        }
                        Ok(row)
                    })
                    .collect::<UiResult<Vec<_>>>()?,
                "No position descriptions",
                "Create a description linked to a position and published competency profile.",
            ),
            3 => (
                p.definitions
                    .iter()
                    .map(|r| {
                        Ok(FlexRow::new(format!("definition-{}", r.id))
                            .slot("code", Html::text(&r.code))
                            .slot("version", Html::text(&r.version.to_string()))
                            .slot("name", Html::text(&r.name))
                            .slot(
                                "measure",
                                Html::text(&format!("{} ({})", r.measure, r.unit)),
                            ))
                    })
                    .collect::<UiResult<Vec<_>>>()?,
                "No KPI definitions",
                "",
            ),
            4 => (
                p.targets
                    .iter()
                    .map(|r| {
                        let mut row = FlexRow::new(format!("target-{}", r.id))
                            .slot("employee", Html::text(&self.employee(r.employee_id)))
                            .slot("target", Html::text(&r.target))
                            .slot("version", Html::text(&r.version.to_string()))
                            .slot(
                                "state",
                                Html::text(&format!(
                                    "{}{}",
                                    r.status,
                                    if r.confidential {
                                        " · confidential"
                                    } else {
                                        ""
                                    }
                                )),
                            );
                        if self.can(KPIS_REVIEW) && r.status == "proposed" {
                            row.actions = self.editor(
                                &format!("target-review-{}", r.id),
                                "Review",
                                "review_target",
                                &["note"],
                                Some(r.id),
                                false,
                            )?;
                        }
                        if self.can(KPIS_APPROVE) && r.status == "reviewed" && !r.confidential {
                            row.actions.push(&action(
                                "request_publish_target",
                                "Communicate",
                                r.id,
                            )?);
                        }
                        Ok(row)
                    })
                    .collect::<UiResult<Vec<_>>>()?,
                "No targets available",
                "",
            ),
            5 => (
                p.observations
                    .iter()
                    .map(|r| {
                        Ok(FlexRow::new(format!("observation-{}", r.id))
                            .slot("source", Html::text(&r.source_reference))
                            .slot("employee", Html::text(&self.employee(r.employee_id)))
                            .slot("evidence", Html::text(&r.evidence))
                            .slot("version", Html::text(&r.source_version)))
                    })
                    .collect::<UiResult<Vec<_>>>()?,
                "No observations recorded",
                "",
            ),
            _ => return Ok(Html::EMPTY),
        };
        self.table(index, rows, title, reason, false)
    }
    fn choices_for(&self, key: &str, form: &str) -> Option<Vec<Choice>> {
        let p = self.planning.as_ref()?;
        let c = self.choices.as_ref()?;
        let options = match key {
            "employee_id" => c
                .employees
                .iter()
                .map(|r| Choice::new(&r.name, r.id.to_string()))
                .collect(),
            "position_id" => c
                .positions
                .iter()
                .map(|r| {
                    Choice::new(
                        format!("{} · version {}", r.name, r.version.unwrap_or(0)),
                        r.id.to_string(),
                    )
                })
                .collect(),
            "profile_id" => c
                .profiles
                .iter()
                .map(|r| {
                    Choice::new(
                        format!("{} · version {}", r.name, r.version),
                        r.id.to_string(),
                    )
                })
                .collect(),
            "definition_id" => p
                .definitions
                .iter()
                .map(|r| {
                    Choice::new(
                        format!("{} · version {}", r.name, r.version),
                        r.id.to_string(),
                    )
                })
                .collect(),
            "description_id" => c
                .descriptions
                .iter()
                .map(|r| {
                    Choice::new(
                        format!("{} · version {}", r.code, r.version),
                        r.id.to_string(),
                    )
                })
                .collect(),
            "observation_ids" => p
                .observations
                .iter()
                .map(|r| {
                    Choice::new(
                        format!(
                            "{} · {}: {}",
                            r.source_reference, r.source_version, r.evidence
                        ),
                        r.id.to_string(),
                    )
                })
                .collect(),
            "target_ids" => c
                .targets
                .iter()
                .map(|r| {
                    Choice::new(
                        format!("{} · version {}", r.target, r.version),
                        r.id.to_string(),
                    )
                })
                .collect(),
            "direction" => ["higher", "lower", "band", "rubric"]
                .iter()
                .map(|s| Choice::new(capitalize(s), *s))
                .collect(),
            "supersedes_id" => match form {
                "review-form" => p
                    .prior_reviews
                    .iter()
                    .map(|r| {
                        Choice::new(
                            format!("{} · {} · version {}", r.period_start, r.outcome, r.version),
                            r.id.to_string(),
                        )
                    })
                    .collect(),
                "target-form" => c
                    .targets
                    .iter()
                    .map(|r| {
                        Choice::new(
                            format!("{} · version {}", r.target, r.version),
                            r.id.to_string(),
                        )
                    })
                    .collect(),
                _ => p
                    .observations
                    .iter()
                    .map(|r| {
                        Choice::new(
                            format!("{} · {}", r.source_reference, r.source_version),
                            r.id.to_string(),
                        )
                    })
                    .collect(),
            },
            _ => return None,
        };
        Some(options)
    }
    fn editor(
        &self,
        id: &str,
        title: &str,
        event: &str,
        keys: &[&str],
        record: Option<i64>,
        fold: bool,
    ) -> UiResult<Html> {
        let mut body = Html::EMPTY;
        if self.active_form == id && !self.errors.is_empty() {
            let mut summary = bilibi_ui::ErrorSummary::new("performance-form-errors");
            for error in &self.errors {
                summary.push(
                    format!("{id}-{}", error.field),
                    bilibi_ui::forms::error_sentence(&label(&error.field), &error.message),
                );
            }
            body.push(&render(&summary)?);
        }
        if let Some(id) = record {
            body.push(&render(
                &Field::named("record_id")
                    .kind(InputKind::Hidden)
                    .value(id.to_string()),
            )?);
        }
        for key in keys {
            let value = self.drafts.get(id).and_then(|d| d.get(*key));
            let mut field = Field::in_form("record", key)
                .id(format!("{id}-{key}"))
                .label(label(key))
                .value(value.and_then(Value::as_str).unwrap_or(""))
                .errors(if self.active_form == id {
                    self.errors
                        .iter()
                        .filter(|e| e.field == *key)
                        .map(|e| e.message.clone())
                        .collect()
                } else {
                    vec![]
                });
            if let Some(options) = self.choices_for(key, id) {
                field = field.kind(InputKind::Select).options(options);
                if matches!(*key, "observation_ids" | "target_ids") {
                    field.name.push_str("[]");
                    field = field.multiple().values(
                        value
                            .and_then(Value::as_array)
                            .map(|a| a.iter().map(Value::to_string).collect())
                            .unwrap_or_default(),
                    );
                } else {
                    field = field.prompt(match *key {
                        "employee_id" => "Choose a direct report",
                        "position_id" => "Choose a position",
                        "profile_id" => "Choose a published profile",
                        "definition_id" => "Choose a measurement",
                        "description_id" => "Choose a published description",
                        "direction" => "Choose an interpretation",
                        _ => "New record",
                    });
                }
            } else {
                field = field.kind(match *key {
                    "confidential" => InputKind::Checkbox,
                    "cutoff_at" => InputKind::DatetimeLocal,
                    "period_start" | "period_end" | "effective_from" | "effective_to"
                    | "window_start" | "window_end" => InputKind::Date,
                    "purpose" | "responsibilities" | "duties" | "authority" | "qualifications"
                    | "evidence" | "rationale" | "interpretation" | "measure" | "response" => {
                        InputKind::Textarea
                    }
                    "version" | "position_version" | "precision" => InputKind::Number,
                    _ => InputKind::Text,
                });
                if *key == "confidential" {
                    field = field.checked(value.and_then(Value::as_str) == Some("true"));
                }
            }
            if matches!(*key, "note" | "response") {
                field = field.required();
            }
            body.push(&render(&field)?);
        }
        let body = render(&Form {
            id,
            event,
            change: false,
            class: "",
            body,
            actions: render(&Button {
                button_type: "submit",
                working: if event == "save_response" {
                    "Recording…"
                } else {
                    "Saving…"
                },
                variant: ButtonVariant::Primary,
                body: Html::text(if event == "save_response" {
                    "Record response"
                } else if event == "review_target" {
                    "Review"
                } else {
                    "Save"
                }),
                ..Button::default()
            })?,
        })?;
        if fold {
            let body = render(&bilibi_ui::Page {
                variant: bilibi_ui::PageVariant::Form,
                body,
                ..bilibi_ui::Page::default()
            })?;
            render(&Fold {
                id: &format!("{id}-fold"),
                title,
                open: self.active_form == id && !self.errors.is_empty(),
                body,
                ..Fold::default()
            })
        } else {
            Ok(body)
        }
    }
    fn detail(&self, detail: &ReviewDetail, mine: bool) -> UiResult<Html> {
        let r = &detail.review;
        let mut facts = vec![
            Fact {
                title: review_fields::get(review_fields::PERIOD.id).label,
                value: Html::text(&format!("{} – {}", r.period_start, r.period_end)),
                ..Fact::default()
            },
            Fact {
                title: review_fields::get(review_fields::OUTCOME.id).label,
                value: Html::text(&r.outcome),
                ..Fact::default()
            },
            Fact {
                title: review_fields::get(review_fields::RATIONALE.id).label,
                value: Html::text(&r.rationale),
                ..Fact::default()
            },
            Fact {
                title: review_fields::get(review_fields::CUTOFF.id).label,
                value: instant(
                    &self.display,
                    &format!("review-cutoff-{}", r.id),
                    &r.cutoff_at,
                )?,
                ..Fact::default()
            },
            Fact {
                title: review_fields::get(review_fields::CORRECTION.id).label,
                value: Html::text(r.change_reason.as_deref().unwrap_or("Original version")),
                ..Fact::default()
            },
            Fact {
                title: review_fields::get(review_fields::RELEASED.id).label,
                value: if let Some(at) = &r.released_at {
                    instant(&self.display, &format!("review-released-{}", r.id), at)?
                } else {
                    Html::text("Pending independent release")
                },
                ..Fact::default()
            },
        ];
        for evidence in &detail.observations {
            facts.push(Fact {
                title: review_fields::get(review_fields::EVIDENCE.id).label,
                value: Html::text(&format!(
                    "{} · {} · {}",
                    evidence.evidence, evidence.source_reference, evidence.source_version
                )),
                ..Fact::default()
            });
        }
        for target in &detail.targets {
            facts.push(Fact {
                title: review_fields::get(review_fields::TARGET.id).label,
                value: Html::text(&format!(
                    "Target version {}: {}",
                    target.version, target.target
                )),
                ..Fact::default()
            });
        }
        for response in &detail.responses {
            facts.push(Fact {
                title: review_fields::get(review_fields::RESPONSE.id).label,
                value: Html::text(&response.response),
                ..Fact::default()
            });
        }
        let mut body = render(&SectionHeading {
            title: &format!("Review version {}", r.version),
            ..SectionHeading::default()
        })?;
        body.push(&render(&Facts {
            id: &format!("review-facts-{}", r.id),
            items: facts,
        })?);
        if mine && self.can(SELF_VIEW) && detail.responses.is_empty() {
            body.push(&self.editor(
                &format!("response-form-{}", r.id),
                "",
                "save_response",
                &["response"],
                Some(r.id),
                false,
            )?);
        }
        render(&Card {
            id: &if mine {
                format!("my-review-{}", r.id)
            } else {
                "performance-detail".into()
            },
            body,
            ..Card::default()
        })
    }
    fn content(&self) -> UiResult<Html> {
        if let Some(reason) = &self.unavailable {
            return render(
                &EmptyState::missing(
                    if self.my {
                        "Your performance records are unavailable"
                    } else {
                        "Performance is unavailable"
                    },
                    reason,
                )
                .id(if self.my {
                    "my-performance-unavailable"
                } else {
                    "performance-unavailable"
                }),
            );
        }
        if self.my {
            let Some(m) = &self.mine else {
                return Ok(Html::EMPTY);
            };
            let mut body = heading("Communicated targets")?;
            let rows = m
                .targets
                .iter()
                .map(|r| {
                    FlexRow::new(format!("my-target-{}", r.id))
                        .slot("target", Html::text(&r.target))
                        .slot(
                            "period",
                            Html::text(&format!("{} – {}", r.period_start, r.period_end)),
                        )
                        .slot("effective", Html::text(&r.effective_from))
                        .slot("version", Html::text(&r.version.to_string()))
                })
                .collect();
            body.push(&self.table(
                6,
                rows,
                "No targets communicated",
                "Your reviewer must approve and communicate targets before they appear here.",
                false,
            )?);
            if m.reviews.rows.is_empty() {
                body.push(&render(
                    &EmptyState::missing(
                        "No reviews released",
                        "Your review appears after independent release.",
                    )
                    .id("my-performance-empty"),
                )?);
            }
            for record in &m.reviews.rows {
                body.push(&self.detail(record, true)?);
            }
            let mut page = Pagination::new(
                "my-performance-pagination",
                PageCounts::new(
                    self.lists[0].state.page,
                    self.lists[0].state.page_size,
                    self.total,
                ),
                &[10, 25, 50, 100],
            );
            page.page_event = "page_0";
            page.filters_event = "filter_0";
            body.push(&render(&page)?);
            return Ok(body);
        }
        let mut body = Html::EMPTY;
        match self.tab() {
            "descriptions" => {
                body.push(&heading("Position description versions")?);
                body.push(&self.planning_table(2)?);
                if self.can(DESCRIPTIONS_MANAGE) {
                    body.push(&self.editor(
                        "description-form",
                        "New position description version",
                        "create_description",
                        fields_for("create_description"),
                        None,
                        true,
                    )?);
                } else {
                    body.push(&render(&EmptyState::missing(
                        "Description management unavailable",
                        "An operator must grant description management access.",
                    ))?);
                }
            }
            "kpis" => {
                body.push(&heading("KPI definition versions")?);
                body.push(&self.planning_table(3)?);
                if self.can(KPIS_SUBMIT) {
                    body.push(&self.editor(
                        "definition-form",
                        "New measurement version",
                        "create_definition",
                        fields_for("create_definition"),
                        None,
                        true,
                    )?);
                }
                body.push(&heading("Individual targets")?);
                body.push(&self.planning_table(4)?);
                if self.can(KPIS_SUBMIT) {
                    body.push(&self.editor(
                        "target-form",
                        "Propose a target or amendment",
                        "create_target",
                        fields_for("create_target"),
                        None,
                        true,
                    )?);
                } else {
                    body.push(&render(&EmptyState::missing(
                        "Target proposals are read-only",
                        "An operator must grant KPI submission access.",
                    ))?);
                }
            }
            "evidence" => {
                body.push(&heading("Evidence you recorded")?);
                body.push(&self.planning_table(5)?);
                if self.can(REVIEWS_SUBMIT) {
                    body.push(&self.editor(
                        "observation-form",
                        "Record evidence or a correction",
                        "create_observation",
                        fields_for("create_observation"),
                        None,
                        true,
                    )?);
                } else {
                    body.push(&render(&EmptyState::missing(
                        "Evidence recording is read-only",
                        "An operator must grant review submission access.",
                    ))?);
                }
            }
            _ => {
                body.push(&heading("Reviews you authored")?);
                body.push(&self.reviews_table()?);
                if let Some(selected) = &self.selected {
                    body.push(&self.detail(selected, false)?);
                }
                if self.can(REVIEWS_APPROVE) {
                    body.push(&heading("Independent release queue")?);
                    body.push(&self.planning_table(1)?);
                }
                if self.can(REVIEWS_SUBMIT) {
                    body.push(&self.editor(
                        "review-form",
                        "Draft a review or correction",
                        "create_review",
                        fields_for("create_review"),
                        None,
                        true,
                    )?);
                } else {
                    body.push(&render(&EmptyState::missing("Review drafting is read-only","An operator must grant review submission access to draft reviews for your direct reports."))?);
                }
            }
        }
        Ok(body)
    }
    fn render(&self) -> UiResult<String> {
        let tabs = if self.my {
            Html::EMPTY
        } else {
            let entries = [
                ("reviews", "Reviews"),
                ("descriptions", "Position descriptions"),
                ("kpis", "KPIs"),
                ("evidence", "Evidence"),
            ];
            let addresses: Vec<_> = entries
                .iter()
                .map(|(tab, _)| {
                    let mut query = self.query();
                    query.insert("tab".into(), (*tab).into());
                    query.insert("page".into(), "1".into());
                    params::address("/people/performance", &query)
                })
                .collect();
            render(&Tabs {
                id: "performance-tasks",
                label: "Performance tasks",
                tabs: entries
                    .iter()
                    .zip(addresses.iter())
                    .map(|((key, label), address)| Tab::patch(address, label, self.tab() == *key))
                    .collect(),
                ..Tabs::default()
            })?
        };
        let confirmation = if let Some(p) = &self.pending {
            render(&ConfirmDialog {
                id: "performance-publication",
                consequence: if matches!(
                    p.command,
                    Command::PublishDescription(_)
                        | Command::PublishTarget(_)
                        | Command::Release(_)
                ) {
                    "This version will become an immutable published record."
                } else {
                    "This record will be added to permanent history."
                },
                detail: &p.detail,
                confirm: if matches!(
                    p.command,
                    Command::PublishDescription(_)
                        | Command::PublishTarget(_)
                        | Command::Release(_)
                ) {
                    "Publish"
                } else {
                    "Save"
                },
                working: if matches!(
                    p.command,
                    Command::PublishDescription(_)
                        | Command::PublishTarget(_)
                        | Command::Release(_)
                ) {
                    "Publishing…"
                } else {
                    "Saving…"
                },
                confirm_event: "confirm_publication",
                cancel_event: "cancel_publication",
            })?
        } else {
            Html::EMPTY
        };
        View {
            title: if self.my {
                "My performance"
            } else {
                "Performance reviews"
            },
            variant: if self.my {
                bilibi_ui::PageVariant::Detail
            } else {
                bilibi_ui::PageVariant::List
            },
            flash: self.flash(),
            tabs,
            content: self.content()?,
            confirmation,
        }
        .render()
        .map_err(|_| Error::Internal)
    }
}
impl LiveState<Scope, Error> for State {
    fn update<'a>(&'a mut self, request: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            match request.input {
                LiveInput::Refresh => {}
                LiveInput::Params(query) => {
                    for index in 0..self.lists.len() {
                        let parsed = List::new(index, query)?;
                        self.lists[index].state = parsed.state;
                    }
                    self.pending = None;
                    self.selected = None;
                }
                LiveInput::Event(payload) => {
                    self.load(request.scope).await?;
                    self.event(request.scope, payload).await?;
                }
            }
            self.load(request.scope).await?;
            Ok(LiveBody {
                html: self.render()?,
                query: self.query(),
                rows: self.reviews.len(),
                ..LiveBody::default()
            })
        })
    }
}
#[derive(Template)]
#[template(path = "page.html")]
struct View<'a> {
    title: &'a str,
    variant: bilibi_ui::PageVariant,
    flash: FlashMap<'a>,
    tabs: Html,
    content: Html,
    confirmation: Html,
}
fn event_capability(event: &str) -> Option<&'static str> {
    match event {
        "create_description" | "request_publish_description" => Some(DESCRIPTIONS_MANAGE),
        "create_definition" | "create_target" => Some(KPIS_SUBMIT),
        "review_target" => Some(KPIS_REVIEW),
        "request_publish_target" => Some(KPIS_APPROVE),
        "create_observation" | "create_review" => Some(REVIEWS_SUBMIT),
        "request_release" => Some(REVIEWS_APPROVE),
        "save_response" => Some(SELF_VIEW),
        _ => None,
    }
}
fn event_id(payload: &Value) -> Option<i64> {
    params::positive_id(
        payload
            .get("value")
            .or_else(|| payload.get("record_id"))
            .or_else(|| payload.get("id"))
            .and_then(Value::as_str),
    )
}
fn fields_for(event: &str) -> &'static [&'static str] {
    match event {
        "create_description" => &[
            "code",
            "version",
            "position_id",
            "position_version",
            "effective_from",
            "effective_to",
            "purpose",
            "responsibilities",
            "duties",
            "authority",
            "qualifications",
            "profile_id",
        ],
        "create_definition" => &[
            "code",
            "version",
            "name",
            "purpose",
            "unit",
            "measure",
            "source_reference",
            "calculation_version",
            "direction",
            "rubric",
            "precision",
            "interpretation",
        ],
        "create_target" => &[
            "definition_id",
            "employee_id",
            "target",
            "period_start",
            "period_end",
            "effective_from",
            "confidential",
            "supersedes_id",
            "change_reason",
        ],
        "create_observation" => &[
            "employee_id",
            "window_start",
            "window_end",
            "evidence",
            "source_reference",
            "source_version",
            "supersedes_id",
            "change_reason",
        ],
        "create_review" => &[
            "employee_id",
            "description_id",
            "period_start",
            "period_end",
            "cutoff_at",
            "outcome",
            "rationale",
            "observation_ids",
            "target_ids",
            "supersedes_id",
            "change_reason",
        ],
        "review_target" => &["note"],
        "save_response" => &["response"],
        _ => &[],
    }
}
fn label(key: &str) -> String {
    match key {
        "cutoff_at" => "Evidence cutoff (UTC)",
        "observation_ids" => "Evidence versions",
        "target_ids" => "Communicated target versions",
        "supersedes_id" => "Prior version (corrections only)",
        "employee_id" => "Employee",
        "position_id" => "Position",
        "definition_id" => "KPI definition version",
        "description_id" => "Position description version",
        "profile_id" => "Published competency profile",
        "direction" => "Direction (higher, lower, band or rubric)",
        "note" => "Review rationale",
        "response" => "Your response or dispute",
        _ => return capitalize(&key.replace('_', " ")),
    }
    .into()
}
fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |c| {
        c.to_uppercase().to_string() + chars.as_str()
    })
}
fn decode<T: DeserializeOwned>(values: Map<String, Value>) -> Result<T> {
    serde_json::from_value(Value::Object(values)).map_err(|_| PerformanceError::Invalid(vec![]))
}
fn render(template: &impl Template) -> UiResult<Html> {
    Html::render(template).map_err(|_| Error::Internal)
}
fn heading(title: &str) -> UiResult<Html> {
    render(&SectionHeading {
        title,
        ..SectionHeading::default()
    })
}
fn action(event: &str, label: &str, id: i64) -> UiResult<Html> {
    render(&Button {
        button_type: "button",
        event,
        name: "value",
        value: &id.to_string(),
        body: Html::text(label),
        ..Button::default()
    })
}
fn instant(context: &bilibi_datetime::DisplayContext, id: &str, text: &str) -> UiResult<Html> {
    let timestamp = text
        .parse::<jiff::Timestamp>()
        .map_err(|_| Error::Internal)?;
    let r = context.render(
        timestamp,
        bilibi_datetime::Format::DateTime,
        bilibi_datetime::Precision::Minute,
    );
    render(&InstantView {
        id,
        instant: bilibi_ui::flex_table::Instant::new(
            &r.iso,
            &r.text,
            &r.company,
            &r.utc,
            r.mode.as_str(),
        ),
    })
}

#[derive(Template)]
#[template(
    source = r#"{% import "flex_table.html" as flex %}{% call flex::instant(id=id, i=instant) %}{% endcall %}"#,
    ext = "html"
)]
struct InstantView<'a> {
    id: &'a str,
    instant: bilibi_ui::flex_table::Instant,
}
