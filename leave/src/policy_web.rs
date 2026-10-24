//! Bilimbi `People.Leave.Web.PoliciesLive`: company year, rules, types,
//! entitlement versions, grants and operator-delegated carry-forward.
//! The facade owns each business rule. Selections awaiting confirmation
//! live here, never in the confirmation event's untrusted payload.
use crate::{
    CarryForwardSkipped, Leave, LeaveError, LeaveType, NewLeaveType, NewPolicy, Policy,
    RequestRules, Rules, POLICIES_MANAGE,
};
use askama::Template;
use bilibi_database::{Db, Error, Result};
use bilibi_people_workforce::{authorization, CompanySummary, WorkforceError};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    choices,
    page::{
        LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState, MountFuture, Page, PageBody,
        PageFuture, PageRequest,
    },
    params, BuiltinColumn, Button, ButtonVariant, Card, ConfirmDialog, EmptyState, Field, FlashMap,
    FlexRow, FlexTable, Form, Html, InputKind, ListColumns, ListSpec, ListState, StatusKind,
    TableEmpty, NO_FLASH,
};
use rust_decimal::Decimal;
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

pub(super) const SERVICE: &str = "people/leave.policy_pages";
const FORBIDDEN: &str = "You no longer have permission to change this company's leave policies.";
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

pub(super) struct PoliciesPage(Arc<Core>);
struct Core {
    db: Arc<Db>,
    leave: Arc<Leave>,
}
impl PoliciesPage {
    pub(super) fn new(db: Arc<Db>, leave: Arc<Leave>) -> Self {
        Self(Arc::new(Core { db, leave }))
    }
}
impl Page<Scope, Error> for PoliciesPage {
    fn title(&self, _: &str) -> Option<&'static str> {
        Some("Leave policies")
    }
    fn render<'a>(&'a self, request: PageRequest<'a, Scope>) -> PageFuture<'a, Error> {
        Box::pin(async move {
            let mut state = Policies::new(self.0.clone(), request.query)?;
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
                Box::new(Policies::new(self.0.clone(), request.query)?)
                    as Box<dyn LiveState<Scope, Error>>,
            ))
        })
    }
}

#[derive(Template)]
#[template(path = "policies.html")]
struct View<'a> {
    flash: FlashMap<'a>,
    subtitle: String,
    chooser: Html,
    state: Html,
    available: bool,
    sections: Vec<Html>,
    versions: Html,
    confirmation: Html,
}
#[derive(Template)]
#[template(path = "policy_fields.html")]
struct Fields {
    fields: Vec<Field>,
}
#[derive(Template)]
#[template(path = "policy_note.html")]
struct Note<'a> {
    id: &'a str,
    text: &'a str,
}
fn render(view: &impl Template) -> Result<Html> {
    Html::render(view).map_err(|_| Error::Internal)
}

/// Bilimbi has no sorting, filtering or pagination on these three lists.
/// Their ListState keeps that contract; ListColumns owns the arrangement.
struct Table {
    columns: ListColumns,
    state: ListState,
    event: &'static str,
    caption: &'static str,
}
impl Table {
    fn new(
        id: &str,
        caption: &'static str,
        event: &'static str,
        names: &[(&str, &str)],
        query: &HashMap<String, String>,
    ) -> Result<Self> {
        let spec = Arc::new(ListSpec {
            page_sizes: vec![1000],
            default_page_size: 1000,
            omit_defaults: true,
            param_prefix: format!("{id}_"),
            ..ListSpec::default()
        });
        Ok(Self {
            columns: ListColumns::mount(
                id,
                names
                    .iter()
                    .map(|(id, label)| {
                        let column = BuiltinColumn::new(id, label);
                        if ["version", "entitlement", "cap", "year"].contains(id) {
                            column.right()
                        } else {
                            column
                        }
                    })
                    .collect(),
            )
            .map_err(|_| Error::Internal)?,
            state: ListState::parse(query, spec).map_err(|_| Error::Internal)?,
            event,
            caption,
        })
    }
    fn render(&self, mut rows: Vec<FlexRow>, empty: &str) -> Result<Html> {
        // Keep Bilimbi's persisted DOM anchors when replacing its ul/table.
        for row in &mut rows {
            row.dom_id = Some(row.key.clone());
        }
        let none = rows.is_empty();
        let mut table = FlexTable::new(&self.columns.id, self.columns.flex_columns(), rows)
            .sorted(self.state.sort_by.as_deref(), self.state.sort_dir);
        table.event = self.event;
        table.caption = self.caption;
        table.zoom = self.columns.zoom;
        table.suggestions = self.columns.flex_suggestions();
        table.add_query = &self.columns.add_query;
        table.framed = false;
        if none {
            table.empty = Some(TableEmpty::State(EmptyState::missing(empty, "").id(
                match self.columns.id.as_str() {
                    "leave-types" => "",
                    "leave-policy-versions" => "leave-policies-none",
                    _ => "",
                },
            )));
        }
        render(&table)
    }
}

enum Pending {
    Grant {
        company: i64,
        year: i32,
    },
    Carry {
        company: i64,
        year: i32,
    },
    Status {
        company: i64,
        id: i64,
        name: String,
        status: &'static str,
    },
}
struct Policies {
    core: Arc<Core>,
    company_id: Option<i64>,
    initialized: bool,
    companies: Vec<CompanySummary>,
    rules: Option<Rules>,
    request_rules: Option<RequestRules>,
    types: Vec<LeaveType>,
    policies: Vec<Policy>,
    skipped: CarryForwardSkipped,
    current_year: i32,
    carried: i64,
    types_table: Table,
    policies_table: Table,
    skips_table: Table,
    forms: HashMap<String, HashMap<String, String>>,
    pending: Option<Pending>,
    flash: Option<(StatusKind, String)>,
}
impl Policies {
    fn new(core: Arc<Core>, query: &HashMap<String, String>) -> Result<Self> {
        Ok(Self {
            core,
            company_id: params::positive_id(query.get("company_id").map(String::as_str)),
            initialized: false,
            companies: vec![],
            rules: None,
            request_rules: None,
            types: vec![],
            policies: vec![],
            skipped: CarryForwardSkipped {
                skips: vec![],
                total: 0,
            },
            current_year: 0,
            carried: 0,
            types_table: Table::new(
                "leave-types",
                "Leave types",
                "types_grid",
                &[
                    ("name", "Name"),
                    ("code", "Code"),
                    ("unit", "Unit"),
                    ("paid", "Paid"),
                    ("balance", "Balance"),
                    ("status", "Status"),
                    ("actions", "Actions"),
                ],
                query,
            )?,
            policies_table: Table::new(
                "leave-policy-versions",
                "Entitlement policy versions",
                "policies_grid",
                &[
                    ("type", "Leave type"),
                    ("version", "Version"),
                    ("from", "Effective from"),
                    ("to", "Effective to"),
                    ("entitlement", "Entitlement per year"),
                    ("cap", "Carry-forward cap"),
                ],
                query,
            )?,
            skips_table: Table::new(
                "leave-carry-skipped",
                "Skipped balances",
                "skips_grid",
                &[
                    ("year", "Skipped from leave year"),
                    ("employee", "Employee"),
                    ("type", "Leave type"),
                    ("reason", "Reason"),
                ],
                query,
            )?,
            forms: HashMap::new(),
            pending: None,
            flash: None,
        })
    }
    async fn access(&mut self, scope: &Scope) -> Result<()> {
        let client = self.core.db.get().await?;
        self.companies =
            match authorization::selectable_companies(&**client, scope, POLICIES_MANAGE).await {
                Ok(rows) => rows
                    .into_iter()
                    .filter(|company| !company.archived)
                    .collect(),
                Err(WorkforceError::Failed(error)) => return Err(error),
                Err(_) => vec![],
            };
        if !self.initialized {
            if !self.companies.iter().any(|c| Some(c.id) == self.company_id) {
                self.company_id = self.companies.first().map(|c| c.id);
            }
            self.initialized = true;
        }
        if !self.companies.iter().any(|c| Some(c.id) == self.company_id) {
            self.clear_records();
            self.pending = None;
        }
        Ok(())
    }
    fn company(&self) -> Option<&CompanySummary> {
        self.companies
            .iter()
            .find(|c| Some(c.id) == self.company_id)
    }
    fn clear_records(&mut self) {
        self.rules = None;
        self.request_rules = None;
        self.types.clear();
        self.policies.clear();
        self.skipped = CarryForwardSkipped {
            skips: vec![],
            total: 0,
        };
        self.carried = 0;
    }
    async fn load(&mut self, scope: &Scope) -> Result<()> {
        self.access(scope).await?;
        let Some(company_id) = self.company().map(|c| c.id) else {
            return Ok(());
        };
        // Stage the read: an unavailable workforce never leaves a partial page.
        let result = async {
            let leave = &self.core.leave;
            let rules = leave.rules(scope, company_id).await?;
            let requests = leave.request_rules(scope, company_id).await?;
            let types = leave.list_types(scope, company_id).await?;
            let policies = leave.list_policies(scope, company_id).await?;
            let today = leave.today(scope, company_id).await?;
            let year = rules.leave_year(today);
            let carried = leave
                .carried_forward_count(scope, company_id, year - 1)
                .await?;
            let skipped = leave.carry_forward_skipped(scope, company_id).await?;
            crate::Result::Ok((rules, requests, types, policies, year, carried, skipped))
        }
        .await;
        match result {
            Ok((rules, requests, types, policies, year, carried, skipped)) => {
                self.rules = Some(rules);
                self.request_rules = Some(requests);
                self.types = types;
                self.policies = policies;
                self.current_year = year;
                self.carried = carried;
                self.skipped = skipped;
                self.types_table
                    .columns
                    .load(&self.types, |t| t.id.to_string());
                self.policies_table
                    .columns
                    .load(&self.policies, |p| p.id.to_string());
                self.skips_table.columns.load(&self.skipped.skips, |s| {
                    format!("{}-{}-{}", s.from_year, s.employee_id, s.leave_type_id)
                });
            }
            Err(LeaveError::Failed(error)) => return Err(error),
            Err(_) => self.clear_records(),
        }
        Ok(())
    }
    fn error(&mut self, error: LeaveError, fallback: &str) {
        let message = match error {
            LeaveError::Unauthorized => FORBIDDEN.into(),
            LeaveError::YearInUse
            | LeaveError::NotAfterLatest
            | LeaveError::GrantedAfterEffectiveDate
            | LeaveError::CompanyArchived => error.to_string(),
            _ => fallback.into(),
        };
        self.flash = Some((StatusKind::Error, message));
    }
    async fn event(&mut self, scope: &Scope, payload: &Value) -> Result<()> {
        let kind = payload.get("type").and_then(Value::as_str).unwrap_or("");
        self.access(scope).await?;
        match kind {
            "clear-flash" => {
                if self.flash.as_ref().is_some_and(|(k, _)| {
                    Some(k.name()) == payload.get("key").and_then(Value::as_str)
                }) {
                    self.flash = None;
                }
                return Ok(());
            }
            "types_grid" | "policies_grid" | "skips_grid" => {
                let table = match kind {
                    "types_grid" => &mut self.types_table,
                    "policies_grid" => &mut self.policies_table,
                    _ => &mut self.skips_table,
                };
                table.columns.handle(&params::string_values(payload));
                return Ok(());
            }
            "cancel_confirmation" => {
                self.pending = None;
                return Ok(());
            }
            "select_company" => {
                let id = params::positive_id(payload.get("company_id").and_then(Value::as_str));
                if self.companies.iter().any(|c| Some(c.id) == id) {
                    self.company_id = id;
                    self.pending = None;
                    self.forms.clear();
                    self.clear_records();
                    self.load(scope).await?;
                }
                return Ok(());
            }
            _ => {}
        }
        // Every write-shaped event rechecks both the capability and company axis
        // before parsing input; the facade proves them again in its transaction.
        let Some(company_id) = self.company().map(|c| c.id) else {
            self.pending = None;
            self.flash = Some((StatusKind::Error, FORBIDDEN.into()));
            return Ok(());
        };
        let client = self.core.db.get().await?;
        if authorization::authorize(&**client, scope, company_id, POLICIES_MANAGE)
            .await
            .is_err()
        {
            self.pending = None;
            self.flash = Some((StatusKind::Error, FORBIDDEN.into()));
            return Ok(());
        }
        drop(client);
        // Forms have a small fixed vocabulary. Keep overlong strings intact so
        // the facade refuses them rather than treating a dropped cap as empty.
        let keys = [
            "year_start_month",
            "backdate_days",
            "from_year",
            "leave_year",
            "weekday_1",
            "weekday_2",
            "weekday_3",
            "weekday_4",
            "weekday_5",
            "weekday_6",
            "weekday_7",
            "type[code]",
            "type[name]",
            "type[unit]",
            "type[paid]",
            "type[balance_required]",
            "policy[leave_type_id]",
            "policy[effective_from]",
            "policy[entitlement]",
            "policy[carry_forward_cap]",
        ];
        let values: HashMap<String, String> = keys
            .into_iter()
            .filter_map(|key| {
                payload
                    .get(key)
                    .and_then(Value::as_str)
                    .map(|v| (key.to_owned(), v.to_owned()))
            })
            .collect();
        match kind {
            "carry_forward" | "grant" => {
                self.forms.insert(kind.into(), values.clone());
                let year = integer(
                    &values,
                    if kind == "grant" {
                        "leave_year"
                    } else {
                        "from_year"
                    },
                );
                if kind == "grant" {
                    if (1900..=9998).contains(&year) {
                        self.pending = Some(Pending::Grant {
                            company: company_id,
                            year,
                        });
                    } else {
                        self.error(
                            LeaveError::InvalidYear,
                            "Entitlements could not be granted for that year.",
                        );
                    }
                } else if (1900..=9997).contains(&year) {
                    self.pending = Some(Pending::Carry {
                        company: company_id,
                        year,
                    });
                } else {
                    self.error(
                        LeaveError::InvalidCarryForward,
                        "Carry-forward could not be queued for that year.",
                    );
                }
            }
            "set_status" => {
                if let Some(t) = self.types.iter().find(|t| {
                    Some(t.id) == params::positive_id(payload.get("value").and_then(Value::as_str))
                }) {
                    self.pending = Some(Pending::Status {
                        company: company_id,
                        id: t.id,
                        name: t.name.clone(),
                        status: if t.status == "active" {
                            "archived"
                        } else {
                            "active"
                        },
                    });
                } else {
                    self.error(LeaveError::NotFound, "The leave type could not be updated.");
                }
            }
            "confirm_action" => {
                match self.pending.take() {
                    Some(Pending::Grant { company, year }) if company == company_id => {
                        let values = HashMap::from([("leave_year".into(), year.to_string())]);
                        self.write(scope, company, "grant", &values).await;
                    }
                    Some(Pending::Carry { company, year }) if company == company_id => {
                        match self.core.leave.enqueue_carry_forward(scope, company, year).await {
                            Ok(_) => self.flash = Some((StatusKind::Success, "Carry-forward queued. Balances update once it runs; running it again changes nothing.".into())),
                            Err(e) => self.error(e, "Carry-forward could not be queued for that year."),
                        }
                    }
                    Some(Pending::Status {
                        company,
                        id,
                        status,
                        ..
                    }) if company == company_id => {
                        match self
                            .core
                            .leave
                            .set_type_status(scope, company, id, status)
                            .await
                        {
                            Ok(_) => {
                                self.flash =
                                    Some((StatusKind::Success, "Leave type updated.".into()))
                            }
                            Err(e) => self.error(e, "The leave type could not be updated."),
                        }
                    }
                    _ => {
                        self.flash =
                            Some((StatusKind::Error, "Choose an action to confirm.".into()))
                    }
                }
                self.load(scope).await?;
            }
            "save_year" | "save_request_rules" | "create_type" | "add_policy" => {
                self.forms.insert(kind.into(), values.clone());
                self.write(scope, company_id, kind, &values).await;
                self.load(scope).await?;
            }
            _ => return Err(Error::Invalid),
        }
        Ok(())
    }
    async fn write(
        &mut self,
        scope: &Scope,
        company: i64,
        kind: &str,
        v: &HashMap<String, String>,
    ) {
        let leave = &self.core.leave;
        let (outcome, success, fallback) = match kind {
            "save_year" => (
                leave
                    .put_rules(scope, company, integer(v, "year_start_month"))
                    .await
                    .map(|_| ()),
                "Leave year saved.".into(),
                "Choose a month for the leave year.",
            ),
            "save_request_rules" => {
                let weekdays = (1..=7)
                    .filter(|day| value(v, &format!("weekday_{day}")) == "true")
                    .collect();
                (
                    leave
                        .put_request_rules(
                            scope,
                            company,
                            &RequestRules {
                                working_weekdays: weekdays,
                                backdate_days: integer(v, "backdate_days"),
                            },
                        )
                        .await
                        .map(|_| ()),
                    "Request rules saved.".into(),
                    "Choose at least one working weekday and a backdating limit of 0 to 366 days.",
                )
            }
            "create_type" => (
                leave
                    .create_type(
                        scope,
                        company,
                        &NewLeaveType {
                            code: value(v, "type[code]").into(),
                            name: value(v, "type[name]").into(),
                            unit: value(v, "type[unit]").into(),
                            paid: value(v, "type[paid]") == "true",
                            balance_required: value(v, "type[balance_required]") == "true",
                        },
                    )
                    .await
                    .map(|_| ()),
                "Leave type added.".into(),
                "Enter a unique lowercase code, a name and a unit for the leave type.",
            ),
            "add_policy" => {
                let policy = (|| {
                    Ok(NewPolicy {
                        effective_from: value(v, "policy[effective_from]")
                            .trim()
                            .parse()
                            .map_err(|_| LeaveError::InvalidPolicy)?,
                        entitlement: value(v, "policy[entitlement]")
                            .trim()
                            .parse::<Decimal>()
                            .map_err(|_| LeaveError::InvalidPolicy)?,
                        carry_forward_cap: if value(v, "policy[carry_forward_cap]")
                            .trim()
                            .is_empty()
                        {
                            None
                        } else {
                            Some(
                                value(v, "policy[carry_forward_cap]")
                                    .trim()
                                    .parse::<Decimal>()
                                    .map_err(|_| LeaveError::InvalidPolicy)?,
                            )
                        },
                    })
                })();
                let result = match policy {
                    Ok(policy) => leave
                        .add_policy(
                            scope,
                            company,
                            integer64(v, "policy[leave_type_id]"),
                            &policy,
                        )
                        .await
                        .map(|_| ()),
                    Err(e) => Err(e),
                };
                (result, "Policy version added.".into(), "Choose an active leave type, a start date, an entitlement of 0 to 9999.99 and an optional carry-forward cap in the same range.")
            }
            "grant" => {
                let result = leave
                    .grant_entitlements(scope, company, integer(v, "leave_year"))
                    .await;
                let success = result.as_ref().map(|c| format!("{} entitlements granted; {} were already granted; {} skipped because the year is carried forward.",c.granted,c.existing,c.closed)).unwrap_or_default();
                (
                    result.map(|_| ()),
                    success,
                    "Entitlements could not be granted for that year.",
                )
            }
            _ => return,
        };
        match outcome {
            Ok(()) => {
                self.flash = Some((StatusKind::Success, success));
                self.forms.remove(kind);
            }
            Err(error) => self.error(error, fallback),
        }
    }
    fn form_value(&self, event: &str, key: &str, default: impl ToString) -> String {
        self.forms
            .get(event)
            .and_then(|v| v.get(key))
            .cloned()
            .unwrap_or_else(|| default.to_string())
    }
    fn field(
        &self,
        event: &str,
        key: &str,
        id: &str,
        label: &str,
        default: impl ToString,
    ) -> Field {
        Field::named(key)
            .id(id)
            .label(label)
            .value(self.form_value(event, key, default))
    }
    fn render(&self) -> Result<String> {
        let mut flash = NO_FLASH;
        if let Some((kind, message)) = &self.flash {
            match kind {
                StatusKind::Success => flash.success = message,
                _ => flash.error = message,
            }
        }
        let company = self.company();
        let chooser = if self.companies.is_empty() {
            Html::EMPTY
        } else {
            let options: Vec<_> = self
                .companies
                .iter()
                .map(|c| (c.name.clone(), c.id.to_string()))
                .collect();
            form(
                "leave-company-form",
                "select_company",
                vec![Field::named("company_id")
                    .id("leave-company")
                    .label("Company")
                    .kind(InputKind::Select)
                    .options(choices(&options))
                    // A removed target must not look like the first remaining company.
                    .prompt(if company.is_none() {
                        "Choose a company"
                    } else {
                        ""
                    })
                    .value(company.map(|c| c.id.to_string()).unwrap_or_default())],
                "Choose company",
                false,
                true,
            )?
        };
        let state = if company.is_none() && self.companies.is_empty() {
            render(
                &EmptyState::missing("No active company is available for leave policies.", "")
                    .id("leave-policies-empty"),
            )?
        } else if company.is_none() || self.rules.is_none() {
            render(
                &EmptyState::missing("Leave policies are unavailable for this company.", "")
                    .id("leave-policies-unavailable"),
            )?
        } else {
            Html::EMPTY
        };
        let mut sections = vec![];
        let mut versions = Html::EMPTY;
        if let (Some(rules), Some(requests)) = (&self.rules, &self.request_rules) {
            let months: Vec<_> = MONTHS
                .iter()
                .enumerate()
                .map(|(i, name)| (name.to_string(), (i + 1).to_string()))
                .collect();
            let mut year = form(
                "leave-year-form",
                "save_year",
                vec![self
                    .field(
                        "save_year",
                        "year_start_month",
                        "leave-year-start",
                        "Starts in",
                        rules.year_start_month,
                    )
                    .kind(InputKind::Select)
                    .options(choices(&months))],
                "Save leave year",
                false,
                false,
            )?;
            year.push(&form(
                "leave-grant-form",
                "grant",
                vec![self
                    .field(
                        "grant",
                        "leave_year",
                        "leave-grant-year",
                        "Grant entitlements for leave year",
                        self.current_year,
                    )
                    .kind(InputKind::Number)
                    .required()
                    .attr("min", "1900")
                    .attr("max", "9998")],
                "Grant",
                self.policies.is_empty(),
                false,
            )?);
            year.push(&note("leave-grant-help", "Each current employee receives each active type's entitlement from the version in force on the first day of that leave year. Repeating a grant changes nothing.")?);
            sections.push(card("leave-year", "Leave year", year)?);

            let mut fields: Vec<_> = WEEKDAYS
                .iter()
                .enumerate()
                .map(|(i, label)| {
                    let key = format!("weekday_{}", i + 1);
                    let checked = self.form_value(
                        "save_request_rules",
                        &key,
                        requests.working_weekdays.contains(&((i + 1) as i32)),
                    ) == "true";
                    Field::named(&key)
                        .id(format!("leave-{key}"))
                        .label(*label)
                        .kind(InputKind::Checkbox)
                        .checked(checked)
                })
                .collect();
            fields.push(
                self.field(
                    "save_request_rules",
                    "backdate_days",
                    "leave-backdate-days",
                    "Requests may start up to (days before today)",
                    requests.backdate_days,
                )
                .kind(InputKind::Number)
                .attr("aria-label", "Requests may start up to days before today")
                .required()
                .attr("min", "0")
                .attr("max", "366"),
            );
            let mut request_rules =
                note("leave-weekdays-label", "Working weekdays counted as leave")?;
            request_rules.push(&form(
                "leave-request-rules-form",
                "save_request_rules",
                fields,
                "Save request rules",
                false,
                false,
            )?);
            request_rules.push(&note(
                "leave-request-rules-help",
                "Calendar exceptions in People references are never counted as leave days.",
            )?);
            sections.push(card("leave-request-rules", "Request rules", request_rules)?);

            let mut carry = form(
                "leave-carry-forward-form",
                "carry_forward",
                vec![self
                    .field(
                        "carry_forward",
                        "from_year",
                        "leave-carry-year",
                        "Carry forward from leave year",
                        self.current_year - 1,
                    )
                    .kind(InputKind::Number)
                    .required()
                    .attr("min", "1900")
                    .attr("max", "9997")],
                "Queue carry-forward",
                false,
                false,
            )?;
            carry.push(&note(
                "leave-carried-count",
                &format!(
                    "{} balances from leave year {} have been carried forward.",
                    self.carried,
                    self.current_year - 1
                ),
            )?);
            if !self.skipped.skips.is_empty() {
                if (self.skipped.skips.len() as i64) < self.skipped.total {
                    carry.push(&note(
                        "leave-carry-skipped-truncated",
                        &format!(
                            "Showing {} of {} skipped balances, oldest leave year first.",
                            self.skipped.skips.len(),
                            self.skipped.total
                        ),
                    )?);
                }
                carry.push(&self.skips_table()?);
            }
            carry.push(&note("leave-carry-help", "For each type whose policy on the year's last day sets a cap, each current employee's closing balance up to the cap moves into the next year and the rest expires. The year must have ended, and years are carried in order. Each year's latest run lists the employees it skipped: those with a pending request in the year, or with an earlier year not carried forward yet, naming the year to resolve. Run the years again, oldest first, once that is resolved. A carried year, and every year before it, is closed to new requests and entries.")?);
            sections.push(card(
                "leave-carry-forward",
                "Year-end carry-forward",
                carry,
            )?);

            let mut types = self.types_table()?;
            types.push(&form(
                "leave-type-form",
                "create_type",
                vec![
                    self.field("create_type", "type[code]", "leave-type-code", "Code", "")
                        .required()
                        .attr("aria-label", "Leave type code")
                        .attr("maxlength", "40"),
                    self.field("create_type", "type[name]", "leave-type-name", "Name", "")
                        .required()
                        .attr("aria-label", "Leave type name")
                        .attr("maxlength", "120"),
                    self.field(
                        "create_type",
                        "type[unit]",
                        "leave-type-unit",
                        "Unit",
                        "day",
                    )
                    .kind(InputKind::Select)
                    .attr("aria-label", "Leave type unit")
                    .options(choices(&[("Days", "day"), ("Hours", "hour")])),
                    Field::in_form("type", "paid")
                        .id("leave-type-paid")
                        .label("Paid leave")
                        .kind(InputKind::Checkbox)
                        .checked(self.form_value("create_type", "type[paid]", "true") == "true"),
                    Field::in_form("type", "balance_required")
                        .id("leave-type-balance-required")
                        .label("Requests need an available balance")
                        .kind(InputKind::Checkbox)
                        .checked(
                            self.form_value("create_type", "type[balance_required]", "true")
                                == "true",
                        ),
                ],
                "Add leave type",
                false,
                false,
            )?);
            sections.push(card("leave-types-section", "Leave types", types)?);

            let mut policies = self.policies_table()?;
            let active: Vec<_> = self
                .types
                .iter()
                .filter(|t| t.status == "active")
                .map(|t| (t.name.clone(), t.id.to_string()))
                .collect();
            if !active.is_empty() {
                policies.push(&form(
                    "leave-policy-form",
                    "add_policy",
                    vec![
                        self.field(
                            "add_policy",
                            "policy[leave_type_id]",
                            "leave-policy-type",
                            "Leave type for policy",
                            &active[0].1,
                        )
                        .kind(InputKind::Select)
                        .options(choices(&active)),
                        self.field(
                            "add_policy",
                            "policy[effective_from]",
                            "leave-policy-from",
                            "Effective from",
                            "",
                        )
                        .kind(InputKind::Date)
                        .required(),
                        self.field(
                            "add_policy",
                            "policy[entitlement]",
                            "leave-policy-entitlement",
                            "Entitlement per year",
                            "",
                        )
                        .required()
                        .attr("inputmode", "decimal"),
                        self.field(
                            "add_policy",
                            "policy[carry_forward_cap]",
                            "leave-policy-cap",
                            "Carry-forward cap",
                            "",
                        )
                        .attr("inputmode", "decimal"),
                    ],
                    "Add version",
                    false,
                    false,
                )?);
            }
            versions = card("leave-policies", "Entitlement policy versions", policies)?;
        }
        let confirmation = match &self.pending {
            Some(Pending::Grant {year,..}) => render(&ConfirmDialog {
                id: "leave-policy-confirmation", consequence: &format!("Grant entitlements for leave year {year}?"),
                detail: "Each current employee receives each active type's entitlement from the version in force on the first day of that leave year. Repeating a grant changes nothing.",
                confirm: "Grant", working: "Granting…", confirm_event: "confirm_action", cancel_event: "cancel_confirmation",
            })?,
            Some(Pending::Carry {year,..}) => render(&ConfirmDialog {
                id: "leave-policy-confirmation", consequence: &format!("Queue carry-forward from leave year {year}?"),
                detail: "Closing balances up to each policy's cap move to the next year and the rest expires. A carried year, and every year before it, is closed to new requests and entries.",
                confirm: "Queue carry-forward", working: "Queuing…", confirm_event: "confirm_action", cancel_event: "cancel_confirmation",
            })?,
            Some(Pending::Status {name,status,..}) => render(&ConfirmDialog {
                id: "leave-policy-confirmation", consequence: &format!("{} {name}?",if *status=="archived" { "Archive" } else { "Restore" }),
                detail: if *status=="archived" { "Archived types take no new policy versions, grants, requests or entries. History and balances are kept." } else { "This leave type will be available again." },
                confirm: if *status=="archived" { "Archive" } else { "Restore" }, working: "Updating…", confirm_event: "confirm_action", cancel_event: "cancel_confirmation",
            })?,
            None => Html::EMPTY,
        };
        View {
            flash,
            subtitle: company
                .map(|c| format!("{} · Leave year, types and entitlements", c.name))
                .unwrap_or_default(),
            chooser,
            state,
            available: self.rules.is_some(),
            sections,
            versions,
            confirmation,
        }
        .render()
        .map_err(|_| Error::Internal)
    }
    fn types_table(&self) -> Result<Html> {
        let rows = self
            .types
            .iter()
            .map(|t| {
                let action = render(&Button {
                    id: &format!("leave-type-{}-status", t.id),
                    event: "set_status",
                    button_type: "button",
                    name: "value",
                    value: &t.id.to_string(),
                    body: Html::text(if t.status == "active" {
                        "Archive"
                    } else {
                        "Restore"
                    }),
                    ..Button::default()
                })?;
                Ok(FlexRow::new(format!("leave-type-{}", t.id))
                    .slot("name", Html::text(&t.name))
                    .slot("code", Html::text(&t.code))
                    .slot(
                        "unit",
                        Html::text(if t.unit == "hour" { "hours" } else { "days" }),
                    )
                    .slot("paid", Html::text(if t.paid { "paid" } else { "unpaid" }))
                    .slot(
                        "balance",
                        Html::text(if t.balance_required {
                            "Requests need an available balance"
                        } else {
                            "may go negative"
                        }),
                    )
                    .slot("status", Html::text(&t.status))
                    .slot("actions", action))
            })
            .collect::<Result<Vec<_>>>()?;
        self.types_table
            .render(rows, "No leave types have been added for this company.")
    }
    fn policies_table(&self) -> Result<Html> {
        let rows = self
            .policies
            .iter()
            .map(|p| {
                let name = self
                    .types
                    .iter()
                    .find(|t| t.id == p.leave_type_id)
                    .map_or("Leave", |t| t.name.as_str());
                FlexRow::new(format!("leave-policy-{}", p.id))
                    .slot("type", Html::text(name))
                    .slot("version", Html::text(&p.version.to_string()))
                    .slot("from", Html::text(&p.effective_from.to_string()))
                    .slot(
                        "to",
                        Html::text(
                            &p.effective_to
                                .map(|d| d.to_string())
                                .unwrap_or_else(|| "Open".into()),
                        ),
                    )
                    .slot("entitlement", Html::text(&p.entitlement.to_string()))
                    .slot(
                        "cap",
                        Html::text(
                            &p.carry_forward_cap
                                .map(|d| d.to_string())
                                .unwrap_or_else(|| "No carry-forward".into()),
                        ),
                    )
            })
            .collect();
        self.policies_table.render(
            rows,
            "No policy versions have been added. Add a leave type, then its first version.",
        )
    }
    fn skips_table(&self) -> Result<Html> {
        let rows = self
            .skipped
            .skips
            .iter()
            .map(|s| {
                let reason = if s.reason == "pending" {
                    format!("has a pending request in leave year {}", s.blocking_year)
                } else {
                    format!("leave year {} is not carried forward yet", s.blocking_year)
                };
                FlexRow::new(format!(
                    "leave-carry-skipped-{}-{}-{}",
                    s.from_year, s.employee_id, s.leave_type_id
                ))
                .slot("year", Html::text(&s.from_year.to_string()))
                .slot("employee", Html::text(&s.employee_name))
                .slot("type", Html::text(&s.leave_type_name))
                .slot("reason", Html::text(&reason))
            })
            .collect();
        self.skips_table.render(rows, "No skipped balances.")
    }
    fn query(&self) -> HashMap<String, String> {
        let mut query = HashMap::new();
        for table in [&self.types_table, &self.policies_table, &self.skips_table] {
            query.extend(table.state.to_params());
        }
        if let Some(id) = self.company_id {
            query.insert("company_id".into(), id.to_string());
        }
        query
    }
}
impl LiveState<Scope, Error> for Policies {
    fn update<'a>(&'a mut self, request: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            match request.input {
                LiveInput::Event(payload) => self.event(request.scope, payload).await?,
                LiveInput::Refresh => self.load(request.scope).await?,
                LiveInput::Params(query) => {
                    self.company_id =
                        params::positive_id(query.get("company_id").map(String::as_str));
                    self.initialized = false;
                    self.pending = None;
                    self.forms.clear();
                    self.clear_records();
                    for table in [
                        &mut self.types_table,
                        &mut self.policies_table,
                        &mut self.skips_table,
                    ] {
                        table.state = ListState::parse(query, Arc::new(table.state.spec().clone()))
                            .map_err(|_| Error::Internal)?;
                    }
                    self.load(request.scope).await?;
                }
            }
            Ok(LiveBody {
                html: self.render()?,
                query: self.query(),
                rows: self.types.len() + self.policies.len() + self.skipped.skips.len(),
                ..LiveBody::default()
            })
        })
    }
}
fn value<'a>(v: &'a HashMap<String, String>, key: &str) -> &'a str {
    v.get(key).map_or("", String::as_str)
}
fn integer(v: &HashMap<String, String>, key: &str) -> i32 {
    value(v, key).trim().parse().unwrap_or(-1)
}
fn integer64(v: &HashMap<String, String>, key: &str) -> i64 {
    value(v, key).trim().parse().unwrap_or(-1)
}
fn note(id: &str, text: &str) -> Result<Html> {
    render(&Note { id, text })
}
fn card(id: &str, title: &str, body: Html) -> Result<Html> {
    render(&Card {
        id,
        title,
        body,
        class: "mt-4",
        ..Card::default()
    })
}
fn form(
    id: &str,
    event: &str,
    fields: Vec<Field>,
    label: &str,
    disabled: bool,
    change: bool,
) -> Result<Html> {
    render(&Form {
        id,
        event,
        change,
        class: "mt-3",
        body: render(&Fields {
            fields: fields
                .into_iter()
                .map(|field| field.wrapper_class("mb-0"))
                .collect(),
        })?,
        actions: if change {
            Html::EMPTY
        } else {
            render(&Button {
                id: &format!("{id}-submit"),
                button_type: "submit",
                variant: ButtonVariant::Primary,
                disabled,
                body: Html::text(label),
                ..Button::default()
            })?
        },
    })
}
