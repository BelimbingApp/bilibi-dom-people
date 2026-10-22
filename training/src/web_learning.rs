//! Bilimbi `Training.Web.LearningLive`: six governed learning tasks.
//! The mounted state owns the selected predecessor and pending decision.
use crate::*;
use askama::Template;
use bilibi_authz::{CapabilityDefinition, VocabularyContribution, VOCABULARY};
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
    params, BuiltinColumn, Button, ButtonVariant, Choice, ConfirmDialog, EmptyState, ErrorSummary,
    Field, Filter, FlashMap, FlexRow, FlexTable, Fold, Html, InputKind, ListColumns, ListSpec,
    ListState, Modal, Pagination, StatusKind, Tab, TableEmpty, Tabs, NO_FLASH,
};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

pub const LEARNING_VIEW: &str = "people.training.learning.view";
pub(crate) const PAGES: &str = "people/training.learning_pages";
const NO_WRITE: &str = "You cannot change this company's learning records.";
const PROVENANCE: Provenance = Provenance {
    module: MODULE,
    source: file!(),
    line: line!(),
};

inventory::submit! {
    Contribution::new(VOCABULARY, "people/training.learning_vocabulary", PROVENANCE, &VocabularyContribution {
        domains: &[], verbs: &[], capabilities: &[CapabilityDefinition::tenant(LEARNING_VIEW)], roles: &[], system_principals: &[],
    })
}
inventory::submit! {
    Contribution::new(PAGE_WRITES, PAGES, PROVENANCE, &PageWrites {
        service: PAGES, contracts: &[], pending: Pending::Every { reason: "Learning requests, plans and budgets call the Training facade; operation contracts are not yet reviewed." },
    })
}
macro_rules! route {
    ($key:literal, $path:literal, $cap:expr) => {
        inventory::submit! {
            Contribution::new(ROUTES, concat!("people/training.learning_", $key), PROVENANCE, &Route {
                method: Method::Get, path: $path, service: PAGES, operation: $key,
                access: Access::Capability($cap), framed: true,
            })
        }
    };
}
route!("my", "/people/training/my", LEARNING_VIEW);
route!("team", "/people/training/team", REQUESTS_RECOMMEND);
route!("plans", "/people/training/plans", PLANS_SUBMIT);
route!("requests", "/people/training/requests", REQUESTS_VIEW);
route!("plan-reviews", "/people/training/plan-reviews", PLANS_VIEW);
route!("budgets", "/people/training/budgets", BUDGETS_VIEW);
bilibi_menu::contribute!(
    "people/training",
    MenuItem::page(
        "people.my_work.my_learning",
        "My learning",
        "/people/training/my"
    )
    .icon("users")
    .under("people.my_work")
    .requires(Requirement::Capability(LEARNING_VIEW))
    .order(40)
);
bilibi_menu::contribute!(
    "people/training",
    MenuItem::page(
        "people.development.learning_reviews",
        "Learning requests & reviews",
        "/people/training/requests"
    )
    .icon("clipboard-document-list")
    .under("people.development")
    .requires(Requirement::Capability(REQUESTS_VIEW))
    .order(75)
);
bilibi_menu::contribute!(
    "people/training",
    MenuItem::page(
        "people.settings.learning_policy",
        "Learning policy and budgets",
        "/people/training/budgets"
    )
    .icon("cog-6-tooth")
    .under("people.settings")
    .requires(Requirement::Capability(BUDGETS_VIEW))
    .order(60)
);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Task {
    My,
    Team,
    Plans,
    Requests,
    PlanReviews,
    Budgets,
}
impl Task {
    const ALL: [Self; 6] = [
        Self::My,
        Self::Team,
        Self::Plans,
        Self::Requests,
        Self::PlanReviews,
        Self::Budgets,
    ];
    fn parse(operation: &str) -> DbResult<Self> {
        Self::ALL
            .into_iter()
            .find(|t| t.key() == operation)
            .ok_or(Error::NotFound)
    }
    fn key(self) -> &'static str {
        match self {
            Self::My => "my",
            Self::Team => "team",
            Self::Plans => "plans",
            Self::Requests => "requests",
            Self::PlanReviews => "plan-reviews",
            Self::Budgets => "budgets",
        }
    }
    fn title(self) -> &'static str {
        match self {
            Self::My => "My learning",
            Self::Team => "Team requests",
            Self::Plans => "Team plans",
            Self::Requests => "Learning requests & reviews",
            Self::PlanReviews => "Plan reviews",
            Self::Budgets => "Learning policy and budgets",
        }
    }
    fn cap(self) -> &'static str {
        match self {
            Self::My => LEARNING_VIEW,
            Self::Team => REQUESTS_RECOMMEND,
            Self::Plans => PLANS_SUBMIT,
            Self::Requests => REQUESTS_VIEW,
            Self::PlanReviews => PLANS_VIEW,
            Self::Budgets => BUDGETS_VIEW,
        }
    }
    fn kind(self) -> &'static str {
        match self {
            Self::My | Self::Team | Self::Requests => "request",
            Self::Plans | Self::PlanReviews => "plan",
            Self::Budgets => "budget",
        }
    }
    fn audience(self) -> LearningAudience {
        match self {
            Self::My => LearningAudience::SelfService,
            Self::Team | Self::Plans => LearningAudience::Team,
            _ => LearningAudience::Hr,
        }
    }
    fn create_cap(self) -> Option<&'static str> {
        match self {
            Self::My => Some(REQUESTS_SUBMIT),
            Self::Plans => Some(PLANS_SUBMIT),
            Self::Budgets => Some(BUDGETS_MANAGE),
            _ => None,
        }
    }
    fn path(self) -> String {
        format!("/people/training/{}", self.key())
    }
}

pub(crate) struct Pages(pub Arc<Training>);
impl Page<Scope, Error> for Pages {
    fn title(&self, operation: &str) -> Option<&'static str> {
        Task::parse(operation).ok().map(Task::title)
    }
    fn render<'a>(&'a self, req: PageRequest<'a, Scope>) -> PageFuture<'a, Error> {
        Box::pin(async move {
            let mut state = Learning::new(self.0.clone(), Task::parse(req.operation)?, req.query)?;
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
            Ok(Some(Box::new(Learning::new(
                self.0.clone(),
                Task::parse(req.operation)?,
                req.query,
            )?) as Box<dyn LiveState<Scope, Error>>))
        })
    }
}
fn spec() -> DbResult<Arc<ListSpec>> {
    ListSpec {
        page_size_param: "perPage".into(),
        filters: BTreeMap::from([("company_id".into(), Filter::string(""))]),
        omit_blank: vec!["company_id".into()],
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
enum Record {
    Request(LearningRequest),
    Plan(LearningPlan),
    Budget(LearningBudget),
}
impl Record {
    fn id(&self) -> i64 {
        match self {
            Self::Request(r) => r.id,
            Self::Plan(r) => r.id,
            Self::Budget(r) => r.policy.id,
        }
    }
    fn status(&self) -> &str {
        match self {
            Self::Request(r) => &r.status,
            Self::Plan(r) => &r.status,
            Self::Budget(_) => "",
        }
    }
}
struct Decision {
    id: i64,
    action: String,
    reason: String,
    capability: &'static str,
}
struct Learning {
    training: Arc<Training>,
    task: Task,
    list: ListState,
    columns: ListColumns,
    companies: Vec<bilibi_people_workforce::CompanySummary>,
    company: Option<i64>,
    records: Vec<Record>,
    histories: LearningHistories,
    nested_columns: BTreeMap<String, ListColumns>,
    flags: HashMap<&'static str, bool>,
    tabs: Vec<(String, String, bool)>,
    currencies: String,
    error: Option<String>,
    flash: Option<(StatusKind, String)>,
    modal: bool,
    prior_id: Option<i64>,
    item_count: usize,
    draft: HashMap<String, String>,
    summary: ErrorSummary,
    pending: Option<Decision>,
    banner: Html,
    instance_notice: String,
    clock: Option<bilibi_datetime::DisplayContext>,
}
impl Learning {
    fn new(training: Arc<Training>, task: Task, query: &HashMap<String, String>) -> DbResult<Self> {
        Ok(Self {
            training,
            task,
            list: ListState::parse(query, spec()?).map_err(|_| Error::Internal)?,
            columns: columns(
                "learning-records",
                &[
                    ("record", "Record"),
                    ("status", "Status / budget"),
                    ("decisions", "Decisions"),
                ],
            )?,
            companies: vec![],
            company: None,
            records: vec![],
            histories: BTreeMap::new(),
            nested_columns: BTreeMap::new(),
            flags: HashMap::new(),
            tabs: vec![],
            currencies: String::new(),
            error: None,
            flash: None,
            modal: false,
            prior_id: None,
            item_count: 1,
            draft: HashMap::new(),
            summary: ErrorSummary::new("learning-errors"),
            pending: None,
            banner: Html::EMPTY,
            instance_notice: String::new(),
            clock: None,
        })
    }
    fn can(&self, cap: &str) -> bool {
        self.flags.get(cap).copied().unwrap_or(false)
    }
    fn can_create(&self) -> bool {
        self.error.is_none() && self.task.create_cap().is_some_and(|c| self.can(c))
    }
    fn show_records(&self) -> bool {
        self.task != Task::My || self.can(REQUESTS_SUBMIT)
    }
    fn fail(&mut self, message: impl Into<String>) {
        self.flash = Some((StatusKind::Error, message.into()));
    }
    fn finish<T>(&mut self, result: crate::Result<T>, message: &str) -> bool {
        match result {
            Ok(_) => {
                self.flash = Some((StatusKind::Success, message.into()));
                self.modal = false;
                self.summary = ErrorSummary::new("learning-errors");
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
        self.companies = authorization::selectable_companies(&**client, scope, self.task.cap())
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|c| !c.archived)
            .collect();
        let selected = self
            .list
            .filters
            .get("company_id")
            .filter(|s| !s.is_empty())
            .cloned();
        self.company = if let Some(value) = selected.as_deref() {
            params::positive_id(Some(value)).filter(|id| self.companies.iter().any(|c| c.id == *id))
        } else {
            self.companies.first().map(|c| c.id)
        };
        self.records.clear();
        self.histories.clear();
        self.flags.clear();
        self.tabs.clear();
        self.error = None;
        let Some(company) = self.company else {
            self.modal = false;
            self.pending = None;
            self.error = Some("Choose an active company.".into());
            return Ok(());
        };
        // Bind the initial default once. Losing this company's reach must
        // refuse the old form, never retarget it to the next available company.
        self.list
            .filters
            .insert("company_id".into(), company.to_string());
        for cap in [
            LEARNING_VIEW,
            REQUESTS_SUBMIT,
            REQUESTS_RECOMMEND,
            REQUESTS_REVIEW,
            REQUESTS_APPROVE,
            PLANS_SUBMIT,
            PLANS_APPROVE,
            BUDGETS_MANAGE,
            REQUESTS_VIEW,
            PLANS_VIEW,
            BUDGETS_VIEW,
        ] {
            self.flags
                .insert(cap, self.training.allowed(scope, company, cap).await);
        }
        for task in Task::ALL {
            if self.can(task.cap()) {
                self.tabs.push((
                    task.title().into(),
                    params::address(
                        &task.path(),
                        &HashMap::from([("company_id".into(), company.to_string())]),
                    ),
                    task == self.task,
                ));
            }
        }
        // Passport and effectiveness slices add the corresponding destinations when installed.
        for (label, path, cap) in [
            (
                "My passport & evidence",
                "/people/training/records/my",
                "people.training.passport.my.view",
            ),
            (
                "Team passports",
                "/people/training/records/team",
                "people.training.passport.team.view",
            ),
            (
                "Evaluation policy & effectiveness",
                "/people/training/effectiveness",
                "people.training.effectiveness.view",
            ),
        ] {
            if self.training.allowed(scope, company, cap).await {
                self.tabs.push((
                    label.into(),
                    params::address(
                        path,
                        &HashMap::from([("company_id".into(), company.to_string())]),
                    ),
                    false,
                ));
            }
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
        let instance = bilibi_system::Instance::resolve(&self.training.settings, &**client).await?;
        self.instance_notice = instance.notice();
        self.banner = instance
            .banner("learning-instance", "")
            .map_err(|_| Error::Internal)?;
        drop(client);
        let result = match self.task.kind() {
            "request" if self.show_records() => self
                .training
                .learning_requests(scope, company, self.task.audience())
                .await
                .map(|rows| rows.into_iter().map(Record::Request).collect()),
            "request" => Ok(vec![]),
            "plan" => self
                .training
                .learning_plans(scope, company, self.task.audience())
                .await
                .map(|rows| rows.into_iter().map(Record::Plan).collect()),
            _ => self
                .training
                .learning_budgets(scope, company)
                .await
                .map(|rows| rows.into_iter().map(Record::Budget).collect()),
        };
        match result {
            Ok(records) => self.records = records,
            Err(e) => self.error = Some(e.to_string()),
        }
        let page = self.list.paginate(&self.records);
        let visible: Vec<_> = page.entries.iter().map(Record::id).collect();
        if self.task.kind() != "budget" && self.show_records() && self.error.is_none() {
            let kind = if self.task.kind() == "request" {
                LearningKind::Request
            } else {
                LearningKind::Plan
            };
            match self
                .training
                .learning_histories(scope, company, kind, &visible, self.task.audience())
                .await
            {
                Ok(h) => self.histories = h,
                Err(e) => {
                    self.error = Some(e.to_string());
                    self.records.clear();
                }
            }
        }
        if self.task == Task::Budgets {
            match self.training.learning_currencies(scope, company).await {
                Ok(c) => self.currencies = c.join(", "),
                Err(e) => self.error = Some(e.to_string()),
            }
        }
        for id in visible {
            let key = format!("history-{id}");
            if !self.nested_columns.contains_key(&key) {
                self.nested_columns.insert(
                    key.clone(),
                    columns(
                        &key,
                        &[
                            ("action", "Action"),
                            ("user", "User"),
                            ("impersonator", "Impersonator"),
                            ("reason", "Reason"),
                            ("time", "Time"),
                        ],
                    )?,
                );
            }
            if self.task.kind() == "plan" {
                let key = format!("items-{id}");
                if !self.nested_columns.contains_key(&key) {
                    self.nested_columns.insert(
                        key.clone(),
                        columns(
                            &key,
                            &[
                                ("need", "Need"),
                                ("result", "Expected result"),
                                ("cohort", "Target cohort"),
                                ("owner", "Responsible owner"),
                                ("timing", "Intended timing"),
                                ("evaluation", "Evaluation approach"),
                            ],
                        )?,
                    );
                }
            }
        }
        if self.error.is_some() {
            self.modal = false;
            self.pending = None;
        }
        Ok(())
    }
    fn actions(&self, r: &Record) -> Vec<(&'static str, &'static str, &'static str)> {
        let candidates = match (self.task, r.status()) {
            (Task::My, "draft") => vec![
                ("Submit", "submit", REQUESTS_SUBMIT),
                ("Cancel", "cancel", REQUESTS_SUBMIT),
            ],
            (Task::My, "pending_hod" | "pending_hr" | "pending_approval") => {
                vec![("Cancel", "cancel", REQUESTS_SUBMIT)]
            }
            (Task::Team, "pending_hod") => vec![
                ("Recommend", "recommend", REQUESTS_RECOMMEND),
                ("Reject", "reject", REQUESTS_RECOMMEND),
            ],
            (Task::Requests, "pending_hr") => vec![
                ("Review", "review", REQUESTS_REVIEW),
                ("Reject", "reject", REQUESTS_REVIEW),
            ],
            (Task::Requests, "pending_approval") => vec![
                ("Approve", "approve", REQUESTS_APPROVE),
                ("Reject", "reject", REQUESTS_APPROVE),
            ],
            (Task::Plans, "draft") => vec![
                ("Submit", "submit", PLANS_SUBMIT),
                ("Cancel", "cancel", PLANS_SUBMIT),
            ],
            (Task::PlanReviews, "submitted") => vec![
                ("Approve", "approve", PLANS_APPROVE),
                ("Reject", "reject", PLANS_APPROVE),
            ],
            _ => vec![],
        };
        candidates
            .into_iter()
            .filter(|(_, _, cap)| self.can(cap))
            .collect()
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
        if event == "grid" {
            self.columns.handle(&params::string_values(payload));
            return Ok(());
        }
        if let Some(key) = event.strip_prefix("nested-") {
            if let Some(c) = self.nested_columns.get_mut(key) {
                c.handle(&params::string_values(payload));
            }
            return Ok(());
        }
        if matches!(event, "select_company" | "filters") {
            let old = self.company;
            self.list
                .apply_filters(&params::form_values(payload, "filters"));
            self.load(scope).await?;
            if old != self.company {
                self.modal = false;
                self.pending = None;
                self.prior_id = None;
                self.draft.clear();
                self.nested_columns.clear();
            }
            return Ok(());
        }
        if event == "page" {
            self.list
                .put_page(payload.get("page").and_then(Value::as_str));
            return Ok(());
        }
        if event == "close_modal" {
            self.modal = false;
            return Ok(());
        }
        if event == "cancel_decision" {
            self.pending = None;
            return Ok(());
        }
        let Some(company) = self.company else {
            self.fail(NO_WRITE);
            return Ok(());
        };
        match event {
            "open_entry"|"add_item"|"save_entry"|"save_currencies"=> {
                let Some(cap)=self.task.create_cap() else { self.fail(NO_WRITE); return Ok(()); };
                if !self.training.allowed(scope,company,cap).await { self.fail(NO_WRITE); return Ok(()); }
                match event {
                    "open_entry"=> {
                        self.prior_id=None;
                        if let Some(value)=payload.get("value").and_then(Value::as_str).filter(|s|!s.is_empty()) {
                            let id=params::positive_id(Some(value));
                            let eligible=self.records.iter().find(|r|Some(r.id())==id && match r { Record::Plan(p)=>p.status=="approved", Record::Budget(b)=>b.superseded_by.is_none(),_=>false });
                            if eligible.is_none() { self.fail("That record is not available to you."); return Ok(()); }
                            self.prior_id=id;
                        }
                        self.modal=true; self.item_count=1; self.draft.clear(); self.flash=None; self.summary=ErrorSummary::new("learning-errors");
                    },
                    "add_item"=>{ if self.modal && self.task==Task::Plans { self.item_count=(self.item_count+1).min(300); } },
                    "save_currencies"=> {
                        if self.task != Task::Budgets || !self.training.allowed(scope, company, BUDGETS_MANAGE).await { self.fail(NO_WRITE); return Ok(()); }
                        let raw=posted(payload,"currencies[values]");
                        let codes:Vec<String>=raw.split(|c:char|c.is_whitespace()||c==',').filter(|s|!s.is_empty()).map(str::to_owned).collect();
                        let result=self.training.put_learning_currencies(scope,company,&codes).await;
                        self.finish(result,"Currencies saved.");
                    },
                    "save_entry"=> {
                        if !self.modal { self.fail("Choose a learning task first."); return Ok(()); }
                        // Do not use form_values: learning text allows 4,000 characters and plans up to 300 items.
                        self.draft=payload.as_object().into_iter().flat_map(|o|o.iter()).filter(|(k,_)|k.starts_with("entry[")||k.starts_with("items[")).filter_map(|(k,v)|v.as_str().filter(|s|s.len()<=16_000).map(|s|(k.clone(),s.to_owned()))).collect();
                        let result=self.save_entry(scope,company).await;
                        if !self.finish(result,"Learning record created.") {
                            self.summary=ErrorSummary::new("learning-errors");
                            let target=if self.task==Task::Plans { "entry-objectives" } else if self.task==Task::Budgets { "entry-effective_from" } else { "entry-need" };
                            if let Some((_,message))=&self.flash { self.summary.push(target,message.clone()); }
                        }
                    },_=>{},
                }
            },
            "decide"=> {
                let values=params::form_values(payload,"decision");
                let id=params::positive_id(values.get("id").map(String::as_str));
                let action=values.get("action").map(String::as_str).unwrap_or("");
                let eligible=self.records.iter().find(|r|Some(r.id())==id).and_then(|r|self.actions(r).into_iter().find(|(_,a,_)|*a==action));
                let Some((_,_,cap))=eligible else { self.fail(NO_WRITE); return Ok(()); };
                if !self.training.allowed(scope,company,cap).await { self.fail(NO_WRITE); return Ok(()); }
                // Read the full explanation, beyond params::form_values' 1 KiB limit.
                let reason=posted(payload,"decision[reason]");
                if reason.trim().is_empty() || reason.len()>4000 { self.fail("Enter a reason for this decision."); return Ok(()); }
                self.pending=Some(Decision { id:id.ok_or(Error::Invalid)?,action:action.into(),reason,capability:cap });
            },
            "confirm_decision"=> {
                let Some(pending)=self.pending.take() else { return Ok(()); };
                if !self.training.allowed(scope,company,pending.capability).await { self.fail(NO_WRITE); return Ok(()); }
                let result=if self.task.kind()=="request" { self.training.decide_learning_request(scope,company,pending.id,&pending.action,&pending.reason).await.map(|_|()) } else { self.training.decide_learning_plan(scope,company,pending.id,&pending.action,&pending.reason).await.map(|_|()) };
                self.finish(result,"Decision recorded.");
            },
            _=>self.fail("That action is unavailable in the record's current state. Refresh and review it again."),
        }
        Ok(())
    }
    fn value(&self, key: &str) -> String {
        self.draft
            .get(&format!("entry[{key}]"))
            .cloned()
            .unwrap_or_default()
    }
    async fn save_entry(&self, scope: &Scope, company: i64) -> crate::Result<()> {
        match self.task {
            Task::My => self
                .training
                .create_learning_request(
                    scope,
                    company,
                    &NewLearningRequest {
                        course_id: None,
                        need: self.value("need"),
                        objective: self.value("objective"),
                        expected_result: self.value("expected_result"),
                        proposed_on: self.value("proposed_on"),
                        estimated_cost: self.value("estimated_cost"),
                        currency: self.value("currency"),
                    },
                )
                .await
                .map(|_| ()),
            Task::Budgets => {
                let input = NewBudgetPolicy {
                    currency: self.value("currency"),
                    effective_from: self.value("effective_from"),
                    effective_to: self.value("effective_to"),
                    amount: self.value("amount"),
                    reason: self.value("reason"),
                };
                if let Some(id) = self.prior_id {
                    self.training
                        .supersede_budget_policy(scope, company, id, &input)
                        .await
                } else {
                    self.training
                        .create_budget_policy(scope, company, &input)
                        .await
                }
                .map(|_| ())
            }
            Task::Plans => {
                let input = NewLearningPlan {
                    period_start: self.value("period_start"),
                    period_end: self.value("period_end"),
                    objectives: self.value("objectives"),
                    reason: self.value("reason"),
                };
                let items = (0..self.item_count)
                    .map(|i| {
                        let get = |key| {
                            self.draft
                                .get(&format!("items[{i}][{key}]"))
                                .cloned()
                                .unwrap_or_default()
                        };
                        NewLearningPlanItem {
                            request_id: None,
                            need: get("need"),
                            expected_result: get("expected_result"),
                            target_cohort: get("target_cohort"),
                            responsible_owner: get("responsible_owner"),
                            intended_timing: get("intended_timing"),
                            evaluation_approach: get("evaluation_approach"),
                        }
                    })
                    .collect::<Vec<_>>();
                if let Some(id) = self.prior_id {
                    self.training
                        .amend_learning_plan(scope, company, id, &input, &items)
                        .await
                } else {
                    self.training
                        .create_learning_plan(scope, company, &input, &items)
                        .await
                }
                .map(|_| ())
            }
            _ => Err(TrainingError::Unauthorized),
        }
    }
    fn fields(&self) -> Vec<Field> {
        let definitions = match self.task {
            Task::My => vec![
                ("need", "Learning need", InputKind::Text),
                ("objective", "Learning objective", InputKind::Textarea),
                ("expected_result", "Expected result", InputKind::Text),
                ("proposed_on", "Proposed date", InputKind::Date),
                ("estimated_cost", "Estimated cost", InputKind::Number),
                ("currency", "Currency", InputKind::Text),
            ],
            Task::Plans => vec![
                ("objectives", "Objectives", InputKind::Textarea),
                ("period_start", "Period start", InputKind::Date),
                ("period_end", "Period end", InputKind::Date),
                (
                    "reason",
                    "Creation or amendment reason",
                    InputKind::Textarea,
                ),
            ],
            Task::Budgets => vec![
                ("currency", "Currency", InputKind::Text),
                ("effective_from", "Effective from", InputKind::Date),
                ("effective_to", "Effective through", InputKind::Date),
                ("amount", "Allocation", InputKind::Number),
                (
                    "reason",
                    if self.prior_id.is_some() {
                        "Correction reason"
                    } else {
                        "Allocation reason"
                    },
                    InputKind::Textarea,
                ),
            ],
            _ => vec![],
        };
        definitions
            .into_iter()
            .filter(|(key, _, _)| {
                !(*key == "currency" && self.task == Task::Budgets && self.prior_id.is_some())
            })
            .map(|(key, label, kind)| {
                let mut field = Field::in_form("entry", key)
                    .id(format!("entry-{key}"))
                    .label(label)
                    .kind(kind)
                    .value(self.value(key))
                    .required();
                if matches!(key, "estimated_cost" | "amount") {
                    field = field.attr("min", "0").attr("step", "0.0001");
                }
                field.attr("maxlength", if key == "currency" { "3" } else { "4000" })
            })
            .collect()
    }
    fn entry_dialog(&self) -> DbResult<Html> {
        if !self.modal || !self.can_create() {
            return Ok(Html::EMPTY);
        }
        let mut items = vec![];
        if self.task == Task::Plans {
            for i in 0..self.item_count {
                let fields = [
                    ("need", "Need"),
                    ("expected_result", "Expected result"),
                    ("target_cohort", "Target cohort"),
                    ("responsible_owner", "Responsible owner"),
                    ("intended_timing", "Intended timing"),
                    ("evaluation_approach", "Evaluation approach"),
                ]
                .into_iter()
                .map(|(key, label)| {
                    let name = format!("items[{i}][{key}]");
                    Field::named(&name)
                        .id(format!("item-{i}-{key}"))
                        .label(label)
                        .value(self.draft.get(&name).cloned().unwrap_or_default())
                        .required()
                        .attr("maxlength", "4000")
                })
                .collect();
                items.push((format!("Learning item {}", i + 1), fields));
            }
        }
        let body = render(&EntryView {
            fields: self.fields(),
            items,
            summary: &self.summary,
            is_plan: self.task == Task::Plans,
            add_item: button("add_item", "Add learning item", "")?,
            actions: join([button("close_modal", "Cancel", "")?, submit("Save")?]),
        })?;
        let title = if self.prior_id.is_some() {
            if self.task == Task::Budgets {
                "Correct budget policy".into()
            } else {
                "Amend plan".into()
            }
        } else {
            format!("Add {}", self.task.kind())
        };
        render(&Modal {
            id: "learning-entry",
            title: &title,
            cancel_event: "close_modal",
            flash: Some(self.flash_map()),
            body,
            ..Modal::default()
        })
    }
    fn flash_map(&self) -> FlashMap<'_> {
        let mut flash = NO_FLASH;
        if let Some((kind, message)) = &self.flash {
            match kind {
                StatusKind::Success => flash.success = message,
                StatusKind::Error => flash.error = message,
                StatusKind::Info => flash.info = message,
                StatusKind::Warning => flash.warning = message,
            }
        }
        flash
    }
    fn nested_table(&self, key: &str, rows: Vec<FlexRow>) -> DbResult<Html> {
        let Some(columns) = self.nested_columns.get(key) else {
            return Ok(Html::EMPTY);
        };
        let event = format!("nested-{key}");
        let mut table = FlexTable::new(key, columns.flex_columns(), rows);
        table.event = &event;
        table.zoom = columns.zoom;
        table.suggestions = columns.flex_suggestions();
        table.add_query = &columns.add_query;
        render(&table)
    }
    fn history(&self, id: i64) -> DbResult<Html> {
        let rows = self
            .histories
            .get(&id)
            .into_iter()
            .flatten()
            .map(|d| {
                let mut row = FlexRow::new(d.id.to_string())
                    .cell("action", bilibi_ui::Cell::text(&d.action))
                    .cell("user", bilibi_ui::Cell::text(d.actor_user_id.to_string()))
                    .cell(
                        "impersonator",
                        bilibi_ui::Cell::text(
                            d.impersonator_id
                                .map(|id| id.to_string())
                                .unwrap_or_default(),
                        ),
                    )
                    .cell("reason", bilibi_ui::Cell::text(&d.reason));
                if let (Some(clock), Ok(instant)) = (&self.clock, learning_instant(&d.inserted_at))
                {
                    let r = clock.render(
                        instant,
                        bilibi_datetime::Format::DateTime,
                        bilibi_datetime::Precision::Minute,
                    );
                    row = row.cell(
                        "time",
                        bilibi_ui::Cell::instant(bilibi_ui::Instant::new(
                            &r.iso,
                            &r.text,
                            &r.company,
                            &r.utc,
                            r.mode.as_str(),
                        )),
                    );
                }
                row
            })
            .collect();
        render(&Fold {
            id: &format!("learning-history-{id}"),
            title: "History",
            body: self.nested_table(&format!("history-{id}"), rows)?,
            ..Fold::default()
        })
    }
    fn record_content(&self, record: &Record) -> DbResult<(Html, Html)> {
        match record {
            Record::Request(r) => Ok((
                render(&RecordView {
                    heading: &r.need,
                    lines: vec![
                        r.objective.clone(),
                        r.expected_result.clone(),
                        format!("Employee {}", r.employee_id),
                    ],
                    extra: Html::EMPTY,
                })?,
                render(&RecordView {
                    heading: &r.status,
                    lines: vec![format!(
                        "{} {} · {}",
                        r.estimated_cost, r.currency, r.proposed_on
                    )],
                    extra: Html::EMPTY,
                })?,
            )),
            Record::Plan(p) => {
                let rows = p
                    .items
                    .iter()
                    .map(|item| {
                        FlexRow::new(item.id.to_string())
                            .cell("need", bilibi_ui::Cell::text(&item.need))
                            .cell("result", bilibi_ui::Cell::text(&item.expected_result))
                            .cell("cohort", bilibi_ui::Cell::text(&item.target_cohort))
                            .cell("owner", bilibi_ui::Cell::text(&item.responsible_owner))
                            .cell("timing", bilibi_ui::Cell::text(&item.intended_timing))
                            .cell(
                                "evaluation",
                                bilibi_ui::Cell::text(&item.evaluation_approach),
                            )
                    })
                    .collect();
                let items = render(&Fold {
                    id: &format!("learning-items-{}", p.id),
                    title: "Learning items",
                    body: self.nested_table(&format!("items-{}", p.id), rows)?,
                    ..Fold::default()
                })?;
                Ok((
                    render(&RecordView {
                        heading: &p.objectives,
                        lines: vec![
                            format!("Version {}", p.version),
                            format!("{} – {}", p.period_start, p.period_end),
                        ],
                        extra: items,
                    })?,
                    Html::text(&p.status),
                ))
            }
            Record::Budget(b) => {
                let p = &b.policy;
                let mut lines = vec![
                    format!("{} – {}", p.effective_from, p.effective_to),
                    p.reason.clone(),
                ];
                if let Some(id) = p.supersedes_id {
                    lines.push(format!("Corrects policy {id}"));
                }
                let status = b
                    .superseded_by
                    .map(|id| format!("Superseded by policy {id}"))
                    .unwrap_or_default();
                Ok((
                    render(&RecordView {
                        heading: &p.currency,
                        lines,
                        extra: Html::EMPTY,
                    })?,
                    render(&RecordView {
                        heading: &status,
                        lines: vec![
                            format!("Allocation {}", p.amount),
                            format!("Committed {}", b.committed),
                            format!("Remaining {}", b.remaining),
                        ],
                        extra: Html::EMPTY,
                    })?,
                ))
            }
        }
    }
    fn decisions(&self, record: &Record) -> DbResult<Html> {
        let mut content = if self.task.kind() != "budget" {
            self.history(record.id())?
        } else {
            Html::EMPTY
        };
        let actions = self.actions(record);
        if !actions.is_empty() {
            let fields = vec![
                Field::in_form("decision", "id")
                    .id(format!("decision-{}-id", record.id()))
                    .kind(InputKind::Hidden)
                    .value(record.id().to_string()),
                Field::in_form("decision", "action")
                    .id(format!("decision-{}-action", record.id()))
                    .label("Action")
                    .kind(InputKind::Select)
                    .options(
                        actions
                            .iter()
                            .map(|(label, action, _)| Choice::new(*label, *action))
                            .collect(),
                    ),
                Field::in_form("decision", "reason")
                    .id(format!("decision-{}-reason", record.id()))
                    .label("Reason")
                    .required()
                    .attr("maxlength", "4000"),
            ];
            content.push(&render(&FieldsView {
                id: &format!("decision-{}", record.id()),
                event: "decide",
                fields,
                actions: submit("Record decision")?,
            })?);
        }
        let amend = match record {
            Record::Plan(p) => p.status == "approved" && self.can_create(),
            Record::Budget(b) => b.superseded_by.is_none() && self.can_create(),
            _ => false,
        };
        if amend {
            content.push(&button(
                "open_entry",
                if self.task.kind() == "budget" {
                    "Correct"
                } else {
                    "Amend"
                },
                &record.id().to_string(),
            )?);
        }
        Ok(content)
    }
    fn render(&mut self) -> DbResult<String> {
        let show = self.show_records();
        let mut table_html = Html::EMPTY;
        let mut pagination = Html::EMPTY;
        if self.company.is_some() && self.error.is_none() && show {
            let page = self.list.paginate(&self.records);
            let counts = page.counts;
            let rows = page
                .entries
                .iter()
                .map(|r| {
                    let (content, status) = self.record_content(r)?;
                    let mut row = FlexRow::new(r.id().to_string())
                        .slot("record", content)
                        .slot("status", status)
                        .slot("decisions", self.decisions(r)?);
                    row.dom_id = Some(format!("learning-{}", r.id()));
                    Ok(row)
                })
                .collect::<DbResult<Vec<_>>>()?;
            let mut table = FlexTable::new("learning-records", self.columns.flex_columns(), rows);
            table.event = "grid";
            table.caption = self.task.title();
            table.zoom = self.columns.zoom;
            table.suggestions = self.columns.flex_suggestions();
            table.add_query = &self.columns.add_query;
            table.empty=Some(TableEmpty::State(EmptyState::missing("No learning records yet","Choose a company and add a request, plan or budget when you have the required access.")));
            table_html = render(&table)?;
            pagination = render(&Pagination::new(
                "learning-pagination",
                counts,
                &[25, 50, 100, 300],
            ))?;
        }
        let tabs = render(&Tabs {
            id: "learning-tasks",
            label: "Learning tasks",
            tabs: self
                .tabs
                .iter()
                .map(|(label, path, current)| Tab::link(path, label, *current))
                .collect(),
            ..Tabs::default()
        })?;
        let toolbar = render(&bilibi_ui::FilterToolbar {
            id: "learning-filters",
            event: "select_company",
            controls: vec![bilibi_ui::ToolbarControl::select(
                "learning-company",
                "company_id",
                "Company",
                &self.company.map(|id| id.to_string()).unwrap_or_default(),
                self.companies
                    .iter()
                    .map(|c| Choice::new(&c.name, c.id.to_string()))
                    .collect(),
            )],
            class: "",
            target: "",
        })?;
        let writable =
            self.can_create() || self.records.iter().any(|r| !self.actions(r).is_empty());
        let currencies = if self.task == Task::Budgets && self.can(BUDGETS_MANAGE) {
            render(&FieldsView {
                id: "learning-currencies",
                event: "save_currencies",
                fields: vec![Field::in_form("currencies", "values")
                    .id("learning-currency-values")
                    .label("Company currencies")
                    .value(&self.currencies)
                    .placeholder("Comma-separated currency codes")],
                actions: submit("Save currencies")?,
            })?
        } else {
            Html::EMPTY
        };
        let confirmation = if let Some(p) = &self.pending {
            let dialog=render(&ConfirmDialog { id:"learning-decision-confirm",consequence:&format!("Record {} for this learning record?",p.action),detail:&format!("{} The decision and your reason will remain in history. A terminal request cannot be reopened; an approved plan needs a reasoned amendment.", self.instance_notice),confirm:"Record decision",working:"Recording…",confirm_event:"confirm_decision",cancel_event:"cancel_decision" })?;
            // The banner belongs inside the review dialog, beside its consequence.
            // ConfirmDialog has no body slot, so use the shared instance banner immediately before it.
            join([self.banner.clone(), dialog])
        } else {
            Html::EMPTY
        };
        let view = LearningView {
            title: self.task.title(),
            flash: self.flash_map(),
            header_actions: if self.can_create() {
                button("open_entry", &format!("Add {}", self.task.kind()), "")?
            } else {
                Html::EMPTY
            },
            tabs,
            toolbar,
            read_only: !writable && self.error.is_none(),
            error: self.error.as_deref().unwrap_or(""),
            choose_task: if !show {
                render(&EmptyState::missing("Choose a learning task", "Use your authorized passport, evidence or evaluation tasks above. Request submission needs separate access."))?
            } else {
                Html::EMPTY
            },
            currencies,
            table: table_html,
            pagination,
            entry: self.entry_dialog()?,
            confirmation,
        };
        view.render().map_err(|_| Error::Internal)
    }
}
impl LiveState<Scope, Error> for Learning {
    fn update<'a>(&'a mut self, req: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            // Refresh the visible audience and grants before decoding every event.
            self.load(req.scope).await?;
            match req.input {
                LiveInput::Refresh => {}
                LiveInput::Params(query) => {
                    self.list = ListState::parse(query, spec()?).map_err(|_| Error::Internal)?;
                    self.modal = false;
                    self.pending = None;
                    self.prior_id = None;
                }
                LiveInput::Event(payload) => self.event(req.scope, payload).await?,
            }
            self.load(req.scope).await?;
            Ok(LiveBody {
                html: self.render()?,
                query: self.list.to_params(),
                rows: self.records.len(),
                ..LiveBody::default()
            })
        })
    }
}
// Governance stores inserted_at as a UTC timestamp without a zone marker
// (003-learning-governance.sql). Attach that declared zone before DisplayPolicy.
fn learning_instant(value: &str) -> std::result::Result<jiff::Timestamp, jiff::Error> {
    value.parse().or_else(|_| format!("{value}Z").parse())
}
fn posted(payload: &Value, name: &str) -> String {
    payload
        .get(name)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}
fn render(template: &impl Template) -> DbResult<Html> {
    Html::render(template).map_err(|_| Error::Internal)
}
fn join(parts: impl IntoIterator<Item = Html>) -> Html {
    parts.into_iter().collect()
}
fn button(event: &str, label: &str, value: &str) -> DbResult<Html> {
    render(&Button {
        event,
        button_type: "button",
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
#[derive(Template)]
#[template(path = "learning.html")]
struct LearningView<'a> {
    title: &'a str,
    flash: FlashMap<'a>,
    header_actions: Html,
    tabs: Html,
    toolbar: Html,
    read_only: bool,
    error: &'a str,
    choose_task: Html,
    currencies: Html,
    table: Html,
    pagination: Html,
    entry: Html,
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
#[template(path = "learning_entry.html")]
struct EntryView<'a> {
    fields: Vec<Field>,
    items: Vec<(String, Vec<Field>)>,
    summary: &'a ErrorSummary,
    is_plan: bool,
    add_item: Html,
    actions: Html,
}
#[derive(Template)]
#[template(path = "learning_record.html")]
struct RecordView<'a> {
    heading: &'a str,
    lines: Vec<String>,
    extra: Html,
}
