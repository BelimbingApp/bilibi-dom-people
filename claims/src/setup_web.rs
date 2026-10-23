//! Claim policies mirrors Bilimbi People Claims.Web.SetupLive.
//! Lists use the shared FlexTable; the reference has no sorting or paging.
use crate::{
    AssignableEmployee, Assignment, Category, ClaimType, Claims, ClaimsError, FieldError,
    NewAssignment, NewCategory, NewClaimType, NewPolicy, Policy, MANAGE,
};
use askama::Template;
use bilibi_database::{Error, Result};
use bilibi_people_workforce::{authorization, CompanySummary, WorkforceError};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::{
        LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState, MountFuture, Page, PageBody,
        PageFuture, PageRequest,
    },
    params, BuiltinColumn, Button, ButtonVariant, Choice, ConfirmDialog, EmptyState, ErrorSummary,
    Field, Filter, FlashMap, FlexRow, FlexTable, Form, Html, InputKind, ListColumns, ListSpec,
    ListState, MultiSelect, StatusKind, TableEmpty, NO_FLASH,
};
use jiff::civil::Date;
use rust_decimal::Decimal;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

pub(crate) const SERVICE: &str = "people/claims.setup_pages";
const FORBIDDEN: &str = "You no longer have permission to change this company's claim policies.";
type TableSpec = (
    &'static str,
    &'static str,
    &'static [(&'static str, &'static str)],
);
const TABLES: [TableSpec; 5] = [
    (
        "claim-currencies-table",
        "currencies_grid",
        &[("currency", "Currency")],
    ),
    (
        "claim-categories-table",
        "categories_grid",
        &[("name", "Name"), ("code", "Code"), ("status", "Status")],
    ),
    (
        "claim-types-table",
        "types_grid",
        &[
            ("name", "Name"),
            ("code", "Code"),
            ("category", "Category"),
            ("receipt", "Receipt"),
            ("eligibility", "Who may claim"),
            ("status", "Status"),
        ],
    ),
    (
        "claim-assignments-table",
        "assignments_grid",
        &[
            ("name", "Name"),
            ("code", "Code"),
            ("from", "From"),
            ("to", "To"),
            ("members", "Members"),
        ],
    ),
    (
        "claim-policies-table",
        "policies_grid",
        &[
            ("type", "Claim type"),
            ("from", "From"),
            ("to", "To"),
            ("currency", "Currency"),
            ("per_claim_limit", "Per claim"),
            ("monthly_limit", "Per month"),
            ("yearly_limit", "Per year"),
            ("receipt_threshold", "Receipt above"),
        ],
    ),
];

pub(crate) struct Pages(Arc<Claims>);
impl Pages {
    pub(crate) fn new(claims: Arc<Claims>) -> Self {
        Self(claims)
    }
}
impl Page<Scope, Error> for Pages {
    fn title(&self, _: &str) -> Option<&'static str> {
        Some("Claim policies")
    }
    fn render<'a>(&'a self, request: PageRequest<'a, Scope>) -> PageFuture<'a, Error> {
        Box::pin(async move {
            let mut state = Setup::new(self.0.clone(), request.query)?;
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
            Ok(Some(Box::new(Setup::new(self.0.clone(), request.query)?)
                as Box<dyn LiveState<Scope, Error>>))
        })
    }
}
fn spec() -> Result<Arc<ListSpec>> {
    ListSpec {
        filters: BTreeMap::from([("company_id".into(), Filter::string(""))]),
        omit_defaults: true,
        ..ListSpec::default()
    }
    .shared()
    .map_err(|_| Error::Internal)
}
#[derive(Clone, Copy)]
enum EndKind {
    Assignment,
    Policy,
}
struct PendingEnd {
    company: i64,
    id: i64,
    name: String,
    date: Date,
    kind: EndKind,
}
struct Setup {
    claims: Arc<Claims>,
    list: ListState,
    columns: Vec<ListColumns>,
    companies: Vec<CompanySummary>,
    company: Option<i64>,
    currencies: Vec<String>,
    categories: Vec<Category>,
    types: Vec<ClaimType>,
    assignments: Vec<Assignment>,
    employees: Vec<AssignableEmployee>,
    policies: Vec<Policy>,
    drafts: HashMap<String, Value>,
    errors: Vec<FieldError>,
    error_form: String,
    pending: Option<PendingEnd>,
    instance: Option<bilibi_system::Instance>,
    flash: Option<(StatusKind, String)>,
}
impl Setup {
    fn new(claims: Arc<Claims>, query: &HashMap<String, String>) -> Result<Self> {
        Ok(Self {
            claims,
            list: ListState::parse(query, spec()?).map_err(|_| Error::Internal)?,
            columns: TABLES
                .iter()
                .map(|(id, _, fields)| {
                    ListColumns::mount(
                        id,
                        fields
                            .iter()
                            .map(|(id, label)| BuiltinColumn::new(id, label))
                            .collect(),
                    )
                    .map_err(|_| Error::Internal)
                })
                .collect::<Result<_>>()?,
            companies: vec![],
            company: None,
            currencies: vec![],
            categories: vec![],
            types: vec![],
            assignments: vec![],
            employees: vec![],
            policies: vec![],
            drafts: HashMap::new(),
            errors: vec![],
            error_form: String::new(),
            pending: None,
            instance: None,
            flash: None,
        })
    }
    fn clear_records(&mut self) {
        self.currencies.clear();
        self.categories.clear();
        self.types.clear();
        self.assignments.clear();
        self.employees.clear();
        self.policies.clear();
    }
    fn clear_forms(&mut self) {
        self.pending = None;
        self.drafts.clear();
        self.errors.clear();
        self.error_form.clear();
    }
    fn message(&mut self, kind: StatusKind, message: impl Into<String>) {
        self.flash = Some((kind, message.into()));
    }
    fn flash(&self) -> FlashMap<'_> {
        let mut flash = NO_FLASH;
        if let Some((kind, text)) = &self.flash {
            if *kind == StatusKind::Success {
                flash.success = text;
            } else {
                flash.error = text;
            }
        }
        flash
    }
    async fn load(&mut self, scope: &Scope) -> Result<()> {
        let client = self.claims.db.get().await?;
        self.companies = match authorization::selectable_companies(&**client, scope, MANAGE).await {
            Ok(rows) => rows,
            Err(WorkforceError::Failed(e)) => return Err(e),
            Err(_) => vec![],
        };
        let chosen = &self.list.filters["company_id"];
        let company = if chosen.is_empty() {
            self.companies.first()
        } else {
            self.companies.iter().find(|c| c.id.to_string() == *chosen)
        }
        .map(|c| c.id);
        if self.company.is_some() && self.company != company {
            self.clear_forms();
        }
        self.company = company;
        self.instance =
            Some(bilibi_system::Instance::resolve(&self.claims.settings, &**client).await?);
        drop(client);
        self.clear_records();
        if let Some(company) = company {
            self.list
                .filters
                .insert("company_id".into(), company.to_string());
            let loaded = async {
                self.currencies = self.claims.currencies(scope, company).await?;
                self.categories = self.claims.categories(scope, company).await?;
                self.types = self.claims.claim_types(scope, company).await?;
                self.assignments = self.claims.assignments(scope, company).await?;
                self.policies = self.claims.policies(scope, company).await?;
                self.employees = self.claims.assignable_employees(scope, company).await?;
                Ok::<_, ClaimsError>(())
            }
            .await;
            if let Err(error) = loaded {
                self.company = None;
                self.clear_records();
                self.clear_forms();
                self.refused(error, "Claim setup is unavailable.");
            }
        }
        Ok(())
    }
    async fn permitted(&mut self, scope: &Scope) -> Result<bool> {
        let company = self.pending.as_ref().map(|p| p.company).or(self.company);
        let allowed = if let Some(company) = company {
            let client = self.claims.db.get().await?;
            match authorization::authorize(&**client, scope, company, MANAGE).await {
                Ok(_) => true,
                Err(WorkforceError::Failed(e)) => return Err(e),
                Err(_) => false,
            }
        } else {
            false
        };
        if !allowed {
            self.clear_forms();
            self.message(StatusKind::Error, FORBIDDEN);
        }
        Ok(allowed)
    }
    fn refused(&mut self, error: ClaimsError, fallback: &str) {
        let message = match error {
            ClaimsError::Unauthorized => FORBIDDEN,
            ClaimsError::CompanyArchived => "This company is archived and read-only",
            ClaimsError::OverlappingPolicy => {
                "This period overlaps another policy for the claim type."
            }
            ClaimsError::RequestsAfterEnd => {
                "Claims under this policy were incurred after that date."
            }
            ClaimsError::Invalid(errors) => {
                self.errors = errors;
                fallback
            }
            _ => fallback,
        };
        self.message(StatusKind::Error, message);
    }
    async fn event(&mut self, scope: &Scope, payload: &Value) -> Result<()> {
        let event = text(payload, "type");
        if event == "clear-flash" {
            if self
                .flash
                .as_ref()
                .is_some_and(|(kind, _)| kind.name() == text(payload, "key"))
            {
                self.flash = None;
            }
            return Ok(());
        }
        if let Some(i) = TABLES
            .iter()
            .position(|(_, event_name, _)| *event_name == event)
        {
            self.columns[i].handle(&params::string_values(payload));
            return Ok(());
        }
        if event == "cancel_end" {
            self.pending = None;
            return Ok(());
        }
        if event == "select_company" {
            self.clear_forms();
            let id = text(payload, "company_id");
            if self.companies.iter().any(|c| c.id.to_string() == id) {
                self.list.filters.insert("company_id".into(), id.into());
            }
            return Ok(());
        }
        if !matches!(
            event,
            "save_currencies"
                | "create_category"
                | "toggle_category"
                | "create_claim_type"
                | "toggle_claim_type"
                | "create_assignment"
                | "save_assignment_members"
                | "end_assignment"
                | "create_policy"
                | "end_policy"
                | "confirm_end"
        ) {
            return Err(Error::Invalid);
        }
        // Every write-shaped intent rechecks now, before reading any posted values.
        if !self.permitted(scope).await? {
            return Ok(());
        }
        if event == "confirm_end" {
            return self.confirm(scope).await;
        }
        let Some(company) = self.company else {
            return Ok(());
        };
        self.errors.clear();
        self.error_form = form_id(event, payload);
        self.drafts.insert(self.error_form.clone(), payload.clone());
        let (success, failure) = messages(event);
        let outcome: crate::Result<()> = match event {
            "save_currencies" => {
                let codes = text(payload, "currencies")
                    .split([',', ' ', '\n'])
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                self.claims
                    .put_currencies(scope, company, &codes)
                    .await
                    .map(|_| ())
            }
            "create_category" => self
                .claims
                .create_category(
                    scope,
                    company,
                    &NewCategory {
                        code: text(payload, "category[code]").into(),
                        name: text(payload, "category[name]").into(),
                    },
                )
                .await
                .map(|_| ()),
            "toggle_category" => {
                let category =
                    id(payload, "value").and_then(|id| self.categories.iter().find(|c| c.id == id));
                match category {
                    Some(c) => self
                        .claims
                        .set_category_active(scope, company, c.id, !c.active)
                        .await
                        .map(|_| ()),
                    None => Err(ClaimsError::NotFound),
                }
            }
            "create_claim_type" => self
                .claims
                .create_claim_type(
                    scope,
                    company,
                    &NewClaimType {
                        category_id: id(payload, "claim_type[category_id]").unwrap_or(0),
                        code: text(payload, "claim_type[code]").into(),
                        name: text(payload, "claim_type[name]").into(),
                        receipt_requirement: text(payload, "claim_type[receipt_requirement]")
                            .into(),
                        eligibility: Some(text(payload, "claim_type[eligibility]").into()),
                    },
                )
                .await
                .map(|_| ()),
            "toggle_claim_type" => {
                let claim_type =
                    id(payload, "value").and_then(|id| self.types.iter().find(|t| t.id == id));
                match claim_type {
                    Some(t) => self
                        .claims
                        .set_claim_type_active(scope, company, t.id, !t.active)
                        .await
                        .map(|_| ()),
                    None => Err(ClaimsError::NotFound),
                }
            }
            "create_assignment" => match parse_assignment(payload) {
                Ok(new) => self
                    .claims
                    .create_assignment(scope, company, &new)
                    .await
                    .map(|_| ()),
                Err(e) => Err(e),
            },
            "save_assignment_members" => {
                // Read these exact event keys; form_values/string_values truncate after 64 fields.
                // The multi-selects post arrays under filters[key][]. Empty arrays clear membership.
                match (
                    ids(payload, "filters[claim_type_ids][]"),
                    ids(payload, "filters[employee_ids][]"),
                ) {
                    (Ok(types), Ok(employees)) => self
                        .claims
                        .set_assignment_members(
                            scope,
                            company,
                            id(payload, "assignment_id").unwrap_or(0),
                            &types,
                            &employees,
                        )
                        .await
                        .map(|_| ()),
                    _ => Err(ClaimsError::EmployeeNotFound),
                }
            }
            "create_policy" => match parse_policy(payload) {
                Ok(new) => self
                    .claims
                    .create_policy(scope, company, &new)
                    .await
                    .map(|_| ()),
                Err(e) => Err(e),
            },
            "end_policy" | "end_assignment" => return self.prepare_end(payload),
            _ => return Err(Error::Invalid),
        };
        match outcome {
            Ok(()) => {
                self.drafts.remove(&self.error_form);
                self.errors.clear();
                self.message(StatusKind::Success, success);
            }
            Err(error) => self.refused(error, failure),
        }
        Ok(())
    }
    fn prepare_end(&mut self, payload: &Value) -> Result<()> {
        let kind = if text(payload, "type") == "end_policy" {
            EndKind::Policy
        } else {
            EndKind::Assignment
        };
        let key = match kind {
            EndKind::Policy => "policy_id",
            EndKind::Assignment => "assignment_id",
        };
        let record = id(payload, key).and_then(|id| match kind {
            EndKind::Policy => self
                .policies
                .iter()
                .find(|p| p.id == id && p.effective_to.is_none())
                .map(|p| {
                    (
                        id,
                        type_name(&self.types, p.claim_type_id),
                        p.effective_from,
                    )
                }),
            EndKind::Assignment => self
                .assignments
                .iter()
                .find(|a| a.id == id && a.effective_to.is_none())
                .map(|a| (id, a.name.clone(), a.effective_from)),
        });
        let result = date(payload, "effective_to").and_then(|date| {
            let (id, name, from) = record.ok_or(ClaimsError::NotFound)?;
            if date < from {
                return Err(invalid("effective_to", "must not be before the start date"));
            }
            Ok(PendingEnd {
                company: self.company.ok_or(ClaimsError::NotFound)?,
                id,
                name,
                date,
                kind,
            })
        });
        match result {
            Ok(pending) => self.pending = Some(pending),
            Err(e) => self.refused(e, "Choose an end date on or after the start date."),
        }
        Ok(())
    }
    async fn confirm(&mut self, scope: &Scope) -> Result<()> {
        let Some(pending) = self.pending.take() else {
            return Ok(());
        };
        let result = match pending.kind {
            EndKind::Policy => self
                .claims
                .end_policy(scope, pending.company, pending.id, pending.date)
                .await
                .map(|_| ()),
            EndKind::Assignment => self
                .claims
                .end_assignment(scope, pending.company, pending.id, pending.date)
                .await
                .map(|_| ()),
        };
        match result {
            Ok(()) => {
                self.drafts.remove(&self.error_form);
                self.errors.clear();
                self.message(
                    StatusKind::Success,
                    match pending.kind {
                        EndKind::Policy => "Policy end date saved.",
                        EndKind::Assignment => "Assignment end date saved.",
                    },
                );
            }
            Err(e) => self.refused(e, "Choose an end date on or after the start date."),
        }
        Ok(())
    }
    fn field(&self, form: &str, name: &str, label: &str, kind: InputKind) -> Field {
        let mut field = Field::named(name)
            .id(field_id(form, name))
            .label(label)
            .wrapper_class("mb-0")
            .kind(kind);
        if let Some(payload) = self.drafts.get(form) {
            field = field.value(text(payload, name));
        }
        if self.error_form == form {
            let key = name
                .rsplit('[')
                .next()
                .unwrap_or(name)
                .trim_end_matches(']');
            field = field.errors(
                self.errors
                    .iter()
                    .filter(|e| e.field == key)
                    .map(|e| e.message.clone())
                    .collect(),
            );
        }
        field
    }
    fn form(
        &self,
        id: &str,
        event: &str,
        label: &str,
        fields: Vec<Field>,
        extra: Html,
    ) -> Result<Html> {
        let mut body = Html::EMPTY;
        if self.error_form == id && !self.errors.is_empty() {
            let mut summary = ErrorSummary::new(format!("{id}-errors"));
            for field in &fields {
                for error in &field.errors {
                    summary.push(&field.id, bilibi_ui::error_sentence(&field.label, error));
                }
            }
            body.push(&render(&summary)?);
        }
        for field in fields {
            body.push(&render(&field)?);
        }
        body.push(&extra);
        render(&Form {
            id,
            event,
            change: false,
            class: "grid gap-2 sm:grid-cols-2",
            body,
            actions: render(&Button {
                button_type: "submit",
                variant: ButtonVariant::Primary,
                body: Html::text(label),
                ..Button::default()
            })?,
        })
    }
    fn add_form(&self, section: usize) -> Result<Html> {
        let (id, event, label) = match section {
            0 => (
                "claim-currencies-form",
                "save_currencies",
                "Save currencies",
            ),
            1 => ("claim-category-form", "create_category", "Add category"),
            2 => ("claim-type-form", "create_claim_type", "Add claim type"),
            3 => (
                "claim-assignment-form",
                "create_assignment",
                "Add assignment",
            ),
            _ => ("claim-policy-form", "create_policy", "Add policy"),
        };
        let f = |name: &str, label: &str, kind: InputKind| self.field(id, name, label, kind);
        let code_name = |namespace: &str| {
            vec![
                f(&format!("{namespace}[code]"), "Code", InputKind::Text)
                    .required()
                    .attr("maxlength", "60"),
                f(&format!("{namespace}[name]"), "Name", InputKind::Text)
                    .required()
                    .attr("maxlength", "120"),
            ]
        };
        let fields = match section {
            0 => vec![{
                let mut field = f("currencies", "Currency codes", InputKind::Text)
                    .placeholder("Three-letter codes, comma separated");
                if !self.drafts.contains_key(id) {
                    field = field.value(self.currencies.join(", "));
                }
                field
            }],
            1 => code_name("category"),
            2 => {
                if self.categories.is_empty() {
                    return Ok(Html::text("Add a category before adding claim types."));
                }
                let mut fields = vec![f(
                    "claim_type[category_id]",
                    "Claim category",
                    InputKind::Select,
                )
                .options(
                    self.categories
                        .iter()
                        .map(|c| Choice::new(&c.name, c.id.to_string()))
                        .collect(),
                )];
                fields.extend(code_name("claim_type"));
                fields.push(
                    f(
                        "claim_type[receipt_requirement]",
                        "Receipt requirement",
                        InputKind::Select,
                    )
                    .options(
                        crate::RECEIPT_REQUIREMENTS
                            .iter()
                            .map(|r| Choice::new(receipt_label(r), *r))
                            .collect(),
                    ),
                );
                fields.push(
                    f(
                        "claim_type[eligibility]",
                        "Who may claim",
                        InputKind::Select,
                    )
                    .options(
                        crate::ELIGIBILITIES
                            .iter()
                            .map(|e| Choice::new(eligibility_label(e), *e))
                            .collect(),
                    ),
                );
                fields
            }
            3 => {
                let mut fields = code_name("assignment");
                fields.push(
                    f(
                        "assignment[effective_from]",
                        "Assignment effective from",
                        InputKind::Date,
                    )
                    .required(),
                );
                fields.push(f(
                    "assignment[effective_to]",
                    "Assignment effective to",
                    InputKind::Date,
                ));
                fields
            }
            _ => {
                if self.types.is_empty() || self.currencies.is_empty() {
                    return Ok(Html::text(
                        "Add a claim type and a claim currency before adding policies.",
                    ));
                }
                let mut fields = vec![
                    f(
                        "policy[claim_type_id]",
                        "Policy claim type",
                        InputKind::Select,
                    )
                    .options(
                        self.types
                            .iter()
                            .map(|t| Choice::new(&t.name, t.id.to_string()))
                            .collect(),
                    ),
                    f("policy[effective_from]", "Effective from", InputKind::Date).required(),
                    f("policy[effective_to]", "Effective to", InputKind::Date),
                    f("policy[currency]", "Policy currency", InputKind::Select)
                        .options(self.currencies.iter().map(|c| Choice::new(c, c)).collect()),
                ];
                for (key, label) in [
                    ("per_claim_limit", "Per-claim limit"),
                    ("monthly_limit", "Monthly limit"),
                    ("yearly_limit", "Yearly limit"),
                    ("receipt_threshold", "Receipt above (threshold types)"),
                ] {
                    fields.push(
                        f(&format!("policy[{key}]"), label, InputKind::Text)
                            .attr("inputmode", "decimal"),
                    );
                }
                fields
            }
        };
        self.form(id, event, label, fields, Html::EMPTY)
    }
    fn end_form(&self, id: i64, kind: EndKind) -> Result<Html> {
        let (key, event, label) = match kind {
            EndKind::Policy => ("policy_id", "end_policy", "Policy end date"),
            EndKind::Assignment => ("assignment_id", "end_assignment", "Assignment end date"),
        };
        let form = format!("claim-{event}-{id}");
        let mut body = render(
            &Field::named(key)
                .id(format!("{form}-id"))
                .kind(InputKind::Hidden)
                .value(id.to_string()),
        )?;
        body.push(&render(
            &self
                .field(&form, "effective_to", label, InputKind::Date)
                .required(),
        )?);
        render(&Form {
            id: &form,
            event,
            change: false,
            class: "flex gap-1",
            body,
            actions: render(&Button {
                button_type: "submit",
                body: Html::text("End"),
                ..Button::default()
            })?,
        })
    }
    fn member_form(&self, assignment: &Assignment) -> Result<Html> {
        let form = format!("claim-assignment-members-{}", assignment.id);
        let draft = self.drafts.get(&form);
        let values = |key: &str, saved: &[i64]| -> Vec<String> {
            if let Some(payload) = draft {
                ids(payload, key)
                    .unwrap_or_default()
                    .iter()
                    .map(i64::to_string)
                    .collect()
            } else {
                saved.iter().map(i64::to_string).collect()
            }
        };
        let mut body = render(
            &Field::named("assignment_id")
                .id(format!("{form}-id"))
                .kind(InputKind::Hidden)
                .value(assignment.id.to_string()),
        )?;
        body.push(&render(
            &MultiSelect::new(format!("{form}-types"), "filters[claim_type_ids][]")
                .label("Claim types")
                .placeholder("No claim types selected")
                .options(
                    self.types
                        .iter()
                        .map(|t| Choice::new(&t.name, t.id.to_string()))
                        .collect(),
                )
                .values(values(
                    "filters[claim_type_ids][]",
                    &assignment.claim_type_ids,
                )),
        )?);
        body.push(&render(
            &MultiSelect::new(format!("{form}-employees"), "filters[employee_ids][]")
                .label("Employees")
                .placeholder("No employees selected")
                .options(
                    self.employees
                        .iter()
                        .map(|e| {
                            Choice::new(
                                format!("{} {}", e.name, e.employee_number),
                                e.id.to_string(),
                            )
                        })
                        .collect(),
                )
                .values(values("filters[employee_ids][]", &assignment.employee_ids)),
        )?);
        self.form(
            &form,
            "save_assignment_members",
            "Save members",
            vec![],
            body,
        )
    }
    fn rows(&self, section: usize) -> Result<Vec<FlexRow>> {
        match section {
            0 => Ok(self
                .currencies
                .iter()
                .map(|c| FlexRow::new(c).slot("currency", Html::text(c)))
                .collect()),
            1 => self
                .categories
                .iter()
                .map(|c| {
                    let mut row = FlexRow::new(format!("claim-category-{}", c.id))
                        .slot("name", Html::text(&c.name))
                        .slot("code", Html::text(&c.code))
                        .slot("status", Html::text(status(c.active)));
                    row.actions = action(
                        "toggle_category",
                        if c.active { "Deactivate" } else { "Activate" },
                        c.id,
                    )?;
                    Ok(row)
                })
                .collect(),
            2 => self
                .types
                .iter()
                .map(|t| {
                    let category = self
                        .categories
                        .iter()
                        .find(|c| c.id == t.category_id)
                        .map_or("—", |c| c.name.as_str());
                    let mut row = FlexRow::new(format!("claim-type-{}", t.id))
                        .slot("name", Html::text(&t.name))
                        .slot("code", Html::text(&t.code))
                        .slot("category", Html::text(category))
                        .slot("receipt", Html::text(receipt_label(&t.receipt_requirement)))
                        .slot("eligibility", Html::text(eligibility_label(&t.eligibility)))
                        .slot("status", Html::text(status(t.active)));
                    row.actions = action(
                        "toggle_claim_type",
                        if t.active { "Deactivate" } else { "Activate" },
                        t.id,
                    )?;
                    Ok(row)
                })
                .collect(),
            3 => self
                .assignments
                .iter()
                .map(|a| {
                    let members = if self.columns[3].mode() == bilibi_ui::RowMode::Compact {
                        Html::text(&format!(
                            "{} types · {} employees",
                            a.claim_type_ids.len(),
                            a.employee_ids.len()
                        ))
                    } else {
                        self.member_form(a)?
                    };
                    let mut row = FlexRow::new(format!("claim-assignment-{}", a.id))
                        .slot("name", Html::text(&a.name))
                        .slot("code", Html::text(&a.code))
                        .slot("from", Html::text(&a.effective_from.to_string()))
                        .slot("to", Html::text(&end_date(a.effective_to)))
                        .slot("members", members);
                    row.dom_id = Some(format!("claim-assignment-{}", a.id));
                    if a.effective_to.is_none() {
                        row.actions = self.end_form(a.id, EndKind::Assignment)?;
                    }
                    Ok(row)
                })
                .collect(),
            _ => self
                .policies
                .iter()
                .map(|p| {
                    let mut row = FlexRow::new(format!("claim-policy-{}", p.id))
                        .slot("type", Html::text(&type_name(&self.types, p.claim_type_id)))
                        .slot("from", Html::text(&p.effective_from.to_string()))
                        .slot("to", Html::text(&end_date(p.effective_to)))
                        .slot("currency", Html::text(&p.currency));
                    for (key, value) in [
                        ("per_claim_limit", p.per_claim_limit),
                        ("monthly_limit", p.monthly_limit),
                        ("yearly_limit", p.yearly_limit),
                        ("receipt_threshold", p.receipt_threshold),
                    ] {
                        row = row.slot(
                            key,
                            Html::text(&value.map_or_else(|| "—".into(), |v| v.to_string())),
                        );
                    }
                    row.dom_id = Some(format!("claim-policy-{}", p.id));
                    if p.effective_to.is_none() {
                        row.actions = self.end_form(p.id, EndKind::Policy)?;
                    }
                    Ok(row)
                })
                .collect(),
        }
    }
    fn table(&self, section: usize, title: &str, empty: &str, empty_id: &str) -> Result<Html> {
        let columns = &self.columns[section];
        let mut table = FlexTable::new(
            TABLES[section].0,
            columns.flex_columns(),
            self.rows(section)?,
        );
        table.caption = title;
        table.event = TABLES[section].1;
        table.zoom = columns.zoom;
        table.suggestions = columns.flex_suggestions();
        table.add_query = &columns.add_query;
        table.actions = section != 0;
        table.empty = Some(TableEmpty::State(
            EmptyState::missing(empty, "").id(empty_id),
        ));
        render(&table)
    }
    fn render(&self) -> Result<String> {
        let company = render(&Form {
            id: "claim-setup-company",
            event: "select_company",
            change: true,
            class: "",
            body: render(
                &Field::named("company_id")
                    .id("claim-setup-company-select")
                    .label("Company")
                    .kind(InputKind::Select)
                    .value(self.company.map_or_else(String::new, |id| id.to_string()))
                    .options(
                        self.companies
                            .iter()
                            .map(|c| Choice::new(&c.name, c.id.to_string()))
                            .collect(),
                    ),
            )?,
            actions: Html::EMPTY,
        })?;
        let metadata = [
            ("claim-currencies-heading", "Claim currencies", "Policies and claims use only these codes. With none, no claim can be submitted.", "No claim currencies have been added for this company.", "claim-currencies-empty"),
            ("claim-categories-heading", "Categories", "", "No claim categories have been added for this company.", "claim-categories-empty"),
            ("claim-types-heading", "Claim types", "", "No claim types have been added for this company.", "claim-types-empty"),
            ("claim-assignments-heading", "Assignments", "An assignment opens assigned-only claim types for the employees it covers while it is in effect.", "No claim assignments have been added for this company.", "claim-assignments-empty"),
            ("claim-policies-heading", "Policies", "A claim is checked against the policy in effect on its expense date. Periods of one claim type never overlap.", "No claim policies have been added for this company.", "claim-policies-empty"),
        ];
        let sections = if self.company.is_some() {
            metadata
                .iter()
                .enumerate()
                .map(|(i, (id, title, description, empty, empty_id))| {
                    Ok(Section {
                        id,
                        title,
                        description,
                        table: self.table(i, title, empty, empty_id)?,
                        form: self.add_form(i)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?
        } else {
            vec![]
        };
        let confirmation = if let Some(p) = &self.pending {
            let subject = match p.kind {
                EndKind::Policy => "policy",
                EndKind::Assignment => "assignment",
            };
            let consequence = format!("The {subject} for {} will end on {}.", p.name, p.date);
            let detail = self.instance.as_ref().map_or_else(
                || "This end date cannot be changed.".into(),
                |i| format!("{} This end date cannot be changed.", i.notice()),
            );
            render(&ConfirmDialog {
                id: "claim-setup-confirm",
                consequence: &consequence,
                detail: &detail,
                confirm: "End",
                working: "Saving…",
                confirm_event: "confirm_end",
                cancel_event: "cancel_end",
            })?
        } else {
            Html::EMPTY
        };
        View {
            flash: self.flash(),
            company,
            available: self.company.is_some(),
            sections,
            confirmation,
            empty: render(
                &EmptyState::missing("No company is available for claim setup.", "")
                    .id("claim-setup-empty"),
            )?,
        }
        .render()
        .map_err(|_| Error::Internal)
    }
}
impl LiveState<Scope, Error> for Setup {
    fn update<'a>(&'a mut self, request: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            match request.input {
                LiveInput::Refresh => {}
                LiveInput::Params(query) => {
                    self.clear_forms();
                    self.list = ListState::parse(query, spec()?).map_err(|_| Error::Internal)?;
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
                rows: self.policies.len(),
                ..LiveBody::default()
            })
        })
    }
}
struct Section {
    id: &'static str,
    title: &'static str,
    description: &'static str,
    table: Html,
    form: Html,
}
#[derive(Template)]
#[template(path = "setup.html")]
struct View<'a> {
    flash: FlashMap<'a>,
    company: Html,
    available: bool,
    sections: Vec<Section>,
    empty: Html,
    confirmation: Html,
}
fn render(template: &impl Template) -> Result<Html> {
    Html::render(template).map_err(|_| Error::Internal)
}
fn text<'a>(payload: &'a Value, key: &str) -> &'a str {
    payload.get(key).and_then(Value::as_str).unwrap_or("")
}
fn id(payload: &Value, key: &str) -> Option<i64> {
    params::positive_id(payload.get(key).and_then(Value::as_str))
}
fn field_id(form: &str, name: &str) -> String {
    format!("{form}-{}", name.replace(['[', ']'], "-"))
}
fn invalid(field: &'static str, message: &str) -> ClaimsError {
    ClaimsError::Invalid(vec![FieldError::new(field, message)])
}
fn date(payload: &Value, key: &'static str) -> crate::Result<Date> {
    text(payload, key).parse().map_err(|_| {
        invalid(
            if key.ends_with("[effective_from]") {
                "effective_from"
            } else {
                "effective_to"
            },
            "is invalid",
        )
    })
}
fn optional_date(payload: &Value, key: &'static str) -> crate::Result<Option<Date>> {
    if text(payload, key).is_empty() {
        Ok(None)
    } else {
        date(payload, key).map(Some)
    }
}
fn parse_assignment(p: &Value) -> crate::Result<NewAssignment> {
    Ok(NewAssignment {
        code: text(p, "assignment[code]").into(),
        name: text(p, "assignment[name]").into(),
        effective_from: date(p, "assignment[effective_from]")?,
        effective_to: optional_date(p, "assignment[effective_to]")?,
    })
}
fn parse_policy(p: &Value) -> crate::Result<NewPolicy> {
    let amount = |key: &'static str| -> crate::Result<Option<Decimal>> {
        let name = format!("policy[{key}]");
        let value = text(p, &name).trim();
        if value.is_empty() {
            Ok(None)
        } else {
            value
                .parse::<Decimal>()
                .map(Some)
                .map_err(|_| invalid(key, "is invalid"))
        }
    };
    Ok(NewPolicy {
        claim_type_id: id(p, "policy[claim_type_id]").unwrap_or(0),
        effective_from: date(p, "policy[effective_from]")?,
        effective_to: optional_date(p, "policy[effective_to]")?,
        currency: text(p, "policy[currency]").into(),
        per_claim_limit: amount("per_claim_limit")?,
        monthly_limit: amount("monthly_limit")?,
        yearly_limit: amount("yearly_limit")?,
        receipt_threshold: amount("receipt_threshold")?,
    })
}
/// Walk exact keys without the generic 64-field truncation; bound array membership.
fn ids(payload: &Value, key: &str) -> std::result::Result<Vec<i64>, ()> {
    let Some(value) = payload.get(key) else {
        return Ok(vec![]);
    };
    let values: Vec<&Value> = match value {
        Value::Array(values) if values.len() <= crate::MEMBER_LIMIT => values.iter().collect(),
        Value::String(_) => vec![value],
        _ => return Err(()),
    };
    values
        .into_iter()
        .map(|v| params::positive_id(v.as_str()).ok_or(()))
        .collect()
}
fn action(event: &str, label: &str, id: i64) -> Result<Html> {
    render(&Button {
        event,
        name: "value",
        value: &id.to_string(),
        body: Html::text(label),
        ..Button::default()
    })
}
fn status(active: bool) -> &'static str {
    if active {
        "Active"
    } else {
        "Inactive"
    }
}
fn receipt_label(rule: &str) -> &'static str {
    match rule {
        "always" => "Always",
        "above_threshold" => "Above threshold",
        _ => "Never",
    }
}
fn eligibility_label(rule: &str) -> &'static str {
    if rule == "assigned_only" {
        "Assigned employees only"
    } else {
        "All working employees"
    }
}
fn end_date(date: Option<Date>) -> String {
    date.map_or_else(|| "open".into(), |d| d.to_string())
}
fn type_name(types: &[ClaimType], id: i64) -> String {
    types
        .iter()
        .find(|t| t.id == id)
        .map_or_else(|| "—".into(), |t| t.name.clone())
}
fn form_id(event: &str, payload: &Value) -> String {
    match event {
        "save_currencies" => "claim-currencies-form".into(),
        "create_category" => "claim-category-form".into(),
        "create_claim_type" => "claim-type-form".into(),
        "create_assignment" => "claim-assignment-form".into(),
        "create_policy" => "claim-policy-form".into(),
        "save_assignment_members" => format!(
            "claim-assignment-members-{}",
            text(payload, "assignment_id")
        ),
        "end_assignment" => format!("claim-end_assignment-{}", text(payload, "assignment_id")),
        "end_policy" => format!("claim-end_policy-{}", text(payload, "policy_id")),
        _ => event.into(),
    }
}
fn messages(event: &str) -> (&'static str, &'static str) {
    match event {
        "save_currencies" => (
            "Claim currencies saved.",
            "Enter up to 20 three-letter currency codes.",
        ),
        "create_category" => (
            "Category added.",
            "Check the category fields and unique code.",
        ),
        "toggle_category" => ("Category updated.", "Category unavailable."),
        "create_claim_type" => (
            "Claim type added.",
            "Choose a category and check the claim type fields and unique code.",
        ),
        "toggle_claim_type" => ("Claim type updated.", "Claim type unavailable."),
        "create_assignment" => (
            "Assignment added.",
            "Check the assignment fields, its dates, and unique code.",
        ),
        "save_assignment_members" => (
            "Assignment members saved.",
            "Choose claim types and employees of this company.",
        ),
        "create_policy" => (
            "Policy added.",
            "Check the dates, an allowed currency, positive limits, and any receipt threshold.",
        ),
        _ => ("", "Choose an end date on or after the start date."),
    }
}
