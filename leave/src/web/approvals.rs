//! Bilimbi `Leave.Web.ApprovalsLive`: independently decide pending requests.
use super::{dates, refusal, render, text, type_name, unit, Core, Flash, Table};
use crate::{Decision, LeaveError, LeaveType, PendingRequest, REQUESTS_APPROVE};
use askama::Template;
use bilibi_database::{Error, Result};
use bilibi_people_workforce::{authorization, CompanySummary};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    choices,
    page::{LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState},
    params, Button, ButtonVariant, ConfirmDialog, EmptyState, Field, FlashMap, FlexRow, Form, Html,
    InputKind,
};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

const NO_PERMISSION: &str =
    "You no longer have permission to decide leave requests for this company.";
#[derive(Template)]
#[template(path = "approvals.html")]
struct View<'a> {
    flash: FlashMap<'a>,
    subtitle: String,
    has_company: bool,
    company: Field,
    state: Html,
    table: Html,
    confirm: Html,
}
struct PendingDecision {
    company: i64,
    request: i64,
    decision: Decision,
    note: String,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Availability {
    Current,
    Unavailable,
    Unauthorized,
}
pub(super) struct Approvals {
    core: Arc<Core>,
    company_id: Option<i64>,
    companies: Vec<CompanySummary>,
    company: Option<CompanySummary>,
    requests: Vec<PendingRequest>,
    types: Vec<LeaveType>,
    availability: Availability,
    pending: Option<PendingDecision>,
    notes: HashMap<i64, String>,
    table: Table,
    flash: Flash,
}
impl Approvals {
    pub(super) fn new(core: Arc<Core>, query: &HashMap<String, String>) -> Result<Self> {
        Ok(Self {
            core,
            company_id: params::positive_id(query.get("company_id").map(String::as_str)),
            companies: vec![],
            company: None,
            requests: vec![],
            types: vec![],
            availability: Availability::Unavailable,
            pending: None,
            notes: HashMap::new(),
            flash: Flash::default(),
            table: Table::new(
                "leave-approvals",
                "approvals_grid",
                &[
                    ("employee", "Employee"),
                    ("type", "Leave type"),
                    ("dates", "Dates"),
                    ("quantity", "Quantity"),
                    ("reason", "Reason"),
                    ("decision", "Decision"),
                ],
            )?,
        })
    }
    pub(super) async fn load(&mut self, scope: &Scope, rows_changed: bool) -> Result<()> {
        let client = self.core.db.get().await?;
        self.companies =
            match authorization::selectable_companies(&**client, scope, REQUESTS_APPROVE).await {
                Ok(companies) => companies.into_iter().filter(|c| !c.archived).collect(),
                Err(error) => {
                    refusal(error.into())?;
                    vec![]
                }
            };
        // Choose the first active company only at mount. Losing an existing
        // company's reach must withdraw its records, never switch a confirmation.
        if self.company_id.is_none() {
            self.company_id = self.companies.first().map(|c| c.id);
        }
        self.company = self
            .companies
            .iter()
            .find(|c| Some(c.id) == self.company_id)
            .cloned();
        self.requests.clear();
        self.types.clear();
        let result = match &self.company {
            Some(company) => {
                async {
                    Ok::<_, LeaveError>((
                        self.core.leave.pending_requests(scope, company.id).await?,
                        self.core.leave.list_types(scope, company.id).await?,
                    ))
                }
                .await
            }
            None => Err(LeaveError::Unauthorized),
        };
        match result {
            Ok((requests, types)) => {
                self.availability = Availability::Current;
                if rows_changed {
                    self.table.load(&requests, |r| r.request.id.to_string());
                }
                self.requests = requests;
                self.types = types;
            }
            Err(error) => {
                self.availability = if matches!(refusal(error)?, LeaveError::Unauthorized) {
                    Availability::Unauthorized
                } else {
                    Availability::Unavailable
                };
                self.pending = None;
                self.notes.clear();
            }
        }
        Ok(())
    }
    async fn authorized(&mut self, scope: &Scope) -> Result<Option<i64>> {
        let Some(company) = self.company_id else {
            self.flash.error(NO_PERMISSION);
            self.pending = None;
            return Ok(None);
        };
        let client = self.core.db.get().await?;
        match authorization::authorize_company(&**client, scope, company, REQUESTS_APPROVE).await {
            Ok(c) if !c.archived => Ok(Some(company)),
            Ok(_) => {
                self.pending = None;
                self.flash.error("This company is archived and read-only");
                Ok(None)
            }
            Err(error) => {
                refusal(error.into())?;
                self.pending = None;
                self.flash.error(NO_PERMISSION);
                Ok(None)
            }
        }
    }
    async fn event(&mut self, scope: &Scope, payload: &Value) -> Result<bool> {
        match text(payload, "type").as_str() {
            "clear-flash" => self.flash.clear(payload),
            "approvals_grid" => self.table.event(payload),
            "dismiss_decision" => self.pending = None,
            "select_company" => {
                let chosen = params::positive_id(payload.get("company_id").and_then(Value::as_str));
                self.load(scope, false).await?;
                if self.companies.iter().any(|c| Some(c.id) == chosen) {
                    self.company_id = chosen;
                    self.pending = None;
                    self.notes.clear();
                }
                return Ok(true);
            }
            "decide" => {
                let Some(company) = self.authorized(scope).await? else {
                    return Ok(true);
                };
                let id = params::positive_id(payload.get("request_id").and_then(Value::as_str));
                let note = text(payload, "note");
                let decision = match text(payload, "decision").as_str() {
                    "approve" => Decision::Approve,
                    "reject" => Decision::Reject,
                    _ => {
                        self.flash.error("The request could not be decided.");
                        return Ok(true);
                    }
                };
                self.load(scope, false).await?;
                if let Some(r) = self.requests.iter().find(|r| Some(r.request.id) == id) {
                    let id = r.request.id;
                    self.notes.insert(id, note.clone());
                    if decision == Decision::Reject && note.trim().is_empty() {
                        self.flash.error("Give a reason when rejecting a request.");
                    } else if note.chars().count() > 500 {
                        self.flash.error(
                            "Choose approve or reject and a note of at most 500 characters.",
                        );
                    } else {
                        self.pending = Some(PendingDecision {
                            company,
                            request: id,
                            decision,
                            note,
                        });
                    }
                } else {
                    self.flash.error("That request has already been decided.");
                }
                return Ok(true);
            }
            "confirm_decision" => {
                let Some(company) = self.authorized(scope).await? else {
                    return Ok(true);
                };
                if let Some(pending) = self.pending.take() {
                    if pending.company != company {
                        self.flash.error("The request could not be decided.");
                        return Ok(true);
                    }
                    match self
                        .core
                        .leave
                        .decide_request(
                            scope,
                            company,
                            pending.request,
                            pending.decision,
                            Some(&pending.note),
                        )
                        .await
                    {
                        Ok(_) => {
                            self.notes.remove(&pending.request);
                            self.flash
                                .success(if pending.decision == Decision::Approve {
                                    "Leave request approved."
                                } else {
                                    "Leave request rejected."
                                });
                        }
                        Err(error) => self.flash.error(decision_refusal(&refusal(error)?)),
                    }
                }
                return Ok(true);
            }
            _ => return Err(Error::Invalid),
        }
        Ok(false)
    }
    pub(super) fn render(&self) -> Result<String> {
        let options: Vec<_> = self
            .companies
            .iter()
            .map(|c| (c.name.clone(), c.id.to_string()))
            .collect();
        let mut view = View {
            flash: self.flash.map(),
            subtitle: self
                .company
                .as_ref()
                .map_or_else(String::new, |c| format!("{} · Pending requests", c.name)),
            has_company: !self.companies.is_empty(),
            company: Field::named("company_id")
                .id("leave-approvals-company")
                .label("Company")
                .kind(InputKind::Select)
                .options(choices(&options))
                .value(
                    self.company_id
                        .map_or_else(String::new, |id| id.to_string()),
                ),
            state: Html::EMPTY,
            table: Html::EMPTY,
            confirm: Html::EMPTY,
        };
        if self.companies.is_empty() {
            view.state = render(
                &EmptyState::missing("No active company is available for leave approvals.", "")
                    .id("leave-approvals-empty"),
            )?;
        } else if self.availability != Availability::Current {
            let (id, reason) = if self.availability == Availability::Unauthorized {
                (
                    "leave-approvals-forbidden",
                    "You no longer have permission to decide leave requests here.",
                )
            } else {
                (
                    "leave-approvals-unavailable",
                    "Workforce data for this company is not current.",
                )
            };
            view.state = render(
                &EmptyState::missing("Leave requests are unavailable for this company.", reason)
                    .id(id),
            )?;
        } else {
            let mut rows = vec![];
            for r in &self.requests {
                let request = &r.request;
                let note = Field::named("note")
                    .id(format!("leave-note-{}", request.id))
                    .label("Decision note")
                    .placeholder("Note (required to reject)")
                    .attr("maxlength", "500")
                    .value(self.notes.get(&request.id).cloned().unwrap_or_default());
                let mut fields = render(
                    &Field::named("request_id")
                        .kind(InputKind::Hidden)
                        .value(request.id.to_string()),
                )?;
                fields.push(&render(&note)?);
                let mut buttons = Html::EMPTY;
                for (decision, label) in [("approve", "Approve"), ("reject", "Reject")] {
                    buttons.push(&render(&Button {
                        button_type: "submit",
                        name: "decision",
                        value: decision,
                        variant: if decision == "approve" {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Secondary
                        },
                        body: Html::text(label),
                        ..Button::default()
                    })?);
                }
                let form = render(&Form {
                    id: &format!("leave-decision-{}", request.id),
                    event: "decide",
                    change: false,
                    class: "",
                    body: fields,
                    actions: buttons,
                })?;
                rows.push(
                    FlexRow::new(format!("leave-approval-{}", request.id))
                        .slot(
                            "employee",
                            Html::text(
                                &r.employee_name
                                    .clone()
                                    .unwrap_or_else(|| format!("Employee {}", request.employee_id)),
                            ),
                        )
                        .slot(
                            "type",
                            Html::text(type_name(&self.types, request.leave_type_id)),
                        )
                        .slot("dates", Html::text(&dates(request)))
                        .slot(
                            "quantity",
                            Html::text(&format!("{} {}", request.quantity, unit(&request.unit))),
                        )
                        .slot(
                            "reason",
                            Html::text(request.reason.as_deref().unwrap_or_default()),
                        )
                        .slot("decision", form),
                );
            }
            view.table = self.table.render(
                "Pending requests",
                rows,
                EmptyState::missing("No leave requests are waiting for a decision.", "")
                    .id("leave-approvals-none"),
            )?;
            if let Some(pending) = &self.pending {
                if let Some(r) = self
                    .requests
                    .iter()
                    .find(|r| r.request.id == pending.request)
                {
                    let approve = pending.decision == Decision::Approve;
                    view.confirm = render(&ConfirmDialog {
                        id: "leave-decision-confirm",
                        consequence: if approve {
                            "Approve this leave request?"
                        } else {
                            "Reject this leave request?"
                        },
                        detail: &format!(
                            "{} · {} · {} · {}",
                            r.employee_name.as_deref().unwrap_or("Employee"),
                            type_name(&self.types, r.request.leave_type_id),
                            dates(&r.request),
                            pending.note
                        ),
                        confirm: if approve { "Approve" } else { "Reject" },
                        working: "Deciding…",
                        confirm_event: "confirm_decision",
                        cancel_event: "dismiss_decision",
                    })?;
                }
            }
        }
        view.render().map_err(|_| Error::Internal)
    }
}
impl LiveState<Scope, Error> for Approvals {
    fn update<'a>(&'a mut self, request: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            let changed = match request.input {
                LiveInput::Event(payload) => self.event(request.scope, payload).await?,
                LiveInput::Params(params) => {
                    self.company_id =
                        params::positive_id(params.get("company_id").map(String::as_str));
                    self.pending = None;
                    self.notes.clear();
                    true
                }
                LiveInput::Refresh => true,
            };
            self.load(request.scope, changed).await?;
            let query = self
                .company_id
                .map(|id| HashMap::from([("company_id".into(), id.to_string())]))
                .unwrap_or_default();
            Ok(LiveBody {
                html: self.render()?,
                rows: self.requests.len(),
                query,
                ..LiveBody::default()
            })
        })
    }
}
fn decision_refusal(error: &LeaveError) -> &str {
    match error {
        LeaveError::NoteRequired => "Give a reason when rejecting a request.",
        LeaveError::SelfApproval => "You cannot decide your own leave request.",
        LeaveError::InsufficientBalance => {
            "The employee's available balance no longer covers this request."
        }
        LeaveError::YearClosed => "That leave year has been carried forward and is closed.",
        LeaveError::NotPending => "That request has already been decided.",
        LeaveError::Unauthorized => NO_PERMISSION,
        LeaveError::CompanyArchived => "This company is archived and read-only",
        _ => "The request could not be decided.",
    }
}
