//! The common leave request home, for its employee and independent approvers.
use super::{dates, quantity, render, unit, Core, Flash};
use crate::{PendingRequest, REQUESTS_APPROVE, SELF_VIEW};
use askama::Template;
use bilibi_database::{Error, Result};
use bilibi_people_workforce::authorization;
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::{LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState},
    params, Button, ConfirmDialog, Fact, Facts, Field, Form, Html,
};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc};

pub(super) struct Record {
    core: Arc<Core>,
    company: i64,
    id: i64,
    row: Option<PendingRequest>,
    type_name: String,
    instance_notice: String,
    can_review: bool,
    can_cancel: bool,
    own: bool,
    pending: Option<(&'static str, Value)>,
    flash: Flash,
}
impl Record {
    pub(super) fn new(
        core: Arc<Core>,
        path: &HashMap<String, String>,
        query: &HashMap<String, String>,
    ) -> Result<Self> {
        Ok(Self {
            core,
            company: params::positive_id(query.get("company_id").map(String::as_str)).unwrap_or(0),
            id: params::positive_id(path.get("id").map(String::as_str)).ok_or(Error::NotFound)?,
            row: None,
            type_name: String::new(),
            instance_notice: String::new(),
            can_review: false,
            can_cancel: false,
            own: false,
            pending: None,
            flash: Flash::default(),
        })
    }
    pub(super) async fn load(&mut self, scope: &Scope) -> Result<()> {
        if self.company == 0 {
            self.company = scope.current_company().map_or(0, |id| id.get());
        }
        self.row = None;
        self.can_review = false;
        self.can_cancel = false;
        let row = self
            .core
            .leave
            .request(scope, self.company, self.id)
            .await
            .map_err(|e| {
                self.pending = None;
                match e {
                    crate::LeaveError::Failed(e) => e,
                    _ => Error::NotFound,
                }
            })?;
        let client = self.core.db.get().await?;
        self.instance_notice =
            bilibi_system::Instance::resolve(&self.core.leave.settings, &**client)
                .await?
                .notice();
        let review_allowed =
            authorization::allowed(&**client, scope, self.company, REQUESTS_APPROVE).await;
        drop(client);
        let mine = self
            .core
            .leave
            .workforce
            .authorize_self(scope, self.company, SELF_VIEW)
            .await
            .ok()
            .is_some_and(|me| me.employee_id == row.request.employee_id);
        self.own = mine;
        self.can_review = review_allowed
            && row.request.status == "pending"
            && row.request.requested_by_user_id != scope.user()
            && !mine;
        self.can_cancel = mine
            && (row.request.status == "pending"
                || row.request.status == "approved"
                    && row.request.starts_on
                        > self
                            .core
                            .leave
                            .today(scope, self.company)
                            .await
                            .map_err(|_| Error::Invalid)?);
        if self.pending.as_ref().is_some_and(|(key, _)| {
            if key.ends_with("cancel") {
                !self.can_cancel
            } else {
                !self.can_review
            }
        }) {
            self.pending = None;
        }
        self.type_name = self
            .core
            .leave
            .list_types(scope, self.company)
            .await
            .map_err(|_| Error::Invalid)?
            .into_iter()
            .find(|t| t.id == row.request.leave_type_id)
            .map(|t| t.name)
            .unwrap_or_else(|| row.request.leave_type_id.to_string());
        self.row = Some(row);
        Ok(())
    }
    pub(super) fn render(&self) -> Result<String> {
        let row = self.row.as_ref().ok_or(Error::NotFound)?;
        let r = &row.request;
        let pairs = [
            (
                "Employee",
                row.employee_name
                    .clone()
                    .unwrap_or_else(|| r.employee_id.to_string()),
            ),
            ("Leave type", self.type_name.clone()),
            ("Dates", dates(r)),
            (
                "Quantity",
                format!("{} {}", quantity(r.quantity), unit(&r.unit)),
            ),
            ("Status", r.status.clone()),
            ("Reason", r.reason.clone().unwrap_or_default()),
            ("Decision note", r.decision_note.clone().unwrap_or_default()),
        ];
        let facts = render(&Facts {
            id: "leave-request-facts",
            items: pairs
                .into_iter()
                .map(|(title, value)| Fact {
                    title,
                    value: Html::text(&value),
                    ..Default::default()
                })
                .collect(),
        })?;
        let mut actions = Html::EMPTY;
        if self.can_review {
            let fields = render(
                &Field::in_form("decision", "note")
                    .id("leave-approve-note")
                    .label("Decision note")
                    .attr("maxlength", "500"),
            )?;
            let buttons = render(&Button {
                button_type: "submit",
                body: Html::text("Approve"),
                ..Default::default()
            })?;
            actions.push(&render(&Form {
                id: "leave-record-approve",
                event: "approve_record",
                body: fields.clone(),
                actions: buttons,
                ..Default::default()
            })?);
            actions.push(&render(&Form {
                id: "leave-record-reject",
                event: "reject_record",
                body: render(
                    &Field::in_form("decision", "note")
                        .id("leave-reject-note")
                        .label("Decision note")
                        .required()
                        .attr("maxlength", "500"),
                )?,
                actions: render(&Button {
                    button_type: "submit",
                    body: Html::text("Reject"),
                    ..Default::default()
                })?,
                ..Default::default()
            })?);
        }
        if self.can_cancel {
            actions.push(&render(&Button {
                event: "cancel_record",
                body: Html::text("Cancel request"),
                ..Default::default()
            })?);
        }
        let confirmation = if let Some((key, _)) = &self.pending {
            render(&ConfirmDialog {id:"leave-record-confirm",consequence:if key.ends_with("cancel"){"Cancel this leave request?"}else if key.ends_with("approve"){"Approve this leave request?"}else{"Reject this leave request?"},detail:&format!("{} This decision is recorded in the request history. Review the request before confirming.",self.instance_notice),confirm:"Confirm",working:"Saving…",confirm_event:"confirm_record",cancel_event:"dismiss_record"})?
        } else {
            Html::EMPTY
        };
        View {
            id: self.id.to_string(),
            back: if self.own {
                "/people/leave/my".into()
            } else {
                format!("/people/leave/requests?company_id={}", self.company)
            },
            flash: self.flash.map(),
            facts,
            actions,
            confirmation,
        }
        .render()
        .map_err(|_| Error::Internal)
    }
}
impl LiveState<Scope, Error> for Record {
    fn update<'a>(&'a mut self, req: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            self.load(req.scope).await?;
            let mut redirect = None;
            if let LiveInput::Event(payload) = req.input {
                match payload.get("type").and_then(Value::as_str).unwrap_or("") {
                    "clear-flash" => self.flash.clear(payload),
                    "dismiss_record" => self.pending = None,
                    event @ ("approve_record" | "reject_record" | "cancel_record") => {
                        if if event == "cancel_record" {
                            !self.can_cancel
                        } else {
                            !self.can_review
                        } {
                            self.flash
                                .error("You may not take that action on this request.");
                        } else {
                            let note = payload
                                .get("decision[note]")
                                .and_then(Value::as_str)
                                .unwrap_or("");
                            if note.chars().count() > 500
                                || event == "reject_record" && note.trim().is_empty()
                            {
                                self.flash.error("Give a reason when rejecting a request.");
                            } else {
                                let key = match event {
                                    "approve_record" => "people.leave.request.approve",
                                    "reject_record" => "people.leave.request.reject",
                                    _ => "people.leave.request.cancel",
                                };
                                self.pending = Some((
                                    key,
                                    json!({"company_id":self.company,"request_id":self.id,"note":note}),
                                ));
                            }
                        }
                    }
                    "confirm_record" => {
                        if let Some((key, input)) = self.pending.take() {
                            match self
                                .core
                                .write(req.scope, req.credential, "confirm_record", key, input)
                                .await
                            {
                                Ok(_) => {
                                    self.flash.success(match key {
                                        "people.leave.request.approve" => "Leave request approved.",
                                        "people.leave.request.reject" => "Leave request rejected.",
                                        _ => "Leave request cancelled.",
                                    });
                                    if key != "people.leave.request.cancel" {
                                        redirect = Some(format!(
                                            "/people/leave/requests?company_id={}",
                                            self.company
                                        ));
                                    }
                                }
                                Err(e) => self.flash.error(&e.to_string()),
                            }
                        }
                    }
                    _ => {}
                }
            }
            if redirect.is_some() {
                return Ok(LiveBody {
                    html: self.render()?,
                    redirect,
                    flash: self.flash.0.clone(),
                    ..Default::default()
                });
            }
            self.load(req.scope).await?;
            Ok(LiveBody {
                html: self.render()?,
                ..Default::default()
            })
        })
    }
}
#[derive(Template)]
#[template(path = "request.html")]
struct View<'a> {
    id: String,
    back: String,
    flash: bilibi_ui::FlashMap<'a>,
    facts: Html,
    actions: Html,
    confirmation: Html,
}
