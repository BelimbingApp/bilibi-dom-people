//! Bilimbi `Attendance.Web.ApprovalsLive`: independently decide pending requests.
use super::{refusal, render, row, text, Core, Flash, Table};
use crate::{AttendanceError, PendingAdjustment, ADJUSTMENTS_APPROVE};
use askama::Template;
use bilibi_database::{Error, Result};
use bilibi_people_workforce::{authorization, CompanySummary};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    choices,
    page::{LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState},
    params, Button, ButtonVariant, ConfirmDialog, EmptyState, Field, FlashMap, Form, Html,
    InputKind,
};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

const NO_PERMISSION: &str =
    "You no longer have permission to decide attendance requests for this company.";
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
    decision: String,
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
    requests: Vec<PendingAdjustment>,
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
            availability: Availability::Unavailable,
            pending: None,
            notes: HashMap::new(),
            flash: Flash::default(),
            table: Table::new(
                "attendance-approvals",
                "approvals_grid",
                &[
                    ("employee", "Employee"),
                    ("clock", "Clock event"),
                    ("reason", "Reason"),
                    ("decision", "Decision"),
                ],
            )?,
        })
    }
    pub(super) async fn load(&mut self, scope: &Scope, rows_changed: bool) -> Result<()> {
        let client = self.core.db.get().await?;
        self.companies = match authorization::company_choices(
            &**client,
            scope,
            ADJUSTMENTS_APPROVE,
            self.company_id,
        )
        .await
        {
            Ok(companies) => companies,
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
        let result = match &self.company {
            Some(company) => {
                self.core
                    .attendance
                    .pending_adjustments(scope, company.id)
                    .await
            }
            None => Err(AttendanceError::Unauthorized),
        };
        match result {
            Ok(requests) => {
                self.availability = Availability::Current;
                if rows_changed {
                    self.table.load(&requests, |r| r.request.id.to_string());
                }
                self.requests = requests;
            }
            Err(error) => {
                self.availability = if matches!(refusal(error)?, AttendanceError::Unauthorized) {
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
        match authorization::authorize_company(&**client, scope, company, ADJUSTMENTS_APPROVE).await
        {
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
            "decision_note" => {
                if self.authorized(scope).await?.is_none() {
                    return Ok(true);
                }
                if let Some(id) =
                    params::positive_id(payload.get("request_id").and_then(Value::as_str))
                {
                    if self.requests.iter().any(|r| r.request.id == id) {
                        self.notes.insert(id, text(payload, "note"));
                    }
                }
            }
            "approve" | "reject" => {
                let Some(company) = self.authorized(scope).await? else {
                    return Ok(true);
                };
                let id = params::positive_id(payload.get("request_id").and_then(Value::as_str));
                let note = id
                    .and_then(|id| self.notes.get(&id))
                    .cloned()
                    .unwrap_or_default();
                let decision = match text(payload, "type").as_str() {
                    "approve" => "approve".to_owned(),
                    "reject" => "reject".to_owned(),
                    _ => {
                        self.flash.error("The request could not be decided.");
                        return Ok(true);
                    }
                };
                self.load(scope, false).await?;
                if let Some(r) = self.requests.iter().find(|r| Some(r.request.id) == id) {
                    let id = r.request.id;
                    self.notes.insert(id, note.clone());
                    if decision == "reject" && note.trim().is_empty() {
                        self.flash.error("Add a note explaining the rejection.");
                    } else if note.chars().count() > 500 {
                        self.flash.error("Keep the note within 500 characters.");
                    } else {
                        self.pending = Some(PendingDecision {
                            company,
                            request: id,
                            decision,
                            note,
                        });
                    }
                } else {
                    self.flash.error("This request has already been decided.");
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
                        .attendance
                        .decide_adjustment(
                            scope,
                            company,
                            pending.request,
                            &pending.decision,
                            Some(&pending.note),
                        )
                        .await
                    {
                        Ok(_) => {
                            self.notes.remove(&pending.request);
                            self.flash.success(if pending.decision == "approve" {
                                "Request approved."
                            } else {
                                "Request rejected."
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
            subtitle: "Missing clock events employees asked to add.".into(),
            has_company: !self.companies.is_empty(),
            company: Field::named("company_id")
                .id("attendance-approvals-company")
                .wrapper_class("max-w-md")
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
                &EmptyState::missing(
                    "No active company is available for attendance approvals.",
                    "",
                )
                .id("attendance-approvals-no-company"),
            )?;
        } else if self.availability != Availability::Current {
            let (id, reason) = if self.availability == Availability::Unauthorized {
                (
                    "attendance-approvals-forbidden",
                    "You no longer have permission to decide attendance requests here.",
                )
            } else {
                (
                    "attendance-approvals-unavailable",
                    "The company's workforce is not current.",
                )
            };
            view.state = render(
                &EmptyState::missing("Approvals are unavailable for this company.", reason).id(id),
            )?;
        } else {
            let mut rows = vec![];
            for r in &self.requests {
                let request = &r.request;
                let note = Field::named("note")
                    .id(format!("attendance-note-{}", request.id))
                    .label("Decision note")
                    .wrapper_class("mb-0")
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
                        button_type: "button",
                        event: decision,
                        name: "request_id",
                        value: &request.id.to_string(),
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
                    id: &format!("adjustment-decision-{}", request.id),
                    // The transport does not serialize a submitter's name/value.
                    // Shared change events keep the note on the server before
                    // either action is queued, preserving Bilimbi's two buttons.
                    event: "decision_note",
                    change: true,
                    class: "block",
                    body: fields,
                    actions: buttons,
                })?;
                rows.push(
                    row("adjustment", request.id)
                        .slot(
                            "employee",
                            Html::text(
                                r.employee_name
                                    .as_deref()
                                    .unwrap_or("Employee no longer working"),
                            ),
                        )
                        .slot("clock", Html::text(&clock_text(request)))
                        .slot("reason", Html::text(&request.reason))
                        .slot("decision", form),
                );
            }
            view.table = self.table.render(
                "Pending requests",
                rows,
                EmptyState::missing(
                    "No adjustment requests are waiting.",
                    "New requests from employees will appear here.",
                )
                .id("attendance-approvals-empty"),
            )?;
            if let Some(pending) = &self.pending {
                if let Some(r) = self
                    .requests
                    .iter()
                    .find(|r| r.request.id == pending.request)
                {
                    let approve = pending.decision == "approve";
                    view.confirm = render(&ConfirmDialog {
                        id: "attendance-decision-confirm",
                        consequence: if approve {
                            "Approve this attendance request?"
                        } else {
                            "Reject this attendance request?"
                        },
                        detail: &format!(
                            "{} · {} · {}",
                            r.employee_name.as_deref().unwrap_or("Employee"),
                            clock_text(&r.request),
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
fn decision_refusal(error: &AttendanceError) -> &str {
    match error {
        AttendanceError::NoteRequired => "Add a note explaining the rejection.",
        AttendanceError::NoteTooLong => "Keep the note within 500 characters.",
        AttendanceError::SelfApproval => "You cannot decide your own attendance request.",
        AttendanceError::NotPending => "This request has already been decided.",
        AttendanceError::EmployeeUnavailable => {
            "The employee is no longer working in this company."
        }
        AttendanceError::EventKeyConflict => "The clock event could not be recorded.",
        AttendanceError::Unauthorized => NO_PERMISSION,
        AttendanceError::CompanyArchived => "This company is archived and read-only",
        _ => "The request could not be decided.",
    }
}
fn clock_text(request: &crate::AdjustmentRequest) -> String {
    // Bilimbi pins the requested clock to its recorded attendance zone.
    let instant = request.proposed_at.parse::<jiff::Timestamp>();
    let at = instant
        .map(|at| {
            bilibi_datetime::DisplayContext {
                mode: bilibi_datetime::Mode::Company,
                timezone: request.timezone.clone(),
                ..bilibi_datetime::DisplayContext::anonymous()
            }
            .render(
                at,
                bilibi_datetime::Format::DateTime,
                bilibi_datetime::Precision::Minute,
            )
            .text
        })
        .unwrap_or_else(|_| request.proposed_at.clone());
    format!(
        "{} at {} ({})",
        if request.event_type == "in" {
            "Clock in"
        } else {
            "Clock out"
        },
        at,
        request.timezone
    )
}
