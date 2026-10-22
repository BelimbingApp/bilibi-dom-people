//! Bilimbi `Leave.Web.MyLive`: current account's balances, requests and ledger.
use super::{
    action, dates, refusal, render, row, submit, text, type_name, unit, Core, Flash, Table,
};
use crate::{LeaveError, NewRequest, Request, SelfSummary, SELF_VIEW};
use askama::Template;
use bilibi_database::{Error, Result};
use bilibi_people_workforce::authorization;
use bilibi_tenancy::Scope;
use bilibi_ui::{
    choices,
    page::{LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState},
    params, ConfirmDialog, EmptyState, Field, FlashMap, FlexRow, Html, InputKind,
};
use jiff::civil::Date;
use rust_decimal::Decimal;
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

const NO_PERMISSION: &str = "You no longer have permission to use leave self-service.";
const INVALID: &str = "Choose an active leave type and valid dates; half days and hours cover a single date, and hours apply only to hour-based types.";

#[derive(Template)]
#[template(path = "my.html")]
struct View<'a> {
    flash: FlashMap<'a>,
    subtitle: String,
    available: bool,
    has_types: bool,
    state: Html,
    balances: Html,
    fields: Vec<Field>,
    actions: Html,
    requests: Html,
    history: Html,
    confirm: Html,
}
pub(super) struct My {
    core: Arc<Core>,
    summary: Option<SelfSummary>,
    requests: Vec<Request>,
    today: Option<Date>,
    form: HashMap<String, String>,
    request_key: String,
    pending: Option<(i64, i64)>,
    balances_table: Table,
    requests_table: Table,
    history_table: Table,
    flash: Flash,
}
impl My {
    pub(super) fn new(core: Arc<Core>) -> Result<Self> {
        Ok(Self {
            core,
            summary: None,
            requests: vec![],
            today: None,
            form: HashMap::new(),
            request_key: uuid::Uuid::new_v4().to_string(),
            pending: None,
            flash: Flash::default(),
            balances_table: Table::new(
                "my-leave-balances",
                "balances_grid",
                &[
                    ("type", "Leave type"),
                    ("unit", "Unit"),
                    ("entitlement", "Entitlement"),
                    ("carried", "Carried in"),
                    ("opening", "Opening"),
                    ("adjustment", "Adjustments"),
                    ("taken", "Taken"),
                    ("balance", "Balance"),
                    ("pending", "Pending"),
                    ("available", "Available"),
                ],
            )?,
            requests_table: Table::new(
                "my-leave-requests",
                "requests_grid",
                &[
                    ("dates", "Dates"),
                    ("type", "Leave type"),
                    ("quantity", "Quantity"),
                    ("status", "Status"),
                    ("note", "Decision note"),
                    ("actions", "Actions"),
                ],
            )?,
            history_table: Table::new(
                "my-leave-history",
                "history_grid",
                &[
                    ("date", "Date"),
                    ("type", "Leave type"),
                    ("entry", "Entry"),
                    ("quantity", "Quantity"),
                    ("note", "Note"),
                ],
            )?,
        })
    }
    fn company(scope: &Scope) -> Option<i64> {
        scope.actor().organisation().map(|id| id.get())
    }
    pub(super) async fn load(&mut self, scope: &Scope, rows_changed: bool) -> Result<()> {
        // No employee id is cached: every event re-resolves the account link.
        let result = async {
            let company = Self::company(scope).ok_or(LeaveError::NotLinked)?;
            let leave = &self.core.leave;
            Ok::<_, LeaveError>((
                leave.self_summary(scope, company, None).await?,
                leave.self_requests(scope, company).await?,
                leave.today(scope, company).await?,
            ))
        }
        .await;
        match result {
            Ok((summary, requests, today)) => {
                if rows_changed {
                    self.balances_table
                        .load(&summary.balances, |r| r.leave_type.id.to_string());
                    self.requests_table.load(&requests, |r| r.id.to_string());
                    self.history_table
                        .load(&summary.entries, |r| r.id.to_string());
                }
                self.summary = Some(summary);
                self.requests = requests;
                self.today = Some(today);
            }
            Err(error) => {
                refusal(error)?;
                self.summary = None;
                self.requests.clear();
                self.today = None;
                self.pending = None;
                self.form.clear();
            }
        }
        Ok(())
    }
    async fn authorized(&mut self, scope: &Scope) -> Result<Option<i64>> {
        let Some(company) = Self::company(scope) else {
            self.flash
                .error("Your account is not linked to a working employee in this company.");
            return Ok(None);
        };
        let client = self.core.db.get().await?;
        match authorization::authorize(&**client, scope, company, SELF_VIEW).await {
            Ok(_) => Ok(Some(company)),
            Err(error) => {
                refusal(error.into())?;
                self.pending = None;
                self.flash.error(NO_PERMISSION);
                Ok(None)
            }
        }
    }
    async fn event(&mut self, scope: &Scope, payload: &Value) -> Result<bool> {
        let kind = text(payload, "type");
        match kind.as_str() {
            "clear-flash" => self.flash.clear(payload),
            "balances_grid" => self.balances_table.event(payload),
            "requests_grid" => self.requests_table.event(payload),
            "history_grid" => self.history_table.event(payload),
            "dismiss_cancel" => self.pending = None,
            "submit_request" | "cancel_request" | "confirm_cancel" => {
                let Some(company) = self.authorized(scope).await? else {
                    return Ok(true);
                };
                match kind.as_str() {
                    "submit_request" => {
                        self.form = [
                            "leave_type_id",
                            "starts_on",
                            "ends_on",
                            "day_part",
                            "hours",
                            "reason",
                        ]
                        .iter()
                        .map(|key| ((*key).into(), text(payload, &format!("request[{key}]"))))
                        .collect();
                        let result = match self.input() {
                            Ok(input) => {
                                self.core.leave.submit_request(scope, company, &input).await
                            }
                            Err(error) => Err(error),
                        };
                        match result {
                            Ok(_) => {
                                self.form.clear();
                                self.request_key = uuid::Uuid::new_v4().to_string();
                                self.flash.success("Leave request submitted.");
                            }
                            Err(error) => self.flash.error(self_refusal(&refusal(error)?)),
                        }
                    }
                    "cancel_request" => {
                        let id = params::positive_id(payload.get("id").and_then(Value::as_str));
                        // Re-read own records before opening a confirmation.
                        self.load(scope, false).await?;
                        if let Some(r) = self
                            .requests
                            .iter()
                            .find(|r| Some(r.id) == id && cancellable(r, self.today))
                        {
                            self.pending = Some((company, r.id));
                        } else {
                            self.flash.error("That request can no longer be cancelled.");
                        }
                    }
                    _ => {
                        if let Some((selected_company, id)) = self.pending.take() {
                            if selected_company != company {
                                self.flash.error("That request can no longer be cancelled.");
                                return Ok(true);
                            }
                            match self.core.leave.cancel_request(scope, company, id).await {
                                Ok(_) => self.flash.success("Leave request cancelled."),
                                Err(error) => {
                                    let error = refusal(error)?;
                                    self.flash.error(match error {
                                        LeaveError::Unauthorized
                                        | LeaveError::NotLinked
                                        | LeaveError::YearClosed
                                        | LeaveError::CompanyArchived => self_refusal(&error),
                                        _ => "That request can no longer be cancelled.",
                                    });
                                }
                            }
                        }
                    }
                }
                return Ok(true);
            }
            _ => return Err(Error::Invalid),
        }
        Ok(false)
    }
    fn input(&self) -> crate::Result<NewRequest> {
        let value = |key| {
            self.form
                .get(key)
                .map(String::as_str)
                .unwrap_or_default()
                .trim()
        };
        let id =
            params::positive_id(Some(value("leave_type_id"))).ok_or(LeaveError::InvalidRequest)?;
        let start = value("starts_on")
            .parse::<Date>()
            .map_err(|_| LeaveError::InvalidRequest)?;
        let mut input = NewRequest::new(id, start, &self.request_key);
        if !value("ends_on").is_empty() {
            input.ends_on = value("ends_on")
                .parse()
                .map_err(|_| LeaveError::InvalidRequest)?;
        }
        input.day_part = value("day_part").to_owned();
        if !value("hours").is_empty() {
            input.hours = Some(
                value("hours")
                    .parse::<Decimal>()
                    .map_err(|_| LeaveError::InvalidRequest)?,
            );
        }
        if !value("reason").is_empty() {
            input.reason = Some(value("reason").into());
        }
        Ok(input)
    }
    pub(super) fn render(&self) -> Result<String> {
        let mut view = View {
            flash: self.flash.map(),
            subtitle: String::new(),
            available: self.summary.is_some(),
            has_types: false,
            state: Html::EMPTY,
            balances: Html::EMPTY,
            fields: vec![],
            actions: submit("Submit request")?,
            requests: Html::EMPTY,
            history: Html::EMPTY,
            confirm: Html::EMPTY,
        };
        if let Some(summary) = &self.summary {
            view.subtitle = format!("Leave year {} to {}", summary.starts_on, summary.ends_on);
            view.has_types = !summary.balances.is_empty();
            let types: Vec<_> = summary
                .balances
                .iter()
                .map(|b| b.leave_type.clone())
                .collect();
            let rows = summary
                .balances
                .iter()
                .map(|b| {
                    row("leave-balance", b.leave_type.id)
                        .slot("type", Html::text(&b.leave_type.name))
                        .slot("unit", Html::text(unit(&b.leave_type.unit)))
                        .slot("entitlement", Html::text(&b.entitlement.to_string()))
                        .slot("carried", Html::text(&b.carried_forward.to_string()))
                        .slot("opening", Html::text(&b.opening.to_string()))
                        .slot("adjustment", Html::text(&b.adjustment.to_string()))
                        .slot("taken", Html::text(&b.taken.to_string()))
                        .slot("balance", Html::text(&b.balance.to_string()))
                        .slot("pending", Html::text(&b.pending.to_string()))
                        .slot("available", Html::text(&b.available.to_string()))
                })
                .collect();
            view.balances = if view.has_types {
                self.balances_table.render(
                    "Leave balances",
                    rows,
                    EmptyState::missing(
                        "No leave types are set up yet.",
                        "Your balances will appear once an operator adds leave types and policies.",
                    )
                    .id("my-leave-no-types"),
                )?
            } else {
                render(
                    &EmptyState::missing(
                        "No leave types are set up yet.",
                        "Your balances will appear once an operator adds leave types and policies.",
                    )
                    .id("my-leave-no-types"),
                )?
            };
            view.fields = self.fields(&types);
            let mut rows = vec![];
            for r in &self.requests {
                rows.push(
                    row("leave-request", r.id)
                        .slot("dates", Html::text(&dates(r)))
                        .slot("type", Html::text(type_name(&types, r.leave_type_id)))
                        .slot(
                            "quantity",
                            Html::text(&format!("{} {}", r.quantity, unit(&r.unit))),
                        )
                        .slot("status", Html::text(&r.status))
                        .slot(
                            "note",
                            Html::text(r.decision_note.as_deref().unwrap_or_default()),
                        )
                        .slot(
                            "actions",
                            if cancellable(r, self.today) {
                                action("cancel_request", "id", &r.id.to_string(), "Cancel")?
                            } else {
                                Html::EMPTY
                            },
                        ),
                );
            }
            view.requests = self.requests_table.render(
                "My requests",
                rows,
                EmptyState::missing("You have not requested any leave yet.", "")
                    .id("my-leave-no-requests"),
            )?;
            let rows = summary
                .entries
                .iter()
                .map(|e| {
                    FlexRow::new(e.id.to_string())
                        .slot("date", Html::text(&e.occurred_on.to_string()))
                        .slot("type", Html::text(type_name(&types, e.leave_type_id)))
                        .slot("entry", Html::text(&e.entry_type))
                        .slot(
                            "quantity",
                            Html::text(&format!("{} {}", e.quantity, unit(&e.unit))),
                        )
                        .slot("note", Html::text(e.note.as_deref().unwrap_or_default()))
                })
                .collect();
            view.history = self.history_table.render(
                "Balance history",
                rows,
                EmptyState::missing(
                    "No entitlements or adjustments have been recorded for this leave year.",
                    "",
                )
                .id("my-leave-no-entries"),
            )?;
            if let Some((_, id)) = self.pending {
                if let Some(r) = self.requests.iter().find(|r| r.id == id) {
                    view.confirm = render(&ConfirmDialog {
                        id: "leave-cancel-confirm",
                        consequence: "Cancel this leave request?",
                        detail: &format!(
                            "{} · {} · {} {}",
                            type_name(&types, r.leave_type_id),
                            dates(r),
                            r.quantity,
                            unit(&r.unit)
                        ),
                        confirm: "Cancel request",
                        working: "Cancelling…",
                        confirm_event: "confirm_cancel",
                        cancel_event: "dismiss_cancel",
                    })?;
                }
            }
        } else {
            view.state = render(
                &EmptyState::missing(
                    "Leave is unavailable for this account.",
                    "Ask an operator to link your account to an active employee in this company.",
                )
                .id("my-leave-unavailable"),
            )?;
        }
        view.render().map_err(|_| Error::Internal)
    }
    fn fields(&self, types: &[crate::LeaveType]) -> Vec<Field> {
        let options: Vec<_> = types
            .iter()
            .filter(|t| t.status == "active")
            .map(|t| (format!("{} ({})", t.name, unit(&t.unit)), t.id.to_string()))
            .collect();
        if options.is_empty() {
            return vec![];
        }
        let field = |key, label| {
            Field::in_form("request", key)
                .label(label)
                .value(self.form.get(key).cloned().unwrap_or_default())
        };
        let parts = [
            ("Full days", "full"),
            ("Morning (half day)", "am"),
            ("Afternoon (half day)", "pm"),
            ("Hours (hour-based types)", "hours"),
        ];
        vec![
            field("leave_type_id", "Leave type")
                .kind(InputKind::Select)
                .options(choices(&options)),
            field("starts_on", "From").kind(InputKind::Date).required(),
            field("ends_on", "To").kind(InputKind::Date),
            field("day_part", "Duration")
                .kind(InputKind::Select)
                .options(choices(&parts)),
            field("hours", "Hours")
                .attr("inputmode", "decimal")
                .placeholder("Only for hours"),
            field("reason", "Reason").attr("maxlength", "500"),
        ]
    }
}
impl LiveState<Scope, Error> for My {
    fn update<'a>(&'a mut self, request: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            let changed = match request.input {
                LiveInput::Event(payload) => self.event(request.scope, payload).await?,
                _ => true,
            };
            self.load(request.scope, changed).await?;
            Ok(LiveBody {
                html: self.render()?,
                rows: self.requests.len(),
                ..LiveBody::default()
            })
        })
    }
}
fn cancellable(r: &Request, today: Option<Date>) -> bool {
    r.status == "pending"
        || (r.status == "approved" && today.is_some_and(|today| r.starts_on > today))
}
fn self_refusal(error: &LeaveError) -> &str {
    match error {
        LeaveError::Unauthorized => NO_PERMISSION,
        LeaveError::NotLinked => {
            "Your account is not linked to a working employee in this company."
        }
        LeaveError::InsufficientBalance => "The available balance does not cover this request.",
        LeaveError::OverlappingRequest => {
            "Another pending or approved request already covers part of these dates."
        }
        LeaveError::NoWorkingDays => "The chosen dates contain no working days.",
        LeaveError::SpansLeaveYears => "A request must fall within one leave year.",
        LeaveError::TooFarBack => "The start date is further back than requests may start.",
        LeaveError::YearClosed => "That leave year has been carried forward and is closed.",
        LeaveError::CompanyArchived => "This company is archived and read-only",
        _ => INVALID,
    }
}
