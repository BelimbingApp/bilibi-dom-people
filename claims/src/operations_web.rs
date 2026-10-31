//! Bilimbi Claims.Web.OperationsLive: decisions, reimbursement and hand-off.
use crate::{
    AssignableEmployee, Claims, ClaimsError, HandoffBatch, HandoffWaiting, QueueClaim, APPROVE,
    REIMBURSE,
};
use askama::Template;
use bilibi_database::{Db, Error, Result};
use bilibi_people_workforce::{authorization, CompanySummary};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::{
        LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState, MountFuture, Page, PageBody,
        PageFuture, PageRequest,
    },
    params, BuiltinColumn, Button, ButtonVariant, Choice, ConfirmDialog, EmptyState, Field, Filter,
    FilterToolbar, FlashMap, FlexRow, FlexTable, Html, InputKind, ListColumns, ListSpec, ListState,
    Modal, StatusKind, TableEmpty, ToolbarControl, NO_FLASH,
};
use rust_decimal::Decimal;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

pub(crate) const SERVICE: &str = "people/claims.pages";
const PATH: &str = "/people/claims";

const FORBIDDEN: &str = "You are not allowed to do that for this company.";
struct Core {
    db: Db,
    claims: Arc<Claims>,
    contracts: Arc<bilibi_agent_api::page::PageContracts>,
}
pub(crate) struct Pages(Arc<Core>);
impl Pages {
    pub(crate) fn new(
        db: Db,
        claims: Arc<Claims>,
        contracts: Arc<bilibi_agent_api::page::PageContracts>,
    ) -> Self {
        Self(Arc::new(Core {
            db,
            claims,
            contracts,
        }))
    }
}
impl Page<Scope, Error> for Pages {
    fn title(&self, _: &str) -> Option<&'static str> {
        Some("Claims")
    }
    fn render<'a>(&'a self, request: PageRequest<'a, Scope>) -> PageFuture<'a, Error> {
        Box::pin(async move {
            let mut query = request.query.clone();
            query.remove("id");
            query.extend(request.path.clone());
            let mut state = Operations::new(self.0.clone(), &query)?;
            state.workspace = request.framed;
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
            Ok(Some(Box::new({
                let mut query = request.query.clone();
                query.remove("id");
                query.extend(request.path.clone());
                let mut state = Operations::new(self.0.clone(), &query)?;
                state.workspace = request.framed;
                state
            }) as Box<dyn LiveState<Scope, Error>>))
        })
    }
}
fn spec() -> Result<Arc<ListSpec>> {
    ListSpec {
        filters: BTreeMap::from([
            ("company_id".into(), Filter::string("")),
            ("status".into(), Filter::string("")),
            ("audience".into(), Filter::string("mine")),
            ("employee_id".into(), Filter::string("")),
        ]),
        // Bilimbi's queues are bounded at 500, with no pagination or search.
        page_sizes: vec![500],
        default_page_size: 500,
        omit_defaults: true,
        ..ListSpec::default()
    }
    .shared()
    .map_err(|_| Error::Internal)
}
fn columns(id: &str, fields: &[(&str, &str)]) -> Result<ListColumns> {
    ListColumns::mount(
        id,
        fields
            .iter()
            .map(|(id, label)| BuiltinColumn::new(id, label))
            .collect(),
    )
    .map_err(|_| Error::Internal)
}
fn queue_columns(_: &str) -> Result<ListColumns> {
    columns(
        "claims-table",
        &[
            ("employee", "Employee"),
            ("claim", "Claim"),
            ("date", "Date"),
            ("claimed", "Claimed"),
            ("approved", "Approved"),
            ("status", "Status"),
            ("receipt", "Receipt"),
            ("batch", "Batch"),
            ("reason", "Reason"),
            ("paid", "Reimbursement"),
        ],
    )
}
#[derive(Clone, Copy)]
enum Action {
    Approve,
    Reject,
    Reimburse,
    Batch,
}
impl Action {
    fn capability(self) -> &'static str {
        match self {
            Self::Approve | Self::Reject => APPROVE,
            _ => REIMBURSE,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Approve => "Approve",
            Self::Reject => "Reject",
            _ => "Mark reimbursed",
        }
    }
}
#[derive(Clone)]
enum Subject {
    Claim(Box<QueueClaim>),
    Batch(HandoffBatch),
    Currency(String),
}
#[derive(Clone)]
struct Selection {
    company: i64,
    action: Action,
    subject: Subject,
}
struct Pending {
    selection: Selection,
    values: HashMap<String, String>,
}
struct Operations {
    core: Arc<Core>,
    list: ListState,
    columns: ListColumns,
    batches_columns: ListColumns,
    waiting_columns: ListColumns,
    companies: Vec<CompanySummary>,
    company: Option<CompanySummary>,
    can_reimburse: bool,
    can_review: bool,
    submission: crate::submission::Submission,
    record: Option<i64>,
    workspace: bool,

    rows: Vec<QueueClaim>,
    employees: Vec<AssignableEmployee>,
    batches: Vec<HandoffBatch>,
    waiting: Vec<HandoffWaiting>,
    selection: Option<Selection>,
    pending: Option<Pending>,
    values: HashMap<String, String>,
    flash: Option<(StatusKind, String)>,
    display: bilibi_datetime::DisplayContext,
    instance: Option<bilibi_system::Instance>,
}
impl Operations {
    fn new(core: Arc<Core>, query: &HashMap<String, String>) -> Result<Self> {
        if query.contains_key("id")
            && params::positive_id(query.get("id").map(String::as_str)).is_none()
        {
            return Err(Error::NotFound);
        }
        let mut state = Self {
            submission: crate::submission::Submission::new(
                core.claims.clone(),
                core.contracts.clone(),
                query,
            )?,
            core,
            can_review: false,
            record: params::positive_id(query.get("id").map(String::as_str)),
            workspace: false,
            list: ListState::parse(query, spec()?).map_err(|_| Error::Internal)?,
            columns: queue_columns("submitted")?,
            batches_columns: columns(
                "claim-batches-list",
                &[
                    ("batch", "Batch"),
                    ("count", "Claims"),
                    ("total", "Total"),
                    ("created", "Created"),
                ],
            )?,
            waiting_columns: columns(
                "claim-handoff-waiting",
                &[
                    ("currency", "Currency"),
                    ("count", "Approved claims"),
                    ("total", "Total not handed off"),
                ],
            )?,
            companies: vec![],
            company: None,
            can_reimburse: false,
            rows: vec![],
            employees: vec![],
            batches: vec![],
            waiting: vec![],
            selection: None,
            pending: None,
            values: HashMap::new(),
            flash: None,
            display: bilibi_datetime::DisplayContext::anonymous(),
            instance: None,
        };
        state.normalize();
        state.columns = queue_columns(state.tab())?;
        Ok(state)
    }
    fn tab(&self) -> &str {
        &self.list.filters["status"]
    }
    fn normalize(&mut self) {
        if !self.tab().is_empty() && !crate::REQUEST_STATUSES.contains(&self.tab()) {
            self.list.filters.insert("status".into(), String::new());
        }
        self.list.page = 1;
    }
    fn clear(&mut self) {
        self.selection = None;
        self.pending = None;
        self.values.clear();
    }
    fn message(&mut self, kind: StatusKind, message: &str) {
        self.flash = Some((kind, message.into()));
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
    async fn load(&mut self, scope: &Scope) -> Result<()> {
        let client = self.core.db.get().await?;
        let current = scope.current_company().map(|id| id.get());
        self.companies =
            match authorization::company_choices(&**client, scope, crate::SUBMIT, current).await {
                Ok(rows) => rows,
                Err(bilibi_people_workforce::WorkforceError::Failed(e)) => return Err(e),
                Err(_) => vec![],
            };
        let review = match authorization::company_choices(&**client, scope, APPROVE, current).await
        {
            Ok(rows) => rows,
            Err(bilibi_people_workforce::WorkforceError::Failed(e)) => return Err(e),
            Err(_) => vec![],
        };
        for c in review {
            if !self.companies.iter().any(|r| r.id == c.id) {
                self.companies.push(c);
            }
        }
        self.company = self
            .companies
            .iter()
            .find(|c| Some(c.id) == current)
            .cloned();
        self.can_review = if let Some(c) = &self.company {
            authorization::allowed(&**client, scope, c.id, APPROVE).await
        } else {
            false
        };
        self.can_reimburse = if let Some(c) = &self.company {
            authorization::allowed(&**client, scope, c.id, REIMBURSE).await
        } else {
            false
        };
        if let Some(c) = &self.company {
            self.display = bilibi_datetime::DisplayPolicy::new(self.core.claims.settings.clone())
                .display(
                    &**client,
                    scope,
                    Some(bilibi_party_ids::OrganisationId::new(c.id).map_err(|_| Error::Invalid)?),
                    "en",
                )
                .await?;
        }
        self.instance =
            Some(bilibi_system::Instance::resolve(&self.core.claims.settings, &**client).await?);
        drop(client);
        self.submission.load(scope).await?;
        self.rows.clear();
        self.employees.clear();
        self.batches.clear();
        self.waiting.clear();
        if let Some(c) = &self.company {
            let company = c.id;
            self.list
                .filters
                .insert("company_id".into(), company.to_string());
            let mut team = self.list.filters["audience"] == "team" && self.can_review;
            if !self.submission.available() && self.can_review {
                team = true;
            }
            self.list
                .filters
                .insert("audience".into(), if team { "team" } else { "mine" }.into());
            if !team {
                self.list
                    .filters
                    .insert("employee_id".into(), String::new());
            }
            let loaded = self
                .core
                .claims
                .requests(
                    scope,
                    company,
                    if self.record.is_some() {
                        self.can_review
                    } else {
                        team
                    },
                    self.tab(),
                    params::positive_id(self.list.filters.get("employee_id").map(String::as_str)),
                    self.record,
                )
                .await;
            match loaded {
                Ok(rows) => self.rows = rows,
                Err(ClaimsError::Failed(e)) => return Err(e),
                Err(e) => self.refused(e),
            }
            if team && self.record.is_none() {
                match self
                    .core
                    .claims
                    .request_employees(scope, company, self.tab())
                    .await
                {
                    Ok(rows) => self.employees = rows,
                    Err(ClaimsError::Failed(e)) => return Err(e),
                    Err(e) => self.refused(e),
                }
            }
            if self.can_review && self.can_reimburse && self.record.is_none() {
                self.waiting = self
                    .core
                    .claims
                    .handoff_waiting(scope, company)
                    .await
                    .unwrap_or_default();
                self.batches = self
                    .core
                    .claims
                    .handoff_batches(scope, company)
                    .await
                    .unwrap_or_default();
            }
        }
        let selected = self
            .pending
            .as_ref()
            .map(|p| &p.selection)
            .or(self.selection.as_ref());
        if selected.is_some_and(|selection| {
            self.company.as_ref().map(|c| c.id) != Some(selection.company)
                || !self.can_review
                || selection.action.capability() == REIMBURSE && !self.can_reimburse
        }) {
            self.clear();
            self.message(StatusKind::Error, FORBIDDEN);
        }
        Ok(())
    }
    async fn permitted(&mut self, scope: &Scope, capability: &str, refusal: &str) -> Result<bool> {
        let company = self
            .pending
            .as_ref()
            .map(|p| p.selection.company)
            .or_else(|| self.selection.as_ref().map(|s| s.company))
            .or_else(|| self.company.as_ref().map(|c| c.id));
        let Some(company) = company else {
            self.clear();
            self.message(StatusKind::Error, refusal);
            return Ok(false);
        };
        let client = self.core.db.get().await?;
        let allowed = authorization::allowed(&**client, scope, company, APPROVE).await
            && authorization::allowed(&**client, scope, company, capability).await;
        if !allowed {
            self.clear();
            self.message(StatusKind::Error, refusal);
        }
        Ok(allowed)
    }
    fn refused(&mut self, error: ClaimsError) {
        let text = match error {
            ClaimsError::OwnClaim => "You cannot act on your own claim.",
            ClaimsError::NotDecidable => {
                "This claim is no longer in the right state for that action."
            }
            ClaimsError::NothingToHandOff => "No approved claims in that currency are waiting.",
            ClaimsError::NothingToReimburse => "No approved claims are left in this batch.",
            ClaimsError::NotFound => "That record is no longer available.",
            ClaimsError::Unauthorized => {
                "You no longer have permission to do that for this company's claims."
            }
            ClaimsError::CompanyArchived => "This company is archived and read-only",
            ClaimsError::Invalid(errors) if errors.iter().any(|e| e.field == "decision_reason") => {
                "Enter a reason for this decision."
            }
            ClaimsError::Invalid(errors) if errors.iter().any(|e| e.field == "approved_amount") => {
                "Check the approved amount: up to the claim."
            }
            ClaimsError::Invalid(_) => "Check the values entered.",
            _ => "The action could not be completed.",
        };
        self.message(StatusKind::Error, text);
    }
    async fn event(&mut self, scope: &Scope, credential: &str, payload: &Value) -> Result<()> {
        let event = payload.get("type").and_then(Value::as_str).unwrap_or("");
        let action = match event {
            "approve" => Some(Action::Approve),
            "reject" => Some(Action::Reject),
            "reimburse" => Some(Action::Reimburse),
            "reimburse_batch" => Some(Action::Batch),
            _ => None,
        };
        let capability = if let Some(action) = action {
            Some(action.capability())
        } else if event == "handoff" {
            Some(REIMBURSE)
        } else if matches!(event, "review" | "confirm") {
            self.pending
                .as_ref()
                .map(|p| p.selection.action.capability())
                .or_else(|| self.selection.as_ref().map(|s| s.action.capability()))
        } else {
            None
        };
        if let Some(capability) = capability {
            if !self
                .permitted(scope, capability, selection_refusal(event))
                .await?
            {
                return Ok(());
            }
        }
        match event {
            "clear-flash" => {
                self.flash = None;
                self.submission.event(scope, credential, payload).await?;
            }
            "cancel" => self.clear(),
            "filters" => {
                self.clear();
                self.list
                    .apply_filters(&params::form_values(payload, "filters"));
                self.normalize();
                self.columns = queue_columns(self.tab())?;
            }
            "grid" => {
                self.columns.handle(&params::string_values(payload));
            }
            "batches_grid" => {
                self.batches_columns.handle(&params::string_values(payload));
            }
            "waiting_grid" => {
                self.waiting_columns.handle(&params::string_values(payload));
            }
            "handoff" => {
                if self.pending.is_some() {
                    return Ok(());
                }
                let value = payload.get("value").and_then(Value::as_str).unwrap_or("");
                if self.waiting.iter().any(|w| w.currency == value) {
                    if let Some(c) = &self.company {
                        self.pending = Some(Pending {
                            selection: Selection {
                                company: c.id,
                                action: Action::Batch,
                                subject: Subject::Currency(value.into()),
                            },
                            values: HashMap::new(),
                        });
                    }
                } else {
                    self.message(
                        StatusKind::Error,
                        "No approved claims in that currency are waiting.",
                    );
                }
            }
            "approve" | "reject" | "reimburse" | "reimburse_batch" => {
                if self.pending.is_some() {
                    return Ok(());
                }
                self.clear();
                let id = payload
                    .get("value")
                    .and_then(Value::as_str)
                    .and_then(|v| v.parse::<i64>().ok());
                let subject = if event == "reimburse_batch" {
                    self.batches
                        .iter()
                        .find(|b| Some(b.id) == id)
                        .cloned()
                        .map(Subject::Batch)
                } else {
                    self.rows
                        .iter()
                        .find(|r| {
                            Some(r.request.id) == id
                                && r.request.status
                                    == if event == "reimburse" {
                                        "approved"
                                    } else {
                                        "submitted"
                                    }
                        })
                        .cloned()
                        .map(|row| Subject::Claim(Box::new(row)))
                };
                if let (Some(subject), Some(c), Some(action)) = (subject, &self.company, action) {
                    self.selection = Some(Selection {
                        company: c.id,
                        action,
                        subject,
                    });
                } else {
                    self.message(StatusKind::Error, selection_refusal(event));
                }
            }
            "review" => {
                if self.pending.is_some() {
                    return Ok(());
                }
                self.values = params::form_values(payload, "decision");
                if let Some(selection) = self.selection.clone() {
                    if matches!(selection.action, Action::Reject)
                        && self.value("decision_reason").trim().is_empty()
                    {
                        self.message(
                            StatusKind::Error,
                            "Enter a reason before rejecting a claim.",
                        );
                    } else if matches!(selection.action, Action::Approve)
                        && !self.value("approved_amount").trim().is_empty()
                        && self.value("approved_amount").parse::<Decimal>().is_err()
                    {
                        self.message(
                            StatusKind::Error,
                            "Check the approved amount: up to the claim.",
                        );
                    } else {
                        self.pending = Some(Pending {
                            selection,
                            values: self.values.clone(),
                        });
                    }
                }
            }
            "confirm" => {
                if let Some(pending) = self.pending.take() {
                    self.selection = None;
                    self.values.clear();
                    let Pending { selection, values } = pending;
                    let company = selection.company;
                    let mut input = serde_json::json!({"company_id":company});
                    for key in ["approved_amount", "decision_reason", "payment_reference"] {
                        if let Some(value) = values.get(key).filter(|v| !v.trim().is_empty()) {
                            input[key] = Value::String(value.clone());
                        }
                    }
                    let (contract, message) = match (selection.action, selection.subject) {
                        (Action::Approve, Subject::Claim(row)) => {
                            input["request_id"] = row.request.id.into();
                            ("people.claims.request.approve", "Claim approved.")
                        }
                        (Action::Reject, Subject::Claim(row)) => {
                            input["request_id"] = row.request.id.into();
                            ("people.claims.request.reject", "Claim rejected.")
                        }
                        (Action::Reimburse, Subject::Claim(row)) => {
                            input["request_id"] = row.request.id.into();
                            (
                                "people.claims.request.reimburse",
                                "Claim marked reimbursed.",
                            )
                        }
                        (_, Subject::Currency(currency)) => {
                            input["currency"] = currency.into();
                            ("people.claims.handoff.create", "Hand-off batch created.")
                        }
                        (_, Subject::Batch(batch)) => {
                            input["batch_id"] = batch.id.into();
                            (
                                "people.claims.handoff.reimburse",
                                "Batch marked reimbursed.",
                            )
                        }
                        _ => return Ok(()),
                    };
                    let result = self
                        .core
                        .contracts
                        .submit(
                            &bilibi_agent_api::page::Call {
                                scope,
                                credential,
                                service: SERVICE,
                                event: "confirm",
                                contract,
                            },
                            input,
                        )
                        .await
                        .map_err(crate::agent::page_error);
                    match result {
                        Ok(_) => self.message(StatusKind::Success, message),
                        Err(error) => self.refused(error),
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
    fn moment(&self, id: String, value: Option<&str>) -> Result<Html> {
        let Some(instant) = value.and_then(|s| s.parse::<jiff::Timestamp>().ok()) else {
            return Ok(Html::text("—"));
        };
        let shown = self.display.render(
            instant,
            bilibi_datetime::Format::DateTime,
            bilibi_datetime::Precision::Minute,
        );
        render(&Moment {
            id,
            instant: bilibi_ui::flex_table::Instant::new(
                &shown.iso,
                &shown.text,
                &shown.company,
                &shown.utc,
                shown.mode.as_str(),
            ),
        })
    }
    fn queue(&self) -> Result<Html> {
        let compact = self.columns.mode() == bilibi_ui::RowMode::Compact;
        let rows = self
            .rows
            .iter()
            .map(|row| {
                let r = &row.request;
                let actions = self.claim_actions(row)?;
                let mut paid =
                    self.moment(format!("claim-paid-{}", r.id), r.reimbursed_at.as_deref())?;
                if let Some(reference) = &r.payment_reference {
                    paid.push(&Html::text(&format!(" · {reference}")));
                }
                let mut item = FlexRow::new(format!("claim-row-{}", r.id))
                    .slot(
                        "employee",
                        Html::text(&format!(
                            "{} {}",
                            row.employee_name.as_deref().unwrap_or("—"),
                            row.employee_number.as_deref().unwrap_or("")
                        )),
                    )
                    .slot(
                        "claim",
                        render(&ClaimCell {
                            row,
                            compact,
                            workspace: self.workspace,
                            href: format!(
                                "{PATH}/{}?company_id={}",
                                r.id,
                                self.company.as_ref().map_or(0, |c| c.id)
                            ),
                        })?,
                    )
                    .slot("status", Html::text(&r.status))
                    .slot("date", Html::text(&r.incurred_on.to_string()))
                    .slot(
                        "claimed",
                        Html::text(&format!("{} {}", r.amount, r.currency)),
                    )
                    .slot("approved", Html::text(&money(r.approved_amount)))
                    .slot(
                        "receipt",
                        Html::text(r.receipt_number.as_deref().unwrap_or("—")),
                    )
                    .slot(
                        "batch",
                        Html::text(
                            &r.handoff_batch_id
                                .map_or_else(|| "—".into(), |id| id.to_string()),
                        ),
                    )
                    .slot(
                        "reason",
                        Html::text(r.decision_reason.as_deref().unwrap_or("")),
                    )
                    .slot("paid", paid);
                item.actions = actions;
                Ok(item)
            })
            .collect::<Result<Vec<_>>>()?;
        let mut table = FlexTable::new("claim-queue-table", self.columns.flex_columns(), rows);
        table.event = "grid";
        table.zoom = self.columns.zoom;
        table.suggestions = self.columns.flex_suggestions();
        table.add_query = &self.columns.add_query;
        table.actions = true;
        table.empty = Some(TableEmpty::State(
            EmptyState::missing("No claims match these filters.", "").id("claim-queue-empty"),
        ));
        render(&table)
    }
    fn batch_table(&self) -> Result<Html> {
        if !self.can_reimburse {
            return render(
                &EmptyState::missing("Hand-off batches need the reimbursement permission.", "")
                    .id("claim-batches-forbidden"),
            );
        }
        let rows = self
            .batches
            .iter()
            .map(|batch| {
                let mut actions = render(&Button {
                    id: &format!("claim-batch-download-{}", batch.id),
                    href: &format!(
                        "/people/claims/handoff/{}/{}",
                        self.company.as_ref().map_or(0, |c| c.id),
                        batch.id
                    ),
                    body: Html::text(&format!("Download claim-handoff-{}.csv", batch.id)),
                    ..Button::default()
                })?;
                actions.push(&action(
                    "reimburse_batch",
                    "Mark reimbursed",
                    &batch.id.to_string(),
                )?);
                let mut row = FlexRow::new(format!("claim-batch-{}", batch.id))
                    .slot("batch", Html::text(&format!("Batch {}", batch.id)))
                    .slot("count", Html::text(&batch.request_count.to_string()))
                    .slot(
                        "total",
                        Html::text(&format!("{} {}", batch.total_amount, batch.currency)),
                    )
                    .slot(
                        "created",
                        self.moment(
                            format!("claim-batch-created-{}", batch.id),
                            Some(&batch.created_at),
                        )?,
                    );
                row.actions = actions;
                Ok(row)
            })
            .collect::<Result<Vec<_>>>()?;
        let mut table = FlexTable::new(
            "claim-batches-list",
            self.batches_columns.flex_columns(),
            rows,
        );
        table.event = "batches_grid";
        table.zoom = self.batches_columns.zoom;
        table.suggestions = self.batches_columns.flex_suggestions();
        table.add_query = &self.batches_columns.add_query;
        table.actions = true;
        table.empty = Some(TableEmpty::State(
            EmptyState::missing(
                "No hand-off batches have been created for this company.",
                "",
            )
            .id("claim-batches-empty"),
        ));
        render(&table)
    }
    fn waiting_table(&self) -> Result<Html> {
        if !self.can_review || !self.can_reimburse || self.waiting.is_empty() {
            return Ok(Html::EMPTY);
        }
        let rows = self
            .waiting
            .iter()
            .map(|w| {
                let mut row = FlexRow::new(&w.currency)
                    .slot("currency", Html::text(&w.currency))
                    .slot("count", Html::text(&w.request_count.to_string()))
                    .slot("total", Html::text(&w.total_amount.to_string()));
                row.actions = action(
                    "handoff",
                    &format!("Create {} hand-off batch", w.currency),
                    &w.currency,
                )?;
                Ok(row)
            })
            .collect::<Result<Vec<_>>>()?;
        let mut table = FlexTable::new(
            "claim-handoff-waiting",
            self.waiting_columns.flex_columns(),
            rows,
        );
        table.event = "waiting_grid";
        table.zoom = self.waiting_columns.zoom;
        table.suggestions = self.waiting_columns.flex_suggestions();
        table.add_query = &self.waiting_columns.add_query;
        table.actions = true;
        render(&table)
    }
    fn dialog(&self) -> Result<Html> {
        if self.pending.is_some() {
            return Ok(Html::EMPTY);
        }
        let Some(selection) = &self.selection else {
            return Ok(Html::EMPTY);
        };
        let field = |key, label, kind| {
            Field::in_form("decision", key)
                .id(format!("claim-{key}"))
                .label(label)
                .kind(kind)
                .value(self.value(key))
        };
        let fields = match selection.action {
            Action::Approve => vec![
                field("approved_amount", "Approved amount", InputKind::Text)
                    .attr("inputmode", "decimal")
                    .hint(match &selection.subject {
                        Subject::Claim(row) => format!(
                            "Leave blank to approve {} {}.",
                            row.request.amount, row.request.currency
                        ),
                        _ => String::new(),
                    }),
                field("decision_reason", "Reason", InputKind::Textarea).attr("maxlength", "500"),
            ],
            Action::Reject => vec![field("decision_reason", "Reason", InputKind::Textarea)
                .required()
                .attr("maxlength", "500")],
            _ => vec![
                field("payment_reference", "Payment reference", InputKind::Text)
                    .attr("maxlength", "100"),
            ],
        };
        let title = match &selection.subject {
            Subject::Claim(row) => format!("{} claim {}", selection.action.label(), row.request.id),
            Subject::Batch(batch) => format!("Mark batch {} reimbursed", batch.id),
            _ => String::new(),
        };
        let mut actions = render(&Button {
            button_type: "submit",
            variant: ButtonVariant::Primary,
            body: Html::text("Review"),
            ..Button::default()
        })?;
        actions.push(&action("cancel", "Cancel", "")?);
        let mut body = match &self.instance {
            Some(instance) => instance
                .banner("claim-operations-instance", "mb-4")
                .map_err(|_| Error::Internal)?,
            None => Html::EMPTY,
        };
        body.push(&render(&DecisionForm { fields, actions })?);
        render(&Modal {
            id: "claim-operations-reason",
            title: &title,
            cancel_event: "cancel",
            flash: Some(self.flash()),
            body,
            ..Modal::default()
        })
    }
    fn confirmation(&self) -> Result<Html> {
        let Some(p) = &self.pending else {
            return Ok(Html::EMPTY);
        };
        let (consequence, detail, confirm, working) =
            match (&p.selection.action, &p.selection.subject) {
                (Action::Approve, Subject::Claim(row)) => (
                    format!(
                        "Claim {} of {} will be approved for {} {}.",
                        row.request.id,
                        row.employee_name.as_deref().unwrap_or("—"),
                        p.values
                            .get("approved_amount")
                            .filter(|s| !s.trim().is_empty())
                            .cloned()
                            .unwrap_or_else(|| row.request.amount.to_string()),
                        row.request.currency
                    ),
                    "The decision is recorded with your name and cannot be undone.",
                    "Approve",
                    "Approving…",
                ),
                (Action::Reject, Subject::Claim(row)) => (
                    format!(
                        "Claim {} of {} will be rejected.",
                        row.request.id,
                        row.employee_name.as_deref().unwrap_or("—")
                    ),
                    "The employee sees your reason. This cannot be undone.",
                    "Reject",
                    "Rejecting…",
                ),
                (Action::Reimburse, Subject::Claim(row)) => (
                    format!(
                        "Claim {} will be marked reimbursed for {} {}.",
                        row.request.id,
                        money(row.request.approved_amount),
                        row.request.currency
                    ),
                    "Record this only after payment. This cannot be undone.",
                    "Mark reimbursed",
                    "Saving…",
                ),
                (_, Subject::Currency(currency)) => (
                    format!(
                        "All approved {currency} claims not yet handed off will join one batch."
                    ),
                    "The batch is a permanent record of who handed them off and when.",
                    "Create batch",
                    "Creating…",
                ),
                (_, Subject::Batch(batch)) => (
                    format!(
                        "Approved claims in batch {} will be marked reimbursed.",
                        batch.id
                    ),
                    "Record this only after payment. This cannot be undone.",
                    "Mark reimbursed",
                    "Saving…",
                ),
                _ => return Ok(Html::EMPTY),
            };
        // ConfirmDialog has no banner slot. Keep its shared anatomy and include
        // the authoritative installation notice in the confirmation detail.
        let detail = self.instance.as_ref().map_or_else(
            || detail.to_owned(),
            |instance| format!("{} {detail}", instance.notice()),
        );
        render(&ConfirmDialog {
            id: "claim-operations-confirm",
            consequence: &consequence,
            detail: &detail,
            confirm,
            working,
            confirm_event: "confirm",
            cancel_event: "cancel",
        })
    }
    fn claim_actions(&self, row: &QueueClaim) -> Result<Html> {
        let r = &row.request;
        let mut body = Html::EMPTY;
        if self.can_review && Some(r.employee_id) != self.submission.own_id() {
            if r.status == "submitted" {
                body.push(&action("approve", "Approve", &r.id.to_string())?);
                body.push(&action("reject", "Reject", &r.id.to_string())?);
            }
            if r.status == "approved" && self.can_reimburse {
                body.push(&action("reimburse", "Mark reimbursed", &r.id.to_string())?);
            }
        }
        if r.status == "submitted" && Some(r.employee_id) == self.submission.own_id() {
            body.push(&action("withdraw_claim", "Withdraw", &r.id.to_string())?);
        }
        Ok(body)
    }
    fn render(&self) -> Result<String> {
        let mut controls = vec![
            ToolbarControl::select(
                "claims-audience",
                "audience",
                "Audience",
                &self.list.filters["audience"],
                if self.can_review {
                    vec![Choice::new("Mine", "mine"), Choice::new("Team", "team")]
                } else {
                    vec![Choice::new("Mine", "mine")]
                },
            ),
            ToolbarControl::select(
                "claims-status",
                "status",
                "Status",
                self.tab(),
                std::iter::once(Choice::new("All statuses", ""))
                    .chain(crate::REQUEST_STATUSES.iter().map(|s| Choice::new(*s, *s)))
                    .collect(),
            ),
        ];
        if self.can_review && self.list.filters["audience"] == "team" {
            controls.push(ToolbarControl::select(
                "claims-employee",
                "employee_id",
                "Employee",
                &self.list.filters["employee_id"],
                std::iter::once(Choice::new("All employees", ""))
                    .chain(self.employees.iter().map(|employee| {
                        Choice::new(
                            format!("{} ({})", employee.name, employee.employee_number),
                            employee.id.to_string(),
                        )
                    }))
                    .collect(),
            ));
        }
        View {
            flash: self.flash(),
            available: self.company.is_some(),
            filters: render(&FilterToolbar::new("claims-filters", controls))?,
            table: if self.record.is_some() {
                self.record_view()?
            } else {
                self.queue()?
            },
            empty: render(&EmptyState::missing(
                "No company is available for claims.",
                "",
            ))?,
            submission: if self.record.is_none() {
                self.submission.render()?
            } else {
                Html::EMPTY
            },
            waiting: if self.record.is_none() {
                self.waiting_table()?
            } else {
                Html::EMPTY
            },
            batches: if self.record.is_none() && self.can_review && self.can_reimburse {
                self.batch_table()?
            } else {
                Html::EMPTY
            },
            truncated: self.record.is_none()
                && self.list.filters["audience"] == "team"
                && self.rows.len() >= 500,
            detail: self.record.is_some(),
            record_id: self.record.map(|id| id.to_string()).unwrap_or_default(),
            dialog: self.dialog()?,
            confirmation: self.confirmation()?,
            withdrawal: if self.record.is_some() {
                self.submission.confirmation()?
            } else {
                Html::EMPTY
            },
        }
        .render()
        .map_err(|_| Error::Internal)
    }
    fn record_view(&self) -> Result<Html> {
        let row = self.rows.first().ok_or(Error::NotFound)?;
        let r = &row.request;
        let facts = vec![
            bilibi_ui::Fact {
                title: "Employee",
                value: Html::text(row.employee_name.as_deref().unwrap_or("—")),
                ..Default::default()
            },
            bilibi_ui::Fact {
                title: "Claim type",
                value: Html::text(&row.claim_type_name),
                ..Default::default()
            },
            bilibi_ui::Fact {
                title: "Expense date",
                value: Html::text(&r.incurred_on.to_string()),
                ..Default::default()
            },
            bilibi_ui::Fact {
                title: "Amount",
                value: Html::text(&format!("{} {}", r.amount, r.currency)),
                ..Default::default()
            },
            bilibi_ui::Fact {
                title: "Approved amount",
                value: Html::text(&money(r.approved_amount)),
                ..Default::default()
            },
            bilibi_ui::Fact {
                title: "Status",
                value: Html::text(&r.status),
                ..Default::default()
            },
            bilibi_ui::Fact {
                title: "Receipt",
                value: Html::text(r.receipt_number.as_deref().unwrap_or("—")),
                ..Default::default()
            },
            bilibi_ui::Fact {
                title: "Description",
                value: Html::text(r.description.as_deref().unwrap_or("—")),
                ..Default::default()
            },
            bilibi_ui::Fact {
                title: "Decision",
                value: Html::text(r.decision_reason.as_deref().unwrap_or("—")),
                ..Default::default()
            },
            bilibi_ui::Fact {
                title: "Reimbursed",
                value: self.moment(format!("claim-paid-{}", r.id), r.reimbursed_at.as_deref())?,
                ..Default::default()
            },
            bilibi_ui::Fact {
                title: "Payment reference",
                value: Html::text(r.payment_reference.as_deref().unwrap_or("—")),
                ..Default::default()
            },
        ];
        let mut body = render(&bilibi_ui::Facts {
            items: facts,
            ..Default::default()
        })?;
        body.push(&self.claim_actions(row)?);
        Ok(body)
    }
}

impl LiveState<Scope, Error> for Operations {
    fn update<'a>(&'a mut self, request: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            match request.input {
                LiveInput::Refresh => {}
                LiveInput::Params(query) => {
                    self.clear();
                    self.list = ListState::parse(query, spec()?).map_err(|_| Error::Internal)?;
                    self.normalize();
                    self.columns = queue_columns(self.tab())?;
                }
                LiveInput::Event(payload) => {
                    self.load(request.scope).await?;
                    let kind = payload.get("type").and_then(Value::as_str).unwrap_or("");
                    if matches!(
                        kind,
                        "submit_claim"
                            | "withdraw_claim"
                            | "confirm_withdraw"
                            | "cancel_withdraw"
                            | "open_types_grid"
                    ) {
                        self.submission
                            .event(request.scope, request.credential, payload)
                            .await?;
                        if let Some(message) = self.submission.take_flash() {
                            self.flash = Some(message);
                        }
                    } else {
                        self.event(request.scope, request.credential, payload)
                            .await?;
                    }
                }
            }
            self.load(request.scope).await?;
            Ok(LiveBody {
                html: self.render()?,
                query: self.list.to_params(),
                rows: self.rows.len(),
                ..LiveBody::default()
            })
        })
    }
}
fn selection_refusal(event: &str) -> &'static str {
    match event {
        "approve" | "reject" => "This claim is no longer awaiting a decision.",
        "reimburse" => "This claim cannot be reimbursed.",
        "handoff" => "You may not hand off claims.",
        "reimburse_batch" => "This batch cannot be reimbursed.",
        _ => FORBIDDEN,
    }
}
fn money(value: Option<Decimal>) -> String {
    value.map_or_else(|| "—".into(), |v| v.to_string())
}
fn render(template: &impl Template) -> Result<Html> {
    Html::render(template).map_err(|_| Error::Internal)
}
fn action(event: &str, label: &str, value: &str) -> Result<Html> {
    render(&Button {
        button_type: "button",
        event,
        name: "value",
        value,
        body: Html::text(label),
        ..Button::default()
    })
}
#[derive(Template)]
#[template(path = "operations.html")]
struct View<'a> {
    flash: FlashMap<'a>,
    available: bool,
    filters: Html,
    empty: Html,
    table: Html,
    submission: Html,
    waiting: Html,
    batches: Html,
    truncated: bool,
    detail: bool,
    record_id: String,
    dialog: Html,
    confirmation: Html,
    withdrawal: Html,
}
#[derive(Template)]
#[template(path = "decision_form.html")]
struct DecisionForm {
    fields: Vec<Field>,
    actions: Html,
}
#[derive(Template)]
#[template(
    source = "{% import \"lists.html\" as lists %}{% call lists::record_link(workspace=workspace, kind=\"people/claims\", record_id=row.request.id.to_string(), href=href) %}{{ row.claim_type_name }}{% endcall %}<div> <span class=\"text-ink-muted\">· {{ row.category_name }}</span></div>{% if !compact %}{% if let Some(description) = row.request.description %}<div class=\"text-xs text-ink-muted\">{{ description }}</div>{% endif %}{% if row.request.duplicate_confirmed %}<div class=\"text-xs text-warning-ink\">Submitter confirmed a similar claim.</div>{% endif %}{% endif %}",
    ext = "html"
)]
struct ClaimCell<'a> {
    row: &'a QueueClaim,
    compact: bool,
    workspace: bool,
    href: String,
}
#[derive(Template)]
#[template(
    source = "{% import \"flex_table.html\" as table %}{% call table::instant(id=id, i=instant) %}{% endcall %}",
    ext = "html"
)]
struct Moment {
    id: String,
    instant: bilibi_ui::flex_table::Instant,
}
