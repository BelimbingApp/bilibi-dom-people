//! My claims mirrors Bilimbi People Claims.Web.MyClaimsLive. Its unpaged lists
//! use FlexTable column controls; no sort/filter/page controls exist in Bilimbi.
use crate::{
    Claims, ClaimsError, FieldError, NewRequest, OpenClaimType, Request, SelfEmployee, SUBMIT,
};
use askama::Template;
use bilibi_database::{Error, Result};
use bilibi_people_workforce::authorization;
use bilibi_tenancy::Scope;
use bilibi_ui::{
    choices,
    page::{
        LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState, MountFuture, Page, PageBody,
        PageFuture, PageRequest,
    },
    params, BuiltinColumn, Button, ButtonVariant, ConfirmDialog, EmptyState, ErrorSummary, Field,
    FlashMap, FlexRow, FlexTable, Html, InputKind, ListColumns, ListSpec, ListState, StatusKind,
    TableEmpty, NO_FLASH,
};
use rust_decimal::Decimal;
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

pub(crate) const SERVICE: &str = "people/claims.pages";
const UNAVAILABLE: &str = "You are not a working employee of this company.";
const NO_PERMISSION: &str = "You no longer have permission to submit claims here.";

pub(crate) struct Pages(Arc<Claims>);
impl Pages {
    pub(crate) fn new(claims: Arc<Claims>) -> Self {
        Self(claims)
    }
}
impl Page<Scope, Error> for Pages {
    fn title(&self, operation: &str) -> Option<&'static str> {
        (operation == "my_claims").then_some("My claims")
    }
    fn render<'a>(&'a self, request: PageRequest<'a, Scope>) -> PageFuture<'a, Error> {
        Box::pin(async move {
            if request.operation != "my_claims" {
                return Err(Error::NotFound);
            }
            let mut state = MyClaims::new(self.0.clone(), request.query)?;
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
            if request.operation != "my_claims" {
                return Ok(None);
            }
            Ok(Some(
                Box::new(MyClaims::new(self.0.clone(), request.query)?)
                    as Box<dyn LiveState<Scope, Error>>,
            ))
        })
    }
}

#[derive(Template)]
#[template(path = "my_claims.html")]
struct View<'a> {
    flash: FlashMap<'a>,
    subtitle: String,
    available: bool,
    has_types: bool,
    open_types: Html,
    requests: Html,
    unavailable: Html,
    fields: Vec<Field>,
    possible_duplicate: bool,
    duplicate_field: Field,
    submit: Html,
    errors: Html,
    confirmation: Html,
}
struct MyClaims {
    claims: Arc<Claims>,
    company: Option<i64>,
    employee: Option<SelfEmployee>,
    open: Vec<OpenClaimType>,
    requests: Vec<Request>,
    names: HashMap<i64, String>,
    form: HashMap<String, String>,
    errors: Vec<FieldError>,
    possible_duplicate: bool,
    /// Only the selection proven to be an own submitted claim is retained.
    pending_withdraw: Option<i64>,
    open_columns: ListColumns,
    request_columns: ListColumns,
    list: ListState,
    flash: Option<(StatusKind, String)>,
}
impl MyClaims {
    fn new(claims: Arc<Claims>, query: &HashMap<String, String>) -> Result<Self> {
        Ok(Self {
            claims,
            company: None,
            employee: None,
            open: vec![],
            requests: vec![],
            names: HashMap::new(),
            form: HashMap::new(),
            errors: vec![],
            possible_duplicate: false,
            pending_withdraw: None,
            open_columns: columns(
                "my-claims-open-types",
                &[
                    ("type", "Type"),
                    ("category", "Category"),
                    ("currency", "Currency"),
                    ("limits", "Limits"),
                    ("receipt", "Receipt rule"),
                ],
            )?,
            request_columns: columns(
                "my-claims-table",
                &[
                    ("type", "Type"),
                    ("date", "Date"),
                    ("amount", "Amount"),
                    ("receipt", "Receipt"),
                    ("status", "Status"),
                    ("decision", "Decision"),
                    ("actions", "Actions"),
                ],
            )?,
            list: list_state(query)?,
            flash: None,
        })
    }
    fn clear_private(&mut self) {
        self.employee = None;
        self.open.clear();
        self.requests.clear();
        self.names.clear();
        self.pending_withdraw = None;
        self.form.clear();
        self.errors.clear();
        self.possible_duplicate = false;
    }
    async fn load(&mut self, scope: &Scope) -> Result<()> {
        // This company comes only from the signed-in account, never from an event.
        self.company = scope.actor().organisation().map(|id| id.get());
        let Some(company) = self.company else {
            self.clear_private();
            return Ok(());
        };
        let result = async {
            let employee = self.claims.self_service_employee(scope, company).await?;
            let open = self.claims.self_open_claim_types(scope, company).await?;
            let types = self.claims.claim_types(scope, company).await?;
            let requests = self.claims.self_requests(scope, company).await?;
            Ok::<_, ClaimsError>((employee, open, types, requests))
        }
        .await;
        match result {
            Ok((employee, open, types, requests)) => {
                if self
                    .employee
                    .as_ref()
                    .is_some_and(|old| old.id != employee.id)
                {
                    self.clear_private();
                }
                if self.open != open {
                    self.open_columns
                        .load(&open, |t| t.claim_type.id.to_string());
                }
                if self.requests != requests {
                    self.request_columns.load(&requests, |r| r.id.to_string());
                }
                self.employee = Some(employee);
                self.open = open;
                self.requests = requests;
                self.names = types.into_iter().map(|t| (t.id, t.name)).collect();
            }
            Err(ClaimsError::Failed(error)) => return Err(error),
            Err(_) => self.clear_private(),
        }
        Ok(())
    }
    fn flash(&mut self, kind: StatusKind, message: impl Into<String>) {
        self.flash = Some((kind, message.into()));
    }
    async fn event(&mut self, scope: &Scope, payload: &Value) -> Result<()> {
        let kind = payload.get("type").and_then(Value::as_str).unwrap_or("");
        if kind == "clear-flash" {
            if self
                .flash
                .as_ref()
                .is_some_and(|(k, _)| Some(k.name()) == payload.get("key").and_then(Value::as_str))
            {
                self.flash = None;
            }
            return Ok(());
        }
        if kind == "open_types_grid" || kind == "claims_grid" {
            let columns = if kind == "open_types_grid" {
                &mut self.open_columns
            } else {
                &mut self.request_columns
            };
            columns.handle(&params::string_values(payload));
            return Ok(());
        }
        if kind == "cancel_withdraw" {
            self.pending_withdraw = None;
            return Ok(());
        }
        if !matches!(kind, "submit_claim" | "withdraw_claim" | "confirm_withdraw") {
            return Err(Error::Invalid);
        }
        // Recheck before parsing write input, then the facade rechecks under its locks.
        let permitted = if let Some(company) = self.company {
            let client = self.claims.db.get().await?;
            authorization::authorize(&**client, scope, company, SUBMIT)
                .await
                .is_ok()
        } else {
            false
        };
        if !permitted || self.employee.is_none() {
            self.pending_withdraw = None;
            self.flash(
                StatusKind::Error,
                if permitted {
                    UNAVAILABLE
                } else {
                    NO_PERMISSION
                },
            );
            return Ok(());
        }
        let Some(company) = self.company else {
            return Ok(());
        };
        match kind {
            "submit_claim" => {
                self.form = claim_values(payload);
                self.errors.clear();
                let result = parse_request(&self.form).map_err(ClaimsError::Invalid);
                let result = match result {
                    Ok(input) => self.claims.submit_request(scope, company, &input).await,
                    Err(error) => Err(error),
                };
                match result {
                    Ok(_) => {
                        self.form.clear();
                        self.possible_duplicate = false;
                        self.flash(StatusKind::Success, "Claim submitted.");
                    }
                    Err(error) => {
                        self.possible_duplicate = matches!(error, ClaimsError::PossibleDuplicate);
                        if let ClaimsError::Invalid(errors) = &error {
                            self.errors = errors.clone();
                        }
                        self.flash(StatusKind::Error, refusal(&error));
                    }
                }
            }
            "withdraw_claim" => {
                let id = params::positive_id(
                    payload
                        .get("value")
                        .or_else(|| payload.get("id"))
                        .and_then(Value::as_str),
                );
                self.pending_withdraw = id.filter(|id| {
                    self.requests
                        .iter()
                        .any(|r| r.id == *id && r.status == "submitted")
                });
                if self.pending_withdraw.is_none() {
                    self.flash(StatusKind::Error, "This claim cannot be withdrawn.");
                }
            }
            "confirm_withdraw" => {
                if let Some(id) = self.pending_withdraw.take() {
                    match self.claims.withdraw_request(scope, company, id).await {
                        Ok(_) => self.flash(StatusKind::Success, "Claim withdrawn."),
                        Err(error) => self.flash(
                            StatusKind::Error,
                            match error {
                                ClaimsError::Unauthorized | ClaimsError::NotLinked => {
                                    refusal(&error)
                                }
                                _ => "This claim cannot be withdrawn.",
                            },
                        ),
                    }
                } else {
                    self.flash(StatusKind::Error, "This claim cannot be withdrawn.");
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    }
    fn render(&self) -> Result<String> {
        let mut flash = NO_FLASH;
        if let Some((kind, message)) = &self.flash {
            match kind {
                StatusKind::Success => flash.success = message,
                StatusKind::Error => flash.error = message,
                StatusKind::Info => flash.info = message,
                StatusKind::Warning => flash.warning = message,
            }
        }
        let open_rows = self
            .open
            .iter()
            .map(|t| {
                FlexRow::new(t.claim_type.id.to_string())
                    .slot("type", Html::text(&t.claim_type.name))
                    .slot("category", Html::text(&t.category_name))
                    .slot("currency", Html::text(&t.policy.currency))
                    .slot("limits", Html::text(&limits(t)))
                    .slot("receipt", Html::text(&receipt_rule(t)))
            })
            .collect();
        let mut rows = Vec::new();
        for r in &self.requests {
            let actions = if r.status == "submitted" {
                render(&Button {
                    id: &format!("claim-{}-withdraw", r.id),
                    event: "withdraw_claim",
                    value: &r.id.to_string(),
                    body: Html::text("Withdraw"),
                    ..Button::default()
                })?
            } else {
                Html::EMPTY
            };
            rows.push(
                FlexRow::new(format!("claim-{}", r.id))
                    .slot(
                        "type",
                        Html::text(self.names.get(&r.claim_type_id).map_or("—", String::as_str)),
                    )
                    .slot("date", Html::text(&r.incurred_on.to_string()))
                    .slot(
                        "amount",
                        Html::text(&format!("{} {}", r.amount, r.currency)),
                    )
                    .slot(
                        "receipt",
                        Html::text(r.receipt_number.as_deref().unwrap_or("—")),
                    )
                    .slot("status", Html::text(&capitalize(&r.status)))
                    .slot("decision", Html::text(&decision(r)))
                    .slot("actions", actions),
            );
        }
        let fields = self.fields();
        let mut summary = ErrorSummary::new("my-claims-errors");
        for field in &fields {
            for error in &field.errors {
                summary.push(
                    &field.id,
                    bilibi_ui::forms::error_sentence(&field.label, error),
                );
            }
        }
        let confirmation = if let Some(id) = self.pending_withdraw {
            render(&ConfirmDialog {
                id: "my-claims-withdraw-confirm",
                consequence: "Withdraw this claim?",
                detail: &format!("Claim #{id} will be withdrawn. This cannot be undone."),
                confirm: "Withdraw",
                working: "Withdrawing…",
                confirm_event: "confirm_withdraw",
                cancel_event: "cancel_withdraw",
            })?
        } else {
            Html::EMPTY
        };
        View {
            flash,
            subtitle: self
                .employee
                .as_ref()
                .map(|e| format!("{} · {}", e.display_name, e.employee_number))
                .unwrap_or_default(),
            available: self.employee.is_some(),
            has_types: !self.open.is_empty(),
            open_types: table(
                &self.open_columns,
                "open_types_grid",
                "Open claim types",
                open_rows,
                "No claim types are open for claims yet.",
                "my-claims-no-types",
                &self.list,
            )?,
            requests: table(
                &self.request_columns,
                "claims_grid",
                "Submitted claims",
                rows,
                "You have not submitted any claims.",
                "my-claims-empty",
                &self.list,
            )?,
            unavailable: render(
                &EmptyState::missing(
                    "Your account is not linked to a working employee in this company.",
                    "",
                )
                .id("my-claims-unavailable"),
            )?,
            fields,
            possible_duplicate: self.possible_duplicate,
            duplicate_field: Field::in_form("claim", "confirm_duplicate")
                .kind(InputKind::Checkbox)
                .label("This is a separate expense, not a repeat of an earlier claim.")
                .checked(
                    self.form
                        .get("confirm_duplicate")
                        .is_some_and(|v| v == "true"),
                ),
            submit: render(&Button {
                id: "my-claims-submit",
                button_type: "submit",
                variant: ButtonVariant::Primary,
                body: Html::text("Submit claim"),
                ..Button::default()
            })?,
            errors: render(&summary)?,
            confirmation,
        }
        .render()
        .map_err(|_| Error::Internal)
    }
    fn fields(&self) -> Vec<Field> {
        let value = |key: &str| self.form.get(key).cloned().unwrap_or_default();
        let types: Vec<_> = self
            .open
            .iter()
            .map(|t| (t.claim_type.name.clone(), t.claim_type.id.to_string()))
            .collect();
        let mut currencies = Vec::new();
        for t in &self.open {
            let c = (t.policy.currency.clone(), t.policy.currency.clone());
            if !currencies.contains(&c) {
                currencies.push(c);
            }
        }
        let mut fields = vec![
            Field::in_form("claim", "claim_type_id")
                .label("Claim type")
                .kind(InputKind::Select)
                .options(choices(&types))
                .value(
                    self.form
                        .get("claim_type_id")
                        .cloned()
                        .unwrap_or_else(|| types.first().map(|t| t.1.clone()).unwrap_or_default()),
                ),
            Field::in_form("claim", "incurred_on")
                .label("Expense date")
                .kind(InputKind::Date)
                .value(value("incurred_on"))
                .required(),
            Field::in_form("claim", "amount")
                .label("Amount")
                .value(value("amount"))
                .attr("inputmode", "decimal")
                .required(),
            Field::in_form("claim", "currency")
                .label("Currency")
                .kind(InputKind::Select)
                .options(choices(&currencies))
                .value(self.form.get("currency").cloned().unwrap_or_else(|| {
                    currencies.first().map(|c| c.1.clone()).unwrap_or_default()
                })),
            Field::in_form("claim", "receipt_number")
                .label("Receipt number")
                .value(value("receipt_number"))
                .attr("maxlength", "100"),
            Field::in_form("claim", "description")
                .label("Description")
                .value(value("description"))
                .attr("maxlength", "500"),
        ];
        for field in &mut fields {
            field.errors = self
                .errors
                .iter()
                .filter(|e| field.name == format!("claim[{}]", e.field))
                .map(|e| e.message.clone())
                .collect();
        }
        fields
    }
}
impl LiveState<Scope, Error> for MyClaims {
    fn update<'a>(&'a mut self, request: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            self.load(request.scope).await?;
            match request.input {
                LiveInput::Event(payload) => self.event(request.scope, payload).await?,
                LiveInput::Params(query) => self.list = list_state(query)?,
                LiveInput::Refresh => {}
            }
            self.load(request.scope).await?;
            Ok(LiveBody {
                html: self.render()?,
                rows: self.requests.len() + self.open.len(),
                query: Some(self.list.to_params()),
                ..LiveBody::default()
            })
        })
    }
}
fn columns(id: &str, labels: &[(&str, &str)]) -> Result<ListColumns> {
    ListColumns::mount(
        id,
        labels
            .iter()
            .map(|(key, label)| {
                let column = BuiltinColumn::new(key, label);
                if *key == "amount" {
                    column.right()
                } else {
                    column
                }
            })
            .collect(),
    )
    .map_err(|_| Error::Internal)
}
fn list_state(query: &HashMap<String, String>) -> Result<ListState> {
    ListState::parse(
        query,
        Arc::new(ListSpec {
            omit_defaults: true,
            omit_blank: vec!["search".into()],
            ..ListSpec::default()
        }),
    )
    .map_err(|_| Error::Internal)
}
#[allow(clippy::too_many_arguments)] // Every table names its own empty state and event.
fn table(
    columns: &ListColumns,
    event: &str,
    caption: &str,
    rows: Vec<FlexRow>,
    empty: &str,
    empty_id: &str,
    state: &ListState,
) -> Result<Html> {
    let is_empty = rows.is_empty();
    let mut table = FlexTable::new(&columns.id, columns.flex_columns(), rows)
        .sorted(state.sort_by.as_deref(), state.sort_dir);
    table.event = event;
    table.caption = caption;
    table.zoom = columns.zoom;
    table.suggestions = columns.flex_suggestions();
    table.add_query = &columns.add_query;
    if is_empty {
        table.empty = Some(TableEmpty::State(
            EmptyState::missing(empty, "").id(empty_id),
        ));
    }
    render(&table)
}
fn render(template: &impl Template) -> Result<Html> {
    Html::render(template).map_err(|_| Error::Internal)
}
fn claim_values(payload: &Value) -> HashMap<String, String> {
    [
        "claim_type_id",
        "incurred_on",
        "amount",
        "currency",
        "receipt_number",
        "description",
        "confirm_duplicate",
    ]
    .into_iter()
    .filter_map(|key| {
        payload
            .get(format!("claim[{key}]"))
            .and_then(Value::as_str)
            .map(|v| (key.into(), v.into()))
    })
    .collect()
}
fn parse_request(
    form: &HashMap<String, String>,
) -> std::result::Result<NewRequest, Vec<FieldError>> {
    let value = |key: &str| form.get(key).map_or("", String::as_str);
    let mut errors = Vec::new();
    let date = value("incurred_on")
        .parse()
        .map_err(|_| errors.push(FieldError::new("incurred_on", "is invalid")))
        .ok();
    let amount = Decimal::from_str_exact(value("amount"))
        .map_err(|_| errors.push(FieldError::new("amount", "is invalid")))
        .ok();
    let id = params::positive_id(Some(value("claim_type_id")));
    if id.is_none() {
        errors.push(FieldError::new("claim_type_id", "can't be blank"));
    }
    match (date, amount, id) {
        (Some(incurred_on), Some(amount), Some(claim_type_id)) => Ok(NewRequest {
            claim_type_id,
            incurred_on,
            amount,
            currency: value("currency").into(),
            receipt_number: Some(value("receipt_number").into()),
            description: Some(value("description").into()),
            confirm_duplicate: value("confirm_duplicate") == "true",
        }),
        _ => Err(errors),
    }
}
fn refusal(error: &ClaimsError) -> &'static str {
    match error {
        ClaimsError::Unauthorized => NO_PERMISSION,
        ClaimsError::NotFound | ClaimsError::NotLinked | ClaimsError::EmployeeNotFound => UNAVAILABLE,
        ClaimsError::ClaimTypeUnavailable | ClaimsError::ClaimTypeNotFound => "Choose a claim type that is open for claims.",
        ClaimsError::ClaimTypeNotAssigned => "This claim type is not assigned to you.",
        ClaimsError::FutureIncurredOn => "The expense date cannot be in the future.",
        ClaimsError::NoEffectivePolicy => "No claim policy covers this type on the expense date.",
        ClaimsError::CurrencyNotAllowed => "Use the currency shown for this claim type.",
        ClaimsError::ReceiptRequired => "This claim needs a receipt number.",
        ClaimsError::PerClaimLimitExceeded => "The amount is above the per-claim limit.",
        ClaimsError::MonthlyLimitExceeded => "The amount would exceed this month's limit.",
        ClaimsError::YearlyLimitExceeded => "The amount would exceed this year's limit.",
        ClaimsError::DuplicateReceipt => "This receipt number is already on one of your claims.",
        ClaimsError::PossibleDuplicate => "You already claimed this type, date, and amount. Confirm it is a separate expense to submit.",
        ClaimsError::CompanyArchived => "This company is archived and read-only",
        _ => "Check the claim details.",
    }
}
fn limits(t: &OpenClaimType) -> String {
    [
        ("per claim", t.policy.per_claim_limit),
        ("per month", t.policy.monthly_limit),
        ("per year", t.policy.yearly_limit),
    ]
    .into_iter()
    .filter_map(|(label, value)| value.map(|v| format!("{v} {label}")))
    .collect::<Vec<_>>()
    .join(" · ")
}
fn receipt_rule(t: &OpenClaimType) -> String {
    match t.claim_type.receipt_requirement.as_str() {
        "always" => "Receipt required".into(),
        "never" => "No receipt needed".into(),
        _ => format!(
            "Receipt required above {}",
            t.policy
                .receipt_threshold
                .map(|v| v.to_string())
                .unwrap_or_default()
        ),
    }
}
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map_or(String::new(), |c| {
        c.to_uppercase().collect::<String>() + chars.as_str()
    })
}
fn decision(r: &Request) -> String {
    match r.status.as_str() {
        "approved" | "reimbursed" => {
            let mut text = format!(
                "Approved {} {}",
                r.approved_amount.map(|a| a.to_string()).unwrap_or_default(),
                r.currency
            );
            if let Some(reason) = &r.decision_reason {
                text.push_str(&format!(": {reason}"));
            }
            if r.status == "reimbursed" {
                if let Some(at) = &r.reimbursed_at {
                    text.push_str(&format!(", paid {}", at.split('T').next().unwrap_or(at)));
                }
            }
            text
        }
        "rejected" => r.decision_reason.clone().unwrap_or_default(),
        _ => "—".into(),
    }
}
