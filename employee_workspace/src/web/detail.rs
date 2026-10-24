//! Bilimbi EmployeeWorkspace.Web.ShowLive, with employment edits moved from
//! Core Employee to Workforce and shared Bilibi inline/confirmation components.
use super::*;
use crate::{Access, ChangeRequest, FactRecord, NewChangeRequest, WorkProfile, MANAGE, REVIEW};
use bilibi_party_ids::AffiliationId;
use bilibi_people_workforce::{Employment, EmploymentChange, EMPLOYMENT_VIEW};
use bilibi_ui::{
    page::PanelSpec, BackLink, Card, CommitStates, CommitStatus, Fact, Facts, Form, InlineChoice,
    InlineEdit, InlineField, InlineLongText, Restricted, SectionHeading,
};

const WRITE_REFUSAL: &str = "You no longer have permission to change this employee's People facts.";
const UNAVAILABLE: &str = "This employee is unavailable in your company.";

#[derive(Template)]
#[template(path = "employee.html")]
struct View<'a> {
    flash: FlashMap<'a>,
    employee_id: String,
    available: bool,
    subtitle: String,
    back: Html,
    identity: Html,
    employment: Html,
    profile: Html,
    access: Html,
    requests: Html,
    unavailable: Html,
    confirmation: Html,
}
#[derive(Template)]
#[template(
    source = "<div id=\"{{ id }}-slot\" data-live-ignore><div id=\"{{ id }}\"></div></div>",
    ext = "html"
)]
struct HistoryAnchor<'a> {
    id: &'a str,
}

pub(super) struct Detail {
    core: Arc<Core>,
    id: i64,
    company: Option<i64>,
    employee: Option<DirectoryEmployee>,
    employment: Option<Employment>,
    identity: Vec<Fact<'static>>,
    profile: Option<FactRecord<WorkProfile>>,
    access: Option<FactRecord<Access>>,
    requests: Vec<ChangeRequest>,
    supervisors: Vec<Choice>,
    types: Vec<Choice>,
    departments: Vec<Choice>,
    list: ListState,
    columns: ListColumns,
    commits: CommitStates,
    can_manage: bool,
    can_review: bool,
    can_employ: bool,
    can_department: bool,
    flash: Option<(StatusKind, String)>,
    notice: String,
    request: HashMap<String, String>,
    errors: Vec<FieldError>,
    pending_review: Option<(i64, &'static str)>,
    history_available: bool,
}
impl Detail {
    pub(super) fn new(core: Arc<Core>, request: &PageRequest<'_, Scope>) -> Result<Self> {
        let id = params::positive_id(request.path.get("id").map(String::as_str))
            .ok_or(Error::NotFound)?;
        let history_available = inventory::iter::<bilibi_module_registry::Contribution>
            .into_iter()
            .any(|entry| {
                entry.consumer == bilibi_module_registry::contributions::EMBEDS
                    && entry.id == "audit.record-history"
            });
        Ok(Self {
            core,
            id,
            company: None,
            employee: None,
            employment: None,
            identity: vec![],
            profile: None,
            access: None,
            requests: vec![],
            supervisors: vec![],
            types: vec![],
            departments: vec![],
            list: ListState::parse(request.query, detail_spec()?).map_err(|_| Error::Internal)?,
            columns: ListColumns::mount(
                "people-employee-requests",
                vec![
                    BuiltinColumn::new("field", "Field").sortable("field"),
                    BuiltinColumn::new("value", "Proposed value").sortable("value"),
                    BuiltinColumn::new("reason", "Reason"),
                    BuiltinColumn::new("status", "Status").sortable("status"),
                ],
            )
            .map_err(|_| Error::Internal)?,
            commits: CommitStates::default(),
            can_manage: false,
            can_review: false,
            can_employ: false,
            can_department: false,
            flash: None,
            notice: String::new(),
            request: HashMap::new(),
            errors: vec![],
            pending_review: None,
            history_available,
        })
    }
    fn clear(&mut self) {
        self.employee = None;
        self.employment = None;
        self.identity.clear();
        self.profile = None;
        self.access = None;
        self.requests.clear();
        self.supervisors.clear();
        self.types.clear();
        self.departments.clear();
        self.can_manage = false;
        self.can_review = false;
        self.can_employ = false;
        self.can_department = false;
        self.pending_review = None;
    }
    fn error(&mut self, text: &str) {
        self.flash = Some((StatusKind::Error, text.into()));
        self.notice = text.into();
    }
    fn success(&mut self, text: &str) {
        self.flash = Some((StatusKind::Success, text.into()));
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
    pub(super) async fn load(&mut self, scope: &Scope) -> Result<()> {
        let client = self.core.db.get().await?;
        let companies = match authorization::selectable_companies(&**client, scope, VIEW).await {
            Ok(companies) => companies,
            Err(WorkforceError::Failed(e)) => return Err(e),
            Err(_) => vec![],
        };
        let requested = self
            .list
            .filters
            .get("company_id")
            .filter(|s| !s.is_empty());
        let bound = self
            .company
            .or_else(|| params::positive_id(requested.map(String::as_str)));
        if requested.is_some() && bound.is_none() {
            self.clear();
            return Ok(());
        }
        // Bind the company from an authorized directory result. Never accept a
        // company or employee from a write event, and keep this mount bound.
        let mut found = None;
        for company in companies
            .iter()
            .filter(|c| bound.is_none_or(|id| id == c.id))
        {
            match self.core.workspace.employees(scope, company.id).await {
                Ok(rows) => {
                    if let Some(employee) = rows.iter().find(|e| e.id == self.id).cloned() {
                        found = Some((company.id, employee, rows));
                        break;
                    }
                }
                Err(WorkspaceError::Workforce(WorkforceError::Failed(e))) => return Err(e),
                Err(_) => {}
            }
        }
        let Some((company, employee, rows)) = found else {
            self.clear();
            return Ok(());
        };
        self.company = Some(company);
        self.list
            .filters
            .insert("company_id".into(), company.to_string());
        self.can_manage = authorization::allowed(&**client, scope, company, MANAGE).await;
        self.can_review = authorization::allowed(&**client, scope, company, REVIEW).await;
        let may_read_employment =
            authorization::allowed(&**client, scope, company, EMPLOYMENT_VIEW).await;
        self.can_employ = may_read_employment
            && authorization::allowed(&**client, scope, company, EMPLOYMENT_MANAGE).await;
        self.can_department =
            authorization::allowed(&**client, scope, company, "admin.company.view").await;
        if !self.can_review {
            self.pending_review = None;
        }
        self.departments = if self.can_department {
            match bilibi_company::list_departments(
                &**client,
                scope,
                company,
                bilibi_company::DepartmentSort::Name,
                bilibi_company::SortDirection::Asc,
            )
            .await
            {
                Ok(rows) => rows
                    .into_iter()
                    .map(|d| Choice::new(d.type_name, d.id.to_string()))
                    .collect(),
                Err(Error::Forbidden | Error::Unauthorized | Error::NotFound) => {
                    self.can_department = false;
                    vec![]
                }
                Err(error) => return Err(error),
            }
        } else {
            vec![]
        };
        drop(client);
        self.employee = Some(employee);
        self.employment = if may_read_employment {
            match self
                .core
                .workforce
                .employment(scope, company, self.id)
                .await
            {
                Ok(row) => row.require_current().ok(),
                Err(WorkforceError::Failed(e)) => return Err(e),
                Err(_) => None,
            }
        } else {
            None
        };
        self.can_employ &= self.employment.is_some();
        self.types = if self.can_employ {
            match self.core.workforce.employment_types(scope, company).await {
                Ok(types) => types
                    .into_iter()
                    .map(|t| Choice::new(t.label, t.code))
                    .collect(),
                Err(WorkforceError::Failed(e)) => return Err(e),
                Err(_) => vec![],
            }
        } else {
            vec![]
        };
        self.supervisors.clear();
        if self.can_employ {
            if let Ok(statuses) = self.core.workforce.working_statuses(scope, company).await {
                if let Ok(statuses) = statuses.require_current() {
                    self.supervisors = rows
                        .iter()
                        .filter(|e| e.id != self.id && statuses.contains(&e.status))
                        .map(|e| {
                            Choice::new(
                                format!(
                                    "{} · {}",
                                    e.employee.display_name, e.employee.employee_number
                                ),
                                e.id.to_string(),
                            )
                        })
                        .collect();
                }
            }
        }
        self.load_identity(scope).await?;
        let facts = async {
            Ok::<_, WorkspaceError>((
                self.core
                    .workspace
                    .work_profile_record(scope, company, self.id)
                    .await?,
                self.core
                    .workspace
                    .access_record(scope, company, self.id)
                    .await?,
                self.core
                    .workspace
                    .change_requests(scope, company, self.id)
                    .await?,
            ))
        }
        .await;
        match facts {
            Ok((profile, access, requests)) => {
                self.profile = Some(profile);
                self.access = Some(access);
                self.requests = requests;
            }
            Err(WorkspaceError::Workforce(WorkforceError::Failed(e))) => return Err(e),
            Err(_) => self.clear(),
        }
        Ok(())
    }
    async fn load_identity(&mut self, scope: &Scope) -> Result<()> {
        self.identity.clear();
        let Some(employee) = &self.employee else {
            return Ok(());
        };
        self.identity
            .push(text_fact("Name", &employee.employee.display_name));
        // Person and Affiliation owners apply their own read capabilities and
        // contact masks. Missing permission is a restricted fact, not a blank.
        let person = if let Some(employment) = &self.employment {
            match self
                .core
                .persons
                .read(
                    scope,
                    PersonId::new(employment.person_id).map_err(|_| Error::Internal)?,
                )
                .await
            {
                Ok(person) => Some(person),
                Err(Error::Forbidden | Error::Unauthorized | Error::NotFound) => None,
                Err(e) => return Err(e),
            }
        } else {
            None
        };
        for (key, label) in [("email", "Personal email"), ("phone", "Personal phone")] {
            let value = match &person {
                Some(p) if !p.restricted.iter().any(|r| r == key) => {
                    Html::text(if key == "email" {
                        p.email.as_deref().unwrap_or("—")
                    } else {
                        p.phone.as_deref().unwrap_or("—")
                    })
                }
                _ => render(&Restricted::new(format!("employee-{key}-restricted")))?,
            };
            self.identity.push(Fact {
                title: label,
                value,
                ..Fact::default()
            });
        }
        let affiliation = if let Some(employment) = &self.employment {
            match self
                .core
                .affiliations
                .read(
                    scope,
                    AffiliationId::new(employment.affiliation_id).map_err(|_| Error::Internal)?,
                )
                .await
            {
                Ok(affiliation) => Some(affiliation),
                Err(Error::Forbidden | Error::Unauthorized | Error::NotFound) => None,
                Err(e) => return Err(e),
            }
        } else {
            None
        };
        for (key, label) in [
            ("title", "Designation"),
            ("work_email", "Email"),
            ("work_phone", "Work phone"),
        ] {
            let value = match &affiliation {
                Some(a) if !a.restricted.iter().any(|r| r == key) => Html::text(match key {
                    "title" => a.affiliation.title.as_deref().unwrap_or("—"),
                    "work_email" => a.affiliation.work_email.as_deref().unwrap_or("—"),
                    _ => a.affiliation.work_phone.as_deref().unwrap_or("—"),
                }),
                None if key == "work_email" && employee.employee.email.is_some() => {
                    Html::text(employee.employee.email.as_deref().unwrap_or("—"))
                }
                _ => render(&Restricted::new(format!("employee-{key}-restricted")))?,
            };
            self.identity.push(Fact {
                title: label,
                value,
                ..Fact::default()
            });
        }
        Ok(())
    }
    fn inline(
        &self,
        key: &str,
        label: &str,
        value: &str,
        version: i64,
        editable: bool,
    ) -> InlineField {
        let mut field = InlineField::new(
            format!("employee-{}", key.replace('_', "-")),
            key,
            label,
            value,
        )
        .version(version)
        .status(self.commits.get(key));
        field.id_value = self.id.to_string();
        if ![
            "employee_number",
            "employment_type",
            "status",
            "portal_enabled",
        ]
        .contains(&key)
        {
            field = field.allow_empty();
        }
        if !editable {
            field = field.readonly();
        }
        field
    }
    #[allow(clippy::too_many_arguments)]
    fn text(
        &self,
        key: &str,
        label: &str,
        value: &str,
        version: i64,
        editable: bool,
        long: bool,
    ) -> Result<Html> {
        let field = self.inline(key, label, value, version, editable);
        if long {
            render(&InlineLongText { field, rows: 3 })
        } else {
            render(&InlineEdit { field })
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn choice(
        &self,
        key: &str,
        label: &str,
        value: &str,
        version: i64,
        editable: bool,
        choices: Vec<Choice>,
    ) -> Result<Html> {
        let display = choices
            .iter()
            .find(|c| c.value == value)
            .map_or_else(|| value.to_owned(), |c| c.label.clone());
        render(&InlineChoice {
            field: self.inline(key, label, value, version, editable),
            display: Html::text(if display.is_empty() { "—" } else { &display }),
            editor: render(
                &Field::named(key)
                    .id(format!("employee-{key}-choice"))
                    .label(label)
                    .kind(InputKind::Select)
                    .value(value)
                    .options(choices),
            )?,
        })
    }
    fn history_anchor(&self, id: &str) -> Result<Html> {
        if self.history_available {
            render(&HistoryAnchor { id })
        } else {
            Ok(Html::EMPTY)
        }
    }
    fn employment_facts(&self) -> Result<Html> {
        let Some(e) = &self.employment else {
            let Some(e) = &self.employee else {
                return Ok(Html::EMPTY);
            };
            return render(&Facts {
                id: "employee-employment-facts",
                items: vec![
                    text_fact("Number", &e.employee.employee_number),
                    text_fact("Status", &e.status),
                    text_fact("Employment type", &e.employment_type),
                ],
            });
        };
        let edit = self.can_employ;
        let mut types = self.types.clone();
        if !types.iter().any(|c| c.value == e.employment_type) {
            types.push(Choice::new(&e.employment_type, &e.employment_type));
        }
        let mut supervisors = vec![Choice::new("None", "")];
        supervisors.extend(self.supervisors.clone());
        // A non-working supervisor still reads as the stored relation, while
        // choosing a replacement is limited to the company's working employees.
        if let Some(id) = e.supervisor_employment_id {
            if !supervisors.iter().any(|c| c.value == id.to_string()) {
                supervisors.push(Choice::new(format!("Employee {id}"), id.to_string()));
            }
        }
        let mut items = vec![
            Fact {
                title: "Number",
                value: self.text(
                    "employee_number",
                    "Employee number",
                    &e.employee_number,
                    e.version,
                    edit,
                    false,
                )?,
                ..Fact::default()
            },
            Fact {
                title: "Employment type",
                value: self.choice(
                    "employment_type",
                    "Employment type",
                    &e.employment_type,
                    e.version,
                    edit,
                    types,
                )?,
                ..Fact::default()
            },
            Fact {
                title: "Status",
                value: self.choice(
                    "status",
                    "Status",
                    &e.status,
                    e.version,
                    edit,
                    EMPLOYEE_STATUSES
                        .iter()
                        .map(|s| Choice::new(capitalize(s), *s))
                        .collect(),
                )?,
                ..Fact::default()
            },
            Fact {
                title: "Employment start",
                value: self.text(
                    "employment_start",
                    "Employment start",
                    &e.employment_start
                        .map(|d| d.to_string())
                        .unwrap_or_default(),
                    e.version,
                    edit,
                    false,
                )?,
                ..Fact::default()
            },
            Fact {
                title: "Employment end",
                value: self.text(
                    "employment_end",
                    "Employment end",
                    &e.employment_end.map(|d| d.to_string()).unwrap_or_default(),
                    e.version,
                    edit,
                    false,
                )?,
                ..Fact::default()
            },
            Fact {
                title: "Supervisor",
                value: self.choice(
                    "supervisor_employment_id",
                    "Supervisor",
                    &e.supervisor_employment_id
                        .map(|id| id.to_string())
                        .unwrap_or_default(),
                    e.version,
                    edit,
                    supervisors,
                )?,
                ..Fact::default()
            },
        ];
        if self.can_department {
            let mut departments = vec![Choice::new("None", "")];
            departments.extend(self.departments.clone());
            items.push(Fact {
                title: "Department",
                value: self.choice(
                    "department_id",
                    "Department",
                    &e.department_id.map(|id| id.to_string()).unwrap_or_default(),
                    e.version,
                    edit,
                    departments,
                )?,
                ..Fact::default()
            });
        }
        render(&Facts {
            id: "employee-employment-facts",
            items,
        })
    }
    fn requests_table(&self) -> Result<Html> {
        let mut requests = self.requests.iter().collect::<Vec<_>>();
        let key = |r: &ChangeRequest| match self.list.sort_by.as_deref() {
            Some("value") => r.proposed_value.clone(),
            Some("status") => r.status.clone(),
            _ => r.field.clone(),
        };
        if self.list.sort_by.as_deref() == Some("id") {
            requests.sort_by_key(|request| request.id);
        } else {
            requests.sort_by(|a, b| key(a).cmp(&key(b)).then(a.id.cmp(&b.id)));
        }
        if self.list.sort_dir == Some(SortDir::Desc) {
            requests.reverse();
        }
        let rows = requests
            .into_iter()
            .map(|r| {
                let id = r.id.to_string();
                let mut row = FlexRow::new(&id)
                    .slot("field", Html::text(&r.field.replace('_', " ")))
                    .slot("value", Html::text(&r.proposed_value))
                    .slot("reason", Html::text(r.reason.as_deref().unwrap_or("—")))
                    .slot("status", Html::text(&r.status));
                if self.can_review && r.status == "pending" {
                    row.actions = action("approve_change", "Approve", &id)?;
                    row.actions.push(&action("reject_change", "Reject", &id)?);
                }
                Ok(row)
            })
            .collect::<Result<Vec<_>>>()?;
        let mut table = FlexTable::new(
            "people-employee-requests",
            self.columns.flex_columns(),
            rows,
        )
        .sorted(self.list.sort_by.as_deref(), self.list.sort_dir);
        table.event = "requests_grid";
        table.zoom = self.columns.zoom;
        table.suggestions = self.columns.flex_suggestions();
        table.add_query = &self.columns.add_query;
        table.actions = self.can_review;
        table.caption = "Profile change requests";
        table.empty = Some(TableEmpty::State(EmptyState::missing(
            "No profile changes have been requested.",
            "",
        )));
        render(&table)
    }
    fn request_form(&self) -> Result<Html> {
        if !self.can_manage {
            return Ok(Html::EMPTY);
        }
        let mut summary = ErrorSummary::new("employee-request-errors");
        for e in &self.errors {
            summary.push(
                format!("request-{}", e.field),
                bilibi_ui::forms::error_sentence(&e.field.replace('_', " "), &e.message),
            );
        }
        let mut body = render(&summary)?;
        for (key, label, max) in [
            ("field", "Change field", ""),
            ("proposed_value", "Proposed value", "500"),
            ("reason", "Change reason", "2000"),
        ] {
            let mut field = Field::in_form("request", key)
                .id(format!("request-{key}"))
                .label(label)
                .value(self.request.get(key).map_or("", String::as_str))
                .errors(
                    self.errors
                        .iter()
                        .filter(|e| e.field == key)
                        .map(|e| e.message.clone())
                        .collect(),
                );
            if key == "field" {
                field = field.kind(InputKind::Select).options(
                    crate::CHANGE_FIELDS
                        .iter()
                        .map(|f| Choice::new(f.replace('_', " "), *f))
                        .collect(),
                );
            } else {
                field = field.attr("maxlength", max);
            }
            if key == "proposed_value" {
                field = field.required();
            }
            body.push(&render(&field)?);
        }
        render(&Form {
            id: "employee-change-request",
            event: "request_change",
            body,
            actions: submit("record-employee-request", "Record request")?,
            ..Form::default()
        })
    }
    pub(super) fn render(&self) -> Result<String> {
        let identity = card(
            "employee-core",
            "Employee",
            "Employee identity and contacts are managed in Contacts.",
            render(&Facts {
                id: "employee-core-facts",
                items: self.identity.clone(),
            })?,
            Html::EMPTY,
        )?;
        let employment = card(
            "employee-employment",
            "Employment",
            "",
            self.employment_facts()?,
            if self.employment.is_some() {
                self.history_anchor("employee-employment-history")?
            } else {
                Html::EMPTY
            },
        )?;
        let profile = if let Some(p) = &self.profile {
            let p_value = &p.value;
            let items = [
                (
                    "work_location",
                    "Location",
                    p_value.work_location.as_deref(),
                ),
                (
                    "work_arrangement",
                    "Arrangement",
                    p_value.work_arrangement.as_deref(),
                ),
                ("notes", "Notes", p_value.notes.as_deref()),
            ]
            .into_iter()
            .map(|(key, label, value)| {
                Ok(Fact {
                    title: label,
                    value: self.text(
                        key,
                        label,
                        value.unwrap_or(""),
                        p.version,
                        self.can_manage,
                        key == "notes",
                    )?,
                    ..Fact::default()
                })
            })
            .collect::<Result<Vec<_>>>()?;
            card(
                "work-profile",
                "Work profile",
                "",
                render(&Facts {
                    id: "work-profile-facts",
                    items,
                })?,
                if p.id.is_some() {
                    self.history_anchor("employee-profile-history")?
                } else {
                    Html::EMPTY
                },
            )?
        } else {
            Html::EMPTY
        };
        let access = if let Some(a) = &self.access {
            let a_value = &a.value;
            let items = vec![
                Fact {
                    title: "Eligible",
                    value: self.choice(
                        "portal_enabled",
                        "Portal eligibility",
                        if a_value.portal_enabled {
                            "true"
                        } else {
                            "false"
                        },
                        a.version,
                        self.can_manage,
                        vec![
                            Choice::new("Disabled", "false"),
                            Choice::new("Enabled", "true"),
                        ],
                    )?,
                    ..Fact::default()
                },
                Fact {
                    title: "Reason",
                    value: self.text(
                        "reason",
                        "Access reason",
                        a_value.reason.as_deref().unwrap_or(""),
                        a.version,
                        self.can_manage,
                        true,
                    )?,
                    ..Fact::default()
                },
            ];
            card(
                "portal-access",
                "Portal access",
                "Eligibility does not create a login account or grant a role.",
                render(&Facts {
                    id: "portal-access-facts",
                    items,
                })?,
                if a.id.is_some() {
                    self.history_anchor("employee-access-history")?
                } else {
                    Html::EMPTY
                },
            )?
        } else {
            Html::EMPTY
        };
        let mut requests = self.requests_table()?;
        requests.push(&self.request_form()?);
        let confirmation = if let Some((_, decision)) = self.pending_review {
            render(&ConfirmDialog {
                id: "review-employee-change",
                consequence: if decision == "approved" {
                    "Approve profile change request?"
                } else {
                    "Reject profile change request?"
                },
                detail: "Core employee details are unchanged. This cannot be undone.",
                confirm: if decision == "approved" {
                    "Approve"
                } else {
                    "Reject"
                },
                working: "Reviewing…",
                confirm_event: "confirm_review",
                cancel_event: "cancel_review",
            })?
        } else {
            Html::EMPTY
        };
        View {
            flash: self.flash(),
            employee_id: self.id.to_string(),
            available: self.employee.is_some(),
            subtitle: self
                .employee
                .as_ref()
                .map(|e| {
                    format!(
                        "{} · {}",
                        e.employee.display_name, e.employee.employee_number
                    )
                })
                .unwrap_or_default(),
            back: render(&BackLink {
                href: &params::address(
                    "/people/employees",
                    &self
                        .company
                        .map(|c| HashMap::from([("company_id".into(), c.to_string())]))
                        .unwrap_or_default(),
                ),
                title: "Back to employees",
                ..BackLink::default()
            })?,
            identity,
            employment,
            profile,
            access,
            requests: card(
                "change-requests",
                "Profile change requests",
                "",
                requests,
                Html::EMPTY,
            )?,
            unavailable: render(&EmptyState::missing(UNAVAILABLE, ""))?,
            confirmation,
        }
        .render()
        .map_err(|_| Error::Internal)
    }
    async fn save_inline(&mut self, scope: &Scope, payload: &Value) {
        let key = string(payload, "field");
        let value = string(payload, "value");
        let employment = [
            "employee_number",
            "employment_type",
            "status",
            "employment_start",
            "employment_end",
            "supervisor_employment_id",
            "department_id",
        ]
        .contains(&key);
        let profile = ["work_location", "work_arrangement", "notes"].contains(&key);
        let access = ["portal_enabled", "reason"].contains(&key);
        if !(employment || profile || access) {
            self.error("Check the proposed change.");
            return;
        }
        if self.employee.is_none()
            || (employment && !self.can_employ)
            || (!employment && !self.can_manage)
            || (key == "department_id" && !self.can_department)
        {
            self.commits.clear_saved();
            self.error(WRITE_REFUSAL);
            return;
        }
        if key == "employment_type" && !self.types.iter().any(|choice| choice.value == value) {
            self.commits.put(
                key,
                CommitStatus::refusal(
                    "Employment type",
                    value,
                    &["is not an active company choice"],
                ),
            );
            return;
        }
        let Some(company) = self.company else {
            return;
        };
        if params::positive_id(payload.get("id").and_then(Value::as_str)) != Some(self.id) {
            self.error(UNAVAILABLE);
            return;
        }
        let version = string(payload, "version")
            .parse::<i64>()
            .ok()
            .filter(|v| *v >= 0);
        let Some(version) = version else {
            self.commits.put(
                key,
                CommitStatus::refusal(key, value, &["has an invalid version"]),
            );
            return;
        };
        let result = if employment {
            match employment_change(key, value, version) {
                Ok(change) => self
                    .core
                    .workforce
                    .update_employment(scope, company, self.id, &change)
                    .await
                    .map(|_| ())
                    .map_err(WorkspaceError::from),
                Err(e) => Err(e.into()),
            }
        } else if profile {
            self.core
                .workspace
                .update_work_profile(scope, company, self.id, version, key, value)
                .await
                .map(|_| ())
        } else {
            self.core
                .workspace
                .update_access(scope, company, self.id, version, key, value)
                .await
                .map(|_| ())
        };
        match result {
            Ok(()) => {
                self.commits.put(key, CommitStatus::Saved);
                self.success(if employment {
                    "Employment updated successfully."
                } else if profile {
                    "Work profile saved."
                } else {
                    "Portal access saved."
                });
            }
            Err(WorkspaceError::Workforce(WorkforceError::Unauthorized)) => {
                self.commits.clear_saved();
                self.error(WRITE_REFUSAL);
            }
            Err(WorkspaceError::Workforce(WorkforceError::Failed(_))) => {
                self.commits.put(key, CommitStatus::failure());
            }
            Err(WorkspaceError::Stale | WorkspaceError::Workforce(WorkforceError::Stale)) => {
                self.commits.put(
                    key,
                    CommitStatus::refusal(
                        &key.replace('_', " "),
                        value,
                        &["has changed. Refresh it and try again"],
                    ),
                );
            }
            Err(e) => {
                let reasons = match &e {
                    WorkspaceError::Workforce(WorkforceError::Invalid(errors)) => errors
                        .iter()
                        .map(|e| {
                            if e.field == key {
                                e.message.clone()
                            } else {
                                format!("{} {}", e.field.replace('_', " "), e.message)
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", "),
                    _ => e.to_string(),
                };
                self.commits.put(
                    key,
                    CommitStatus::refusal(&key.replace('_', " "), value, &[&reasons]),
                );
            }
        }
    }
    async fn event(&mut self, scope: &Scope, payload: &Value) -> Result<()> {
        let kind = string(payload, "type");
        self.load(scope).await?;
        match kind {
            "clear-flash" => {
                if self
                    .flash
                    .as_ref()
                    .is_some_and(|(k, _)| k.name() == string(payload, "key"))
                {
                    self.flash = None;
                }
            }
            "requests_grid" => {
                if let ColumnsOutcome::Sort(key) =
                    self.columns.handle(&params::string_values(payload))
                {
                    self.list.next_sort(&key);
                }
            }
            "cancel_review" => self.pending_review = None,
            "save_field" => self.save_inline(scope, payload).await,
            "request_change" if self.can_manage && self.employee.is_some() => {
                self.request = params::form_values(payload, "request");
                self.errors.clear();
                let request = NewChangeRequest {
                    field: self.request.get("field").cloned().unwrap_or_default(),
                    proposed_value: self
                        .request
                        .get("proposed_value")
                        .cloned()
                        .unwrap_or_default(),
                    reason: self.request.get("reason").cloned(),
                };
                match self
                    .core
                    .workspace
                    .request_change(
                        scope,
                        self.company.ok_or(Error::Invalid)?,
                        self.id,
                        &request,
                    )
                    .await
                {
                    Ok(_) => {
                        self.request.clear();
                        self.success("Change request recorded.");
                    }
                    Err(e) => self.refused(e, "Check the proposed change."),
                }
            }
            "approve_change" | "reject_change" if self.can_review && self.employee.is_some() => {
                let id = params::positive_id(payload.get("value").and_then(Value::as_str));
                self.pending_review = self
                    .requests
                    .iter()
                    .find(|r| Some(r.id) == id && r.status == "pending")
                    .map(|r| {
                        (
                            r.id,
                            if kind == "approve_change" {
                                "approved"
                            } else {
                                "rejected"
                            },
                        )
                    });
                if self.pending_review.is_none() {
                    self.error("Request cannot be reviewed.");
                }
            }
            "confirm_review" if self.can_review && self.employee.is_some() => {
                if let Some((id, decision)) = self.pending_review.take() {
                    match self
                        .core
                        .workspace
                        .review_change(
                            scope,
                            self.company.ok_or(Error::Invalid)?,
                            self.id,
                            id,
                            decision,
                        )
                        .await
                    {
                        Ok(_) => {
                            self.success("Request reviewed. Core employee details are unchanged.")
                        }
                        Err(e) => self.refused(e, "Request cannot be reviewed."),
                    }
                } else {
                    self.error("Request cannot be reviewed.");
                }
            }
            "request_change" | "approve_change" | "reject_change" | "confirm_review" => {
                self.pending_review = None;
                self.error(WRITE_REFUSAL);
            }
            _ => self.error("Check the proposed change."),
        }
        Ok(())
    }
    fn refused(&mut self, e: WorkspaceError, fallback: &str) {
        match e {
            WorkspaceError::Workforce(WorkforceError::Unauthorized) => self.error(WRITE_REFUSAL),
            WorkspaceError::Workforce(WorkforceError::CompanyArchived) => {
                self.error("This company is archived; nothing in it can change.")
            }
            WorkspaceError::Workforce(WorkforceError::Invalid(errors)) => {
                self.errors = errors;
                self.error(fallback);
            }
            _ => self.error(fallback),
        }
    }
}
impl LiveState<Scope, Error> for Detail {
    fn update<'a>(&'a mut self, request: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            self.notice.clear();
            match request.input {
                LiveInput::Event(payload) => self.event(request.scope, payload).await?,
                LiveInput::Params(query) => {
                    self.list =
                        ListState::parse(query, detail_spec()?).map_err(|_| Error::Internal)?;
                    if let Some(company) = self.company {
                        self.list
                            .filters
                            .insert("company_id".into(), company.to_string());
                    }
                }
                LiveInput::Refresh => {}
            }
            self.load(request.scope).await?;
            Ok(LiveBody {
                html: self.render()?,
                query: self.list.to_params(),
                notice: self.notice.clone(),
                rows: self.requests.len(),
                ..LiveBody::default()
            })
        })
    }
    fn panels(&self) -> Vec<PanelSpec> {
        if !self.history_available || self.employee.is_none() {
            return vec![];
        }
        [
            (
                "employee-employment-history",
                "people_workforce_employment",
                self.employment.as_ref().map(|e| e.id),
            ),
            (
                "employee-profile-history",
                "people_employee_workspace_profile",
                self.profile.as_ref().and_then(|p| p.id),
            ),
            (
                "employee-access-history",
                "people_employee_access",
                self.access.as_ref().and_then(|a| a.id),
            ),
        ]
        .into_iter()
        .filter_map(|(panel, kind, id)| {
            id.map(|id| PanelSpec {
                id: panel.into(),
                embed: "audit.record-history".into(),
                path: HashMap::from([
                    ("panel".into(), panel.into()),
                    ("kinds".into(), kind.into()),
                    ("id".into(), id.to_string()),
                ]),
                query: HashMap::new(),
            })
        })
        .collect()
    }
}
fn detail_spec() -> Result<Arc<ListSpec>> {
    ListSpec {
        sortable: ["field", "value", "status"]
            .into_iter()
            .map(|s| (s.into(), SortDir::Asc))
            .chain(std::iter::once(("id".into(), SortDir::Desc)))
            .collect(),
        default_sort: Some("id".into()),
        filters: BTreeMap::from([("company_id".into(), Filter::string(""))]),
        omit_defaults: true,
        ..ListSpec::default()
    }
    .shared()
    .map_err(|_| Error::Internal)
}
fn text_fact(title: &'static str, value: &str) -> Fact<'static> {
    Fact {
        title,
        value: Html::text(if value.is_empty() { "—" } else { value }),
        ..Fact::default()
    }
}
fn card(id: &str, title: &str, description: &str, content: Html, history: Html) -> Result<Html> {
    let mut body = render(&SectionHeading {
        id: &format!("{id}-heading"),
        title,
        description,
        actions: history,
        ..SectionHeading::default()
    })?;
    body.push(&content);
    render(&Card {
        id,
        role: "region",
        labelledby: &format!("{id}-heading"),
        class: "mt-5",
        body,
        ..Card::default()
    })
}
fn string<'a>(payload: &'a Value, key: &str) -> &'a str {
    payload.get(key).and_then(Value::as_str).unwrap_or("")
}
fn employment_change(
    key: &str,
    value: &str,
    version: i64,
) -> std::result::Result<EmploymentChange, WorkforceError> {
    let mut change = EmploymentChange::at(version);
    let field = match key {
        "employee_number" => "employee_number",
        "employment_type" => "employment_type",
        "status" => "status",
        "employment_start" => "employment_start",
        "employment_end" => "employment_end",
        "supervisor_employment_id" => "supervisor_employment_id",
        "department_id" => "department_id",
        _ => "field",
    };
    let invalid = || WorkforceError::Invalid(vec![FieldError::new(field, "is invalid")]);
    match key {
        "employee_number" => change.employee_number = Some(value.into()),
        "employment_type" => change.employment_type = Some(value.into()),
        "status" => change.status = Some(value.into()),
        "employment_start" | "employment_end" => {
            let date = if value.is_empty() {
                None
            } else {
                Some(value.parse::<Date>().map_err(|_| invalid())?)
            };
            if key == "employment_start" {
                change.employment_start = Some(date);
            } else {
                change.employment_end = Some(date);
            }
        }
        "supervisor_employment_id" | "department_id" => {
            let id = if value.is_empty() {
                None
            } else {
                Some(params::positive_id(Some(value)).ok_or_else(invalid)?)
            };
            if key == "supervisor_employment_id" {
                change.supervisor_employment_id = Some(id);
            } else {
                change.department_id = Some(id);
            }
        }
        _ => return Err(invalid()),
    }
    Ok(change)
}
