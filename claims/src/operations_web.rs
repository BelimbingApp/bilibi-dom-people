//! Bilimbi Claims.Web.OperationsLive: decisions, reimbursement and hand-off.
use crate::{
    Approval, Claims, ClaimsError, HandoffBatch, HandoffWaiting, QueueClaim, Reimbursement,
    Rejection, APPROVE, REIMBURSE,
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
    Modal, StatusKind, Tab, TableEmpty, Tabs, ToolbarControl, NO_FLASH,
};
use rust_decimal::Decimal;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

const PATH: &str = "/people/claims/operations";
const TABS: [&str; 5] = ["submitted", "approved", "reimbursed", "rejected", "batches"];
const FORBIDDEN: &str = "You are not allowed to do that for this company.";
struct Core {
    db: Db,
    claims: Arc<Claims>,
}
pub(crate) struct Pages(Arc<Core>);
impl Pages {
    pub(crate) fn new(db: Db, claims: Arc<Claims>) -> Self {
        Self(Arc::new(Core { db, claims }))
    }
}
impl Page<Scope, Error> for Pages {
    fn title(&self, _: &str) -> Option<&'static str> {
        Some("Claim operations")
    }
    fn render<'a>(&'a self, request: PageRequest<'a, Scope>) -> PageFuture<'a, Error> {
        Box::pin(async move {
            let mut state = Operations::new(self.0.clone(), request.query)?;
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
                Box::new(Operations::new(self.0.clone(), request.query)?)
                    as Box<dyn LiveState<Scope, Error>>,
            ))
        })
    }
}
fn spec() -> Result<Arc<ListSpec>> {
    ListSpec {
        filters: BTreeMap::from([
            ("company_id".into(), Filter::string("")),
            ("tab".into(), Filter::string("submitted")),
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
fn queue_columns(tab: &str) -> Result<ListColumns> {
    let mut fields = vec![
        ("employee", "Employee"),
        ("claim", "Claim"),
        ("date", "Date"),
        ("claimed", "Claimed"),
    ];
    if tab != "submitted" {
        fields.push(("approved", "Approved"));
    }
    fields.push(("receipt", "Receipt"));
    if matches!(tab, "approved" | "reimbursed") {
        fields.push(("batch", "Batch"));
    }
    if tab == "rejected" {
        fields.push(("reason", "Reason"));
    }
    if tab == "reimbursed" {
        fields.push(("paid", "Paid"));
    }
    columns("claim-queue-table", &fields)
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
    rows: Vec<QueueClaim>,
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
        let mut state = Self {
            core,
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
        &self.list.filters["tab"]
    }
    fn normalize(&mut self) {
        if !TABS.contains(&self.tab()) {
            self.list.filters.insert("tab".into(), "submitted".into());
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
        self.companies = match authorization::selectable_companies(&**client, scope, APPROVE).await
        {
            Ok(rows) => rows,
            Err(bilibi_people_workforce::WorkforceError::Failed(e)) => return Err(e),
            Err(_) => vec![],
        };
        self.company = self
            .companies
            .iter()
            .find(|c| c.id.to_string() == self.list.filters["company_id"])
            .or_else(|| self.companies.first())
            .cloned();
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
        self.rows.clear();
        self.batches.clear();
        self.waiting.clear();
        if let Some(c) = &self.company {
            let company = c.id;
            self.list
                .filters
                .insert("company_id".into(), company.to_string());
            let loaded = async {
                if self.tab() == "batches" {
                    self.batches = self.core.claims.handoff_batches(scope, company).await?;
                } else {
                    self.rows = self
                        .core
                        .claims
                        .claim_queue(scope, company, self.tab())
                        .await?;
                    if self.tab() == "approved" {
                        self.waiting = self.core.claims.handoff_waiting(scope, company).await?;
                    }
                }
                Ok::<_, ClaimsError>(())
            }
            .await;
            if let Err(error) = loaded {
                self.company = None;
                self.rows.clear();
                self.batches.clear();
                self.waiting.clear();
                self.can_reimburse = false;
                self.refused(error);
            }
        }
        let selected = self
            .pending
            .as_ref()
            .map(|p| &p.selection)
            .or(self.selection.as_ref());
        if selected.is_some_and(|selection| {
            self.company.as_ref().map(|c| c.id) != Some(selection.company)
                || selection.action.capability() == REIMBURSE && !self.can_reimburse
        }) {
            self.clear();
            self.message(StatusKind::Error, FORBIDDEN);
        }
        Ok(())
    }
    async fn permitted(&mut self, scope: &Scope, capability: &str) -> Result<bool> {
        let company = self
            .pending
            .as_ref()
            .map(|p| p.selection.company)
            .or_else(|| self.selection.as_ref().map(|s| s.company))
            .or_else(|| self.company.as_ref().map(|c| c.id));
        let Some(company) = company else {
            self.clear();
            self.message(StatusKind::Error, FORBIDDEN);
            return Ok(false);
        };
        let client = self.core.db.get().await?;
        let allowed = authorization::allowed(&**client, scope, company, APPROVE).await
            && authorization::allowed(&**client, scope, company, capability).await;
        if !allowed {
            self.clear();
            self.message(StatusKind::Error, FORBIDDEN);
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
    async fn event(&mut self, scope: &Scope, payload: &Value) -> Result<()> {
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
            if !self.permitted(scope, capability).await? {
                return Ok(());
            }
        }
        match event {
            "clear-flash" => self.flash = None,
            "cancel" => self.clear(),
            "filters" => {
                self.clear();
                self.list
                    .apply_filters(&params::form_values(payload, "filters"));
                // Bilimbi's company patch opens the awaiting-decision queue.
                self.list.filters.insert("tab".into(), "submitted".into());
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
                    self.message(StatusKind::Error, "That record is no longer available.");
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
                    let reason = values.get("decision_reason").cloned();
                    let payment = Reimbursement {
                        payment_reference: values.get("payment_reference").cloned(),
                    };
                    let claims = &self.core.claims;
                    let (result, message) = match (selection.action, selection.subject) {
                        (Action::Approve, Subject::Claim(row)) => (
                            claims
                                .approve_request(
                                    scope,
                                    company,
                                    row.request.id,
                                    &Approval {
                                        approved_amount: values
                                            .get("approved_amount")
                                            .filter(|s| !s.trim().is_empty())
                                            .and_then(|s| s.parse().ok()),
                                        decision_reason: reason,
                                    },
                                )
                                .await
                                .map(|_| ()),
                            "Claim approved.",
                        ),
                        (Action::Reject, Subject::Claim(row)) => (
                            claims
                                .reject_request(
                                    scope,
                                    company,
                                    row.request.id,
                                    &Rejection {
                                        decision_reason: reason.unwrap_or_default(),
                                    },
                                )
                                .await
                                .map(|_| ()),
                            "Claim rejected.",
                        ),
                        (Action::Reimburse, Subject::Claim(row)) => (
                            claims
                                .reimburse_request(scope, company, row.request.id, &payment)
                                .await
                                .map(|_| ()),
                            "Claim marked reimbursed.",
                        ),
                        (_, Subject::Currency(currency)) => (
                            claims
                                .create_handoff_batch(scope, company, &currency)
                                .await
                                .map(|_| ()),
                            "Hand-off batch created. Open it under Batches to export it.",
                        ),
                        (Action::Batch, Subject::Batch(batch)) => (
                            claims
                                .reimburse_batch(scope, company, batch.id, &payment)
                                .await
                                .map(|_| ()),
                            "Batch claims marked reimbursed.",
                        ),
                        _ => return Ok(()),
                    };
                    match result {
                        Ok(()) => self.message(StatusKind::Success, message),
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
                let mut actions = Html::EMPTY;
                if self.tab() == "submitted" {
                    actions.push(&action("approve", "Approve", &r.id.to_string())?);
                    actions.push(&action("reject", "Reject", &r.id.to_string())?);
                } else if self.tab() == "approved" && self.can_reimburse {
                    actions.push(&action("reimburse", "Mark reimbursed", &r.id.to_string())?);
                }
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
                    .slot("claim", render(&ClaimCell { row, compact })?)
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
        table.actions = self.tab() == "submitted" || self.tab() == "approved" && self.can_reimburse;
        table.empty = Some(TableEmpty::State(
            EmptyState::missing(
                format!("No claims are {}.", label(self.tab()).to_lowercase()),
                "",
            )
            .id("claim-queue-empty"),
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
        if self.tab() != "approved" || !self.can_reimburse || self.waiting.is_empty() {
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
    fn render(&self) -> Result<String> {
        let filters = render(&FilterToolbar::new(
            "claim-operations-company",
            vec![ToolbarControl::select(
                "claim-operations-company-select",
                "company_id",
                "Company",
                &self.list.filters["company_id"],
                self.companies
                    .iter()
                    .map(|c| Choice::new(&c.name, c.id.to_string()))
                    .collect(),
            )],
        ))?;
        let links: Vec<_> = TABS
            .iter()
            .map(|tab| {
                let mut query = self.list.to_params();
                query.insert("tab".into(), (*tab).into());
                params::address(PATH, &query)
            })
            .collect();
        let ids: Vec<_> = TABS.iter().map(|tab| format!("claim-tab-{tab}")).collect();
        let tabs = render(&Tabs {
            id: "claim-operations-tabs",
            label: "Claim queues",
            tabs: TABS
                .iter()
                .zip(&links)
                .zip(&ids)
                .map(|((tab, href), id)| Tab {
                    id,
                    ..Tab::patch(href, label(tab), self.tab() == *tab)
                })
                .collect(),
            ..Tabs::default()
        })?;
        View {
            flash: self.flash(),
            available: self.company.is_some(),
            filters,
            tabs,
            empty: render(
                &EmptyState::missing("No company is available for claim operations.", "")
                    .id("claim-operations-empty"),
            )?,
            heading: label(self.tab()),
            description: if self.tab() == "batches" { "A batch groups approved claims of one currency for finance or payroll. Export it as a CSV, then mark it reimbursed once paid." } else { "" },
            waiting: self.waiting_table()?,
            table: if self.company.is_none() {
                Html::EMPTY
            } else if self.tab() == "batches" {
                self.batch_table()?
            } else {
                self.queue()?
            },
            truncated: self.tab() != "batches" && self.rows.len() >= 500,
            dialog: self.dialog()?,
            confirmation: self.confirmation()?,
        }
        .render()
        .map_err(|_| Error::Internal)
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
                    self.event(request.scope, payload).await?;
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
fn label(tab: &str) -> &'static str {
    match tab {
        "approved" => "Approved",
        "reimbursed" => "Reimbursed",
        "rejected" => "Rejected",
        "batches" => "Hand-off batches",
        _ => "Awaiting decision",
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
    tabs: Html,
    empty: Html,
    heading: &'a str,
    description: &'a str,
    waiting: Html,
    table: Html,
    truncated: bool,
    dialog: Html,
    confirmation: Html,
}
#[derive(Template)]
#[template(path = "decision_form.html")]
struct DecisionForm {
    fields: Vec<Field>,
    actions: Html,
}
#[derive(Template)]
#[template(
    source = "<div>{{ row.claim_type_name }} <span class=\"text-ink-muted\">· {{ row.category_name }}</span></div>{% if !compact %}{% if let Some(description) = row.request.description %}<div class=\"text-xs text-ink-muted\">{{ description }}</div>{% endif %}{% if row.request.duplicate_confirmed %}<div class=\"text-xs text-warning-ink\">Submitter confirmed a similar claim.</div>{% endif %}{% endif %}",
    ext = "html"
)]
struct ClaimCell<'a> {
    row: &'a QueueClaim,
    compact: bool,
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
