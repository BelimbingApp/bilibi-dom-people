//! Bilimbi Leave approvals is a shortcut queue into the common request home.
use super::{dates, quantity, render, row, type_name, unit, Core, Flash, Table};
use crate::{LeaveType, PendingRequest, REQUESTS_APPROVE};
use askama::Template;
use bilibi_database::{Error, Result};
use bilibi_people_workforce::{authorization, CompanySummary};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::{LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState},
    params, Choice, EmptyState, FilterToolbar, Html, ToolbarControl,
};
use std::{collections::HashMap, sync::Arc};

pub(super) struct Approvals {
    core: Arc<Core>,
    company_id: Option<i64>,
    companies: Vec<CompanySummary>,
    requests: Vec<PendingRequest>,
    types: Vec<LeaveType>,
    table: Table,
    flash: Flash,
    pub(super) workspace: bool,
}
impl Approvals {
    pub(super) fn new(core: Arc<Core>, query: &HashMap<String, String>) -> Result<Self> {
        Ok(Self {
            core,
            company_id: params::positive_id(query.get("company_id").map(String::as_str)),
            companies: vec![],
            requests: vec![],
            types: vec![],
            table: Table::new(
                "leave-approvals",
                "approvals_grid",
                &[
                    ("employee", "Employee"),
                    ("type", "Leave type"),
                    ("dates", "Dates"),
                    ("quantity", "Quantity"),
                    ("reason", "Reason"),
                ],
            )?,
            flash: Flash::default(),
            workspace: false,
        })
    }
    pub(super) async fn load(&mut self, scope: &Scope, changed: bool) -> Result<()> {
        let client = self.core.db.get().await?;
        self.companies =
            authorization::company_choices(&**client, scope, REQUESTS_APPROVE, self.company_id)
                .await
                .unwrap_or_default();
        drop(client);
        if self.company_id.is_none() {
            self.company_id = scope
                .current_company()
                .map(|id| id.get())
                .filter(|id| self.companies.iter().any(|c| c.id == *id));
        }
        self.requests.clear();
        self.types.clear();
        if let Some(company) = self
            .company_id
            .filter(|id| self.companies.iter().any(|c| c.id == *id))
        {
            match self.core.leave.pending_requests(scope, company).await {
                Ok(rows) => self.requests = rows,
                Err(crate::LeaveError::Failed(e)) => return Err(e),
                Err(_) => self
                    .flash
                    .error("Leave requests are unavailable for this company."),
            }
            self.types = self
                .core
                .leave
                .list_types(scope, company)
                .await
                .unwrap_or_default();
        }
        if changed {
            self.table
                .load(&self.requests, |r| r.request.id.to_string());
        }
        Ok(())
    }
    pub(super) fn render(&self) -> Result<String> {
        let rows = self
            .requests
            .iter()
            .map(|row_| {
                let r = &row_.request;
                Ok(row("leave-approval", r.id)
                    .slot(
                        "employee",
                        Html::text(row_.employee_name.as_deref().unwrap_or("—")),
                    )
                    .slot("type", Html::text(type_name(&self.types, r.leave_type_id)))
                    .slot(
                        "dates",
                        super::request_link(
                            self.company_id.unwrap_or(0),
                            r.id,
                            dates(r),
                            self.workspace,
                        )?,
                    )
                    .slot(
                        "quantity",
                        Html::text(&format!("{} {}", quantity(r.quantity), unit(&r.unit))),
                    )
                    .slot(
                        "reason",
                        Html::text(r.reason.as_deref().unwrap_or_default()),
                    ))
            })
            .collect::<Result<Vec<_>>>()?;
        View {
            flash: self.flash.map(),
            filters: render(&FilterToolbar::new(
                "leave-queue-company",
                vec![ToolbarControl::select(
                    "leave-queue-company-id",
                    "company_id",
                    "Company",
                    &self.company_id.map(|id| id.to_string()).unwrap_or_default(),
                    self.companies
                        .iter()
                        .map(|c| Choice::new(&c.name, c.id.to_string()))
                        .collect(),
                )],
            ))?,
            table: self.table.render(
                "Pending requests",
                rows,
                EmptyState::missing("No leave requests are waiting for a decision.", ""),
            )?,
        }
        .render()
        .map_err(|_| Error::Internal)
    }
}
impl LiveState<Scope, Error> for Approvals {
    fn update<'a>(&'a mut self, req: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            match req.input {
                LiveInput::Event(payload) => match payload
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                {
                    "clear-flash" => self.flash.clear(payload),
                    "approvals_grid" => self.table.event(payload),
                    "filters" => {
                        self.company_id = params::positive_id(
                            params::form_values(payload, "filters")
                                .get("company_id")
                                .map(String::as_str),
                        )
                    }
                    _ => {}
                },
                LiveInput::Params(query) => {
                    self.company_id =
                        params::positive_id(query.get("company_id").map(String::as_str))
                }
                LiveInput::Refresh => {}
            }
            self.load(req.scope, true).await?;
            Ok(LiveBody {
                html: self.render()?,
                query: self
                    .company_id
                    .map(|id| HashMap::from([("company_id".into(), id.to_string())]))
                    .unwrap_or_default(),
                rows: self.requests.len(),
                ..Default::default()
            })
        })
    }
}
#[derive(Template)]
#[template(path = "approvals.html")]
struct View<'a> {
    flash: bilibi_ui::FlashMap<'a>,
    filters: Html,
    table: Html,
}
