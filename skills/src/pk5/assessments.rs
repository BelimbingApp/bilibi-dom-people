//! Mirrors Bilimbi Skills.Web.AssessmentsLive; each decision rechecks authority.
use super::*;
use crate::{
    AssessableEmployee, AssessmentFilters, AssessmentInput, AssessmentQueue, AssessmentView,
    Coverage, ReassessmentView, ReviewDecision, ScoreView, Skill, ASSESSMENTS_APPROVE,
    ASSESSMENTS_REVIEW, ASSESSMENTS_SUBMIT, ASSESSMENTS_VIEW, REASSESSMENTS_EXECUTE,
    REASSESSMENTS_SUBMIT,
};
use bilibi_ui::page::{LiveBody, LiveFuture, LiveInput, LiveRequest};

struct Pending {
    company: i64,
    command: Command,
    detail: String,
}
enum Command {
    Verify(i64),
    Return(i64, String),
    Finalize(i64),
    Cancel(i64),
}
impl Command {
    fn verb(&self) -> &'static str {
        match self {
            Self::Verify(_) => "Verify",
            Self::Return(..) => "Return",
            Self::Finalize(_) => "Finalize",
            Self::Cancel(_) => "Cancel request",
        }
    }
    fn success(&self) -> &'static str {
        match self {
            Self::Verify(_) => "Assessment verified.",
            Self::Return(..) => "Assessment returned to the assessor.",
            Self::Finalize(_) => "Assessment finalized; the current score is updated.",
            Self::Cancel(_) => "Reassessment request cancelled.",
        }
    }
}

pub(super) struct Assessments {
    core: Arc<Core>,
    company: Company,
    status: String,
    available: bool,
    can_submit: bool,
    can_review: bool,
    can_finalize: bool,
    can_request: bool,
    can_perform: bool,
    can_cancel: bool,
    queue: AssessmentQueue,
    register: Vec<AssessmentView>,
    gaps: Vec<ScoreView>,
    coverage: Option<Vec<Coverage>>,
    requests: Vec<ReassessmentView>,
    people: Vec<AssessableEmployee>,
    skills: Vec<Skill>,
    review_table: Table,
    finalize_table: Table,
    returned_table: Table,
    register_table: Table,
    gaps_table: Table,
    coverage_table: Table,
    requests_table: Table,
    correcting: Option<AssessmentView>,
    returning: Option<i64>,
    performing: Option<i64>,
    pending: Option<Pending>,
    request_key: String,
    draft: HashMap<String, String>,
    request_draft: HashMap<String, String>,
    perform_draft: HashMap<String, String>,
    flash: Flash,
}
impl Assessments {
    pub(super) fn new(core: Arc<Core>, query: &HashMap<String, String>) -> Result<Self> {
        Ok(Self {
            core,
            company: Company::new(query),
            status: status(query.get("status").map(String::as_str)),
            available: false,
            can_submit: false,
            can_review: false,
            can_finalize: false,
            can_request: false,
            can_perform: false,
            can_cancel: false,
            queue: AssessmentQueue {
                review: vec![],
                finalize: vec![],
                returned: vec![],
            },
            register: vec![],
            gaps: vec![],
            coverage: None,
            requests: vec![],
            people: vec![],
            skills: vec![],
            review_table: Table::new(
                "assessment-review-queue",
                "review_grid",
                &[
                    ("employee", "Employee"),
                    ("skill", "Skill"),
                    ("level", "Level"),
                    ("result", "Result"),
                    ("assessed", "Assessed"),
                    ("evidence", "Evidence"),
                    ("controls", "Actions"),
                ],
            )?,
            finalize_table: Table::new(
                "assessment-finalize-queue",
                "finalize_grid",
                &[
                    ("employee", "Employee"),
                    ("skill", "Skill"),
                    ("level", "Level"),
                    ("controls", "Actions"),
                ],
            )?,
            returned_table: Table::new(
                "assessment-returned-queue",
                "returned_grid",
                &[
                    ("employee", "Employee"),
                    ("skill", "Skill"),
                    ("note", "Review note"),
                    ("controls", "Actions"),
                ],
            )?,
            register_table: Table::new(
                "assessment-register",
                "register_grid",
                &[
                    ("employee", "Employee"),
                    ("skill", "Skill"),
                    ("level", "Level"),
                    ("result", "Result"),
                    ("assessed", "Assessed"),
                    ("due", "Next due"),
                    ("status", "Status"),
                ],
            )?,
            gaps_table: Table::new(
                "team-gaps",
                "gaps_grid",
                &[
                    ("employee", "Employee"),
                    ("skill", "Skill"),
                    ("level", "Level"),
                    ("priority", "Priority"),
                    ("mandatory", "Mandatory"),
                    ("state", "State"),
                ],
            )?,
            coverage_table: Table::new(
                "coverage",
                "coverage_grid",
                &[
                    ("skill", "Skill"),
                    ("holders", "Holders"),
                    ("minimum", "Minimum"),
                    ("covered", "Covered"),
                ],
            )?,
            requests_table: Table::new(
                "reassessment-requests",
                "requests_grid",
                &[
                    ("employee", "Employee"),
                    ("skill", "Skill"),
                    ("due", "Due"),
                    ("reason", "Reason"),
                    ("controls", "Actions"),
                ],
            )?,
            correcting: None,
            returning: None,
            performing: None,
            pending: None,
            request_key: key(),
            draft: HashMap::new(),
            request_draft: HashMap::new(),
            perform_draft: HashMap::new(),
            flash: Flash::default(),
        })
    }
    fn reset(&mut self) {
        self.correcting = None;
        self.returning = None;
        self.performing = None;
        self.pending = None;
        self.draft.clear();
        self.request_draft.clear();
        self.perform_draft.clear();
        self.request_key = key();
    }
    pub(super) async fn load(&mut self, scope: &Scope) -> Result<()> {
        self.company
            .load(&self.core, scope, ASSESSMENTS_VIEW)
            .await?;
        self.available = false;
        self.can_submit = false;
        self.can_review = false;
        self.can_finalize = false;
        self.can_request = false;
        self.can_perform = false;
        self.can_cancel = false;
        self.queue = AssessmentQueue {
            review: vec![],
            finalize: vec![],
            returned: vec![],
        };
        self.register.clear();
        self.gaps.clear();
        self.requests.clear();
        self.people.clear();
        self.skills.clear();
        self.coverage = None;
        if let Some(c) = self.company.selected.as_ref() {
            let company = c.id;
            self.can_submit = self
                .company
                .allowed(&self.core, scope, ASSESSMENTS_SUBMIT)
                .await;
            self.can_review = self
                .company
                .allowed(&self.core, scope, ASSESSMENTS_REVIEW)
                .await;
            self.can_finalize = self
                .company
                .allowed(&self.core, scope, ASSESSMENTS_APPROVE)
                .await;
            self.can_request = self
                .company
                .allowed(&self.core, scope, REASSESSMENTS_SUBMIT)
                .await;
            self.can_perform = self
                .company
                .allowed(&self.core, scope, REASSESSMENTS_EXECUTE)
                .await;
            self.can_cancel = self.can_request || self.can_perform;
            // Mandatory reads decide availability. Optional capabilities never hide the register.
            let filters = AssessmentFilters {
                status: optional(&self.status),
                ..AssessmentFilters::default()
            };
            let data = async {
                Ok::<_, SkillsError>((
                    self.core.skills.assessment_queue(scope, company).await?,
                    self.core
                        .skills
                        .list_assessments(scope, company, &filters)
                        .await?,
                    self.core.skills.gaps(scope, company).await?,
                    self.core.skills.list_skills(scope, company).await?,
                ))
            }
            .await;
            if let Ok((queue, register, gaps, skills)) = data {
                self.queue = queue;
                self.register = register;
                self.gaps = gaps;
                self.skills = skills.into_iter().filter(|s| s.active).collect();
                self.people = self
                    .core
                    .skills
                    .assessable(scope, company)
                    .await
                    .unwrap_or_default();
                self.requests = self
                    .core
                    .skills
                    .pending_reassessments(scope, company)
                    .await
                    .unwrap_or_default();
                self.coverage = self
                    .core
                    .skills
                    .coverage(scope, company, today())
                    .await
                    .ok();
                self.available = true;
            }
        }
        self.review_table
            .load(&self.queue.review, |r| r.assessment.id.to_string());
        self.finalize_table
            .load(&self.queue.finalize, |r| r.assessment.id.to_string());
        self.returned_table
            .load(&self.queue.returned, |r| r.assessment.id.to_string());
        self.register_table
            .load(&self.register, |r| r.assessment.id.to_string());
        self.gaps_table.load(&self.gaps, |r| r.score.id.to_string());
        self.requests_table
            .load(&self.requests, |r| r.request.id.to_string());
        self.coverage_table
            .load(self.coverage.as_deref().unwrap_or_default(), |r| {
                r.skill_id.to_string()
            });
        Ok(())
    }
    async fn allowed(&mut self, scope: &Scope, cap: &str) -> bool {
        if self.company.allowed(&self.core, scope, cap).await {
            true
        } else {
            self.flash.error("You cannot do that for this company.");
            false
        }
    }
    async fn event(&mut self, scope: &Scope, p: &Value) -> Result<()> {
        let event = text(p, "type");
        match event {
            "clear-flash" => self.flash.clear(p),
            "select_company" => {
                self.company.id = id(p, "company_id");
                self.reset();
            }
            "filter_register" => self.status = status(Some(text(p, "status"))),
            "review_grid" => self.review_table.event(p),
            "finalize_grid" => self.finalize_table.event(p),
            "returned_grid" => self.returned_table.event(p),
            "register_grid" => self.register_table.event(p),
            "gaps_grid" => self.gaps_table.event(p),
            "coverage_grid" => self.coverage_table.event(p),
            "requests_grid" => self.requests_table.event(p),
            "dismiss_confirm" => self.pending = None,
            "dismiss_return" => self.returning = None,
            "dismiss_perform" => {
                self.performing = None;
                self.perform_draft.clear();
            }
            "dismiss_correction" => {
                self.correcting = None;
                self.draft.clear();
                self.request_key = key();
            }
            "confirm" => self.commit(scope).await,
            "start_correction" => {
                if self.allowed(scope, ASSESSMENTS_SUBMIT).await {
                    self.correcting = self
                        .queue
                        .returned
                        .iter()
                        .find(|r| Some(r.assessment.id) == id(p, "id"))
                        .cloned();
                    self.draft.clear();
                    self.request_key = key();
                }
            }
            "create_assessment" => {
                if self.allowed(scope, ASSESSMENTS_SUBMIT).await {
                    self.draft = form_values(p, "assessment");
                    let input =
                        assessment_input(&self.draft, &self.request_key).map(|mut input| {
                            input.supersedes_assessment_id =
                                self.correcting.as_ref().map(|r| r.assessment.id);
                            input
                        });
                    let result = match (self.company.id, input) {
                        (Some(c), Ok(input)) => {
                            self.core.skills.submit_assessment(scope, c, &input).await
                        }
                        (_, Err(e)) => Err(e),
                        _ => Err(SkillsError::Unauthorized),
                    };
                    if self
                        .flash
                        .result(result, "Assessment submitted for review.")
                    {
                        self.correcting = None;
                        self.draft.clear();
                        self.request_key = key();
                    }
                }
            }
            "review_assessment" | "start_return" | "finalize_assessment" => {
                let cap = if event == "finalize_assessment" {
                    ASSESSMENTS_APPROVE
                } else {
                    ASSESSMENTS_REVIEW
                };
                if self.allowed(scope, cap).await {
                    let rows = if event == "finalize_assessment" {
                        &self.queue.finalize
                    } else {
                        &self.queue.review
                    };
                    if let Some(v) = rows.iter().find(|r| Some(r.assessment.id) == id(p, "id")) {
                        let aid = v.assessment.id;
                        if event == "start_return" {
                            self.returning = Some(aid);
                        } else {
                            self.pending = self.company.id.map(|company| Pending {
                                company,
                                detail: assessment_detail(v),
                                command: if event == "review_assessment" {
                                    Command::Verify(aid)
                                } else {
                                    Command::Finalize(aid)
                                },
                            });
                        }
                    } else {
                        self.flash.error("That item is no longer pending.");
                    }
                }
            }
            "return_assessment" => {
                if self.allowed(scope, ASSESSMENTS_REVIEW).await {
                    let note = text(p, "note").trim();
                    if note.is_empty() {
                        self.flash
                            .error("A note is required to return an assessment.");
                    } else if let Some(v) = self
                        .queue
                        .review
                        .iter()
                        .find(|r| Some(r.assessment.id) == self.returning)
                    {
                        self.pending = self.company.id.map(|company| Pending {
                            company,
                            detail: format!("{} · {}", assessment_detail(v), note),
                            command: Command::Return(v.assessment.id, note.to_owned()),
                        });
                        self.returning = None;
                    } else {
                        self.flash.error("That item is no longer pending.");
                    }
                }
            }
            "create_reassessment_request" => {
                if self.allowed(scope, REASSESSMENTS_SUBMIT).await {
                    self.request_draft = form_values(p, "request");
                    let get = |k: &str| {
                        self.request_draft
                            .get(k)
                            .map(String::as_str)
                            .unwrap_or_default()
                    };
                    let employee =
                        params::positive_id(Some(get("employee_id"))).unwrap_or_default();
                    let skill = params::positive_id(Some(get("skill_id"))).unwrap_or_default();
                    let result = self
                        .core
                        .skills
                        .request_reassessment(
                            scope,
                            self.company.id.unwrap_or_default(),
                            employee,
                            skill,
                            get("reason"),
                        )
                        .await;
                    if self.flash.result(result, "Reassessment requested.") {
                        self.request_draft.clear();
                    }
                }
            }
            "start_perform" => {
                if self.allowed(scope, REASSESSMENTS_EXECUTE).await {
                    self.performing = self
                        .requests
                        .iter()
                        .find(|r| Some(r.request.id) == id(p, "id"))
                        .map(|r| r.request.id);
                    self.perform_draft.clear();
                }
            }
            "perform_reassessment" => {
                if self.allowed(scope, REASSESSMENTS_EXECUTE).await
                    && self.allowed(scope, ASSESSMENTS_SUBMIT).await
                {
                    self.perform_draft = form_values(p, "perform");
                    let input = assessment_input(&self.perform_draft, &self.request_key);
                    let result = match (self.performing, self.company.id, input) {
                        (Some(r), Some(c), Ok(input)) => {
                            self.core
                                .skills
                                .perform_reassessment(scope, c, r, &input)
                                .await
                        }
                        (_, _, Err(e)) => Err(e),
                        _ => Err(SkillsError::NotPending),
                    };
                    if self
                        .flash
                        .result(result, "Reassessment submitted for review.")
                    {
                        self.performing = None;
                        self.perform_draft.clear();
                    }
                }
            }
            "cancel_reassessment" => {
                let executable = self
                    .company
                    .allowed(&self.core, scope, REASSESSMENTS_EXECUTE)
                    .await;
                if !executable && !self.allowed(scope, REASSESSMENTS_SUBMIT).await {
                    return Ok(());
                }
                if let Some(v) = self
                    .requests
                    .iter()
                    .find(|r| Some(r.request.id) == id(p, "id"))
                {
                    self.pending = self.company.id.map(|company| Pending {
                        company,
                        command: Command::Cancel(v.request.id),
                        detail: format!(
                            "{} · {} · due {}",
                            v.employee_name,
                            v.skill_name.as_deref().unwrap_or_default(),
                            shown(v.request.due_on)
                        ),
                    });
                }
            }
            _ => (),
        }
        Ok(())
    }
    async fn commit(&mut self, scope: &Scope) {
        let Some(p) = self.pending.take() else {
            return;
        };
        if Some(p.company) != self.company.id {
            self.flash.error("You cannot do that for this company.");
            return;
        }
        let cap = match p.command {
            Command::Verify(_) | Command::Return(..) => ASSESSMENTS_REVIEW,
            Command::Finalize(_) => ASSESSMENTS_APPROVE,
            Command::Cancel(_) => {
                if self
                    .company
                    .allowed(&self.core, scope, REASSESSMENTS_EXECUTE)
                    .await
                {
                    REASSESSMENTS_EXECUTE
                } else {
                    REASSESSMENTS_SUBMIT
                }
            }
        };
        if !self.allowed(scope, cap).await {
            return;
        }
        let success = p.command.success();
        let result = match p.command {
            Command::Verify(id) => self
                .core
                .skills
                .review_assessment(scope, p.company, id, ReviewDecision::Verify, None)
                .await
                .map(|_| ()),
            Command::Return(id, note) => self
                .core
                .skills
                .review_assessment(scope, p.company, id, ReviewDecision::Return, Some(&note))
                .await
                .map(|_| ()),
            Command::Finalize(id) => self
                .core
                .skills
                .finalize_assessment(scope, p.company, id)
                .await
                .map(|_| ()),
            Command::Cancel(id) => self
                .core
                .skills
                .cancel_reassessment(scope, p.company, id)
                .await
                .map(|_| ()),
        };
        self.flash.result(result, success);
    }
    fn query(&self) -> HashMap<String, String> {
        let mut q = self.company.query();
        if !self.status.is_empty() {
            q.insert("status".into(), self.status.clone());
        }
        q
    }
}
fn status(value: Option<&str>) -> String {
    value
        .filter(|s| ["pending_review", "verified", "returned", "finalized"].contains(s))
        .unwrap_or_default()
        .into()
}
fn today() -> jiff::civil::Date {
    jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date()
}
fn shown<T: std::fmt::Display>(v: Option<T>) -> String {
    v.map(|v| v.to_string()).unwrap_or_default()
}
fn assessment_detail(v: &AssessmentView) -> String {
    format!(
        "{} · {} · level {} of {} required · {} · {} · Evidence: {}",
        v.employee_name,
        v.skill_name.as_deref().unwrap_or_default(),
        v.assessment.assessed_level,
        v.assessment.required_level,
        label(&v.assessment.result_band),
        shown(v.assessment.assessed_on),
        v.assessment.evidence
    )
}
fn assessment_input(draft: &HashMap<String, String>, key: &str) -> crate::Result<AssessmentInput> {
    let get = |k: &str| draft.get(k).map(String::as_str).unwrap_or_default();
    Ok(AssessmentInput {
        employee_id: params::positive_id(Some(get("employee_id"))).unwrap_or_default(),
        skill_id: params::positive_id(Some(get("skill_id"))).unwrap_or_default(),
        assessed_level: get("assessed_level")
            .parse()
            .map_err(|_| SkillsError::InvalidAssessment)?,
        evidence: get("evidence").into(),
        assessed_on: date(get("assessed_on"))?,
        valid_until: date(get("valid_until"))?,
        method: optional(get("method")),
        notes: optional(get("notes")),
        request_key: key.into(),
        supersedes_assessment_id: None,
    })
}

impl Assessments {
    fn company_field(&self, mut field: Field) -> Field {
        field.id = format!(
            "{}-company-{}",
            field.id,
            self.company.id.unwrap_or_default()
        );
        field
    }
    fn assessment_form(&self) -> Result<Html> {
        if !self.can_submit {
            return empty(
                "assessment-form-denied",
                "You do not assess employees for this company.",
            );
        }
        if self.people.is_empty() {
            return empty(
                "assessment-employees-empty",
                "No employee is within your reach to assess.",
            );
        }
        if self.skills.is_empty() {
            return Ok(Html::EMPTY);
        }
        let get = |k: &str| self.draft.get(k).cloned().unwrap_or_default();
        let f = |k: &str, title: &str| {
            self.company_field(field(&format!("assessment[{k}]"), title))
                .value(get(k))
        };
        let corrected = self.correcting.as_ref().map(|r| &r.assessment);
        let employee = self
            .draft
            .get("employee_id")
            .cloned()
            .or_else(|| corrected.map(|a| a.employee_id.to_string()))
            .unwrap_or_default();
        let skill = self
            .draft
            .get("skill_id")
            .cloned()
            .or_else(|| corrected.map(|a| a.skill_id.to_string()))
            .unwrap_or_default();
        let mut result = form(
            "assessment-form",
            "create_assessment",
            vec![
                select(
                    "assessment[employee_id]",
                    "Employee",
                    self.people
                        .iter()
                        .map(|p| (p.name.clone(), p.id.to_string()))
                        .collect(),
                )
                .value(employee)
                .id(format!(
                    "assessment-employee-company-{}",
                    self.company.id.unwrap_or_default()
                )),
                select(
                    "assessment[skill_id]",
                    "Skill",
                    self.skills
                        .iter()
                        .map(|s| (s.name.clone(), s.id.to_string()))
                        .collect(),
                )
                .value(skill)
                .id(format!(
                    "assessment-skill-company-{}",
                    self.company.id.unwrap_or_default()
                )),
                f("assessed_level", "Assessed level")
                    .kind(InputKind::Number)
                    .attr("min", "0")
                    .attr("max", "20")
                    .required(),
                f("assessed_on", "Assessed on").kind(InputKind::Date).value(
                    self.draft
                        .get("assessed_on")
                        .cloned()
                        .unwrap_or_else(|| today().to_string()),
                ),
                f("evidence", "Evidence")
                    .kind(InputKind::Textarea)
                    .required()
                    .attr("maxlength", "4000"),
                f("method", "Method").attr("maxlength", "80"),
                f("valid_until", "Valid until").kind(InputKind::Date),
                f("notes", "Notes").attr("maxlength", "2000"),
            ],
            if corrected.is_some() {
                "Submit correction"
            } else {
                "Submit for review"
            },
        )?;
        if corrected.is_some() {
            result.push(&button("dismiss_correction", "Cancel correction", 0)?);
        }
        Ok(result)
    }
    fn request_form(&self) -> Result<Html> {
        if !self.can_request || self.people.is_empty() || self.skills.is_empty() {
            return Ok(Html::EMPTY);
        }
        let get = |k: &str| self.request_draft.get(k).cloned().unwrap_or_default();
        form(
            "reassessment-form",
            "create_reassessment_request",
            vec![
                select(
                    "request[employee_id]",
                    "Employee",
                    self.people
                        .iter()
                        .map(|p| (p.name.clone(), p.id.to_string()))
                        .collect(),
                )
                .value(get("employee_id")),
                select(
                    "request[skill_id]",
                    "Skill",
                    self.skills
                        .iter()
                        .map(|s| (s.name.clone(), s.id.to_string()))
                        .collect(),
                )
                .value(get("skill_id")),
                field("request[reason]", "Reason")
                    .value(get("reason"))
                    .attr("maxlength", "1000")
                    .required(),
            ],
            "Request reassessment",
        )
    }
    fn queue(&self) -> Result<Html> {
        let mut body = Html::EMPTY;
        if self.queue.review.is_empty()
            && self.queue.finalize.is_empty()
            && self.queue.returned.is_empty()
        {
            return empty(
                "assessment-queue-empty",
                "Nothing is waiting for your review, finalization or correction.",
            );
        }
        if !self.queue.review.is_empty() {
            let rows = self
                .queue
                .review
                .iter()
                .map(|v| {
                    let a = &v.assessment;
                    let mut controls = Html::EMPTY;
                    if self.can_review {
                        controls.push(&button("review_assessment", "Verify", a.id)?);
                        controls.push(&button("start_return", "Return", a.id)?);
                    }
                    Ok(row("review", a.id)
                        .slot("employee", Html::text(&v.employee_name))
                        .slot(
                            "skill",
                            Html::text(v.skill_name.as_deref().unwrap_or_default()),
                        )
                        .slot(
                            "level",
                            Html::text(&format!("{} / {}", a.assessed_level, a.required_level)),
                        )
                        .slot("result", Html::text(&label(&a.result_band)))
                        .slot("assessed", Html::text(&shown(a.assessed_on)))
                        .slot("evidence", Html::text(&a.evidence))
                        .slot("controls", controls))
                })
                .collect::<Result<Vec<_>>>()?;
            body.push(&section(
                "assessment-review-section",
                "To review",
                self.review_table.render("To review", rows, "")?,
            )?);
        }
        if let Some(v) = self
            .queue
            .review
            .iter()
            .find(|r| Some(r.assessment.id) == self.returning)
        {
            let mut f = form(
                &format!("return-form-{}", v.assessment.id),
                "return_assessment",
                vec![field("note", "Reason for returning")
                    .attr("maxlength", "2000")
                    .required()],
                "Return",
            )?;
            f.push(&button("dismiss_return", "Cancel", 0)?);
            body.push(&section(
                "return-assessment-section",
                &format!(
                    "Return: {} · {}",
                    v.employee_name,
                    v.skill_name.as_deref().unwrap_or_default()
                ),
                f,
            )?);
        }
        if !self.queue.finalize.is_empty() {
            let rows = self
                .queue
                .finalize
                .iter()
                .map(|v| {
                    let a = &v.assessment;
                    Ok(row("finalize", a.id)
                        .slot("employee", Html::text(&v.employee_name))
                        .slot(
                            "skill",
                            Html::text(v.skill_name.as_deref().unwrap_or_default()),
                        )
                        .slot("level", Html::text(&a.assessed_level.to_string()))
                        .slot(
                            "controls",
                            if self.can_finalize {
                                button("finalize_assessment", "Finalize", a.id)?
                            } else {
                                Html::EMPTY
                            },
                        ))
                })
                .collect::<Result<Vec<_>>>()?;
            body.push(&section(
                "assessment-finalize-section",
                "To finalize",
                self.finalize_table.render("To finalize", rows, "")?,
            )?);
        }
        if !self.queue.returned.is_empty() {
            let rows = self
                .queue
                .returned
                .iter()
                .map(|v| {
                    let a = &v.assessment;
                    Ok(row("returned", a.id)
                        .slot("employee", Html::text(&v.employee_name))
                        .slot(
                            "skill",
                            Html::text(v.skill_name.as_deref().unwrap_or_default()),
                        )
                        .slot(
                            "note",
                            Html::text(a.review_note.as_deref().unwrap_or_default()),
                        )
                        .slot(
                            "controls",
                            if self.can_submit {
                                button("start_correction", "Correct", a.id)?
                            } else {
                                Html::EMPTY
                            },
                        ))
                })
                .collect::<Result<Vec<_>>>()?;
            body.push(&section(
                "assessment-returned-section",
                "Returned to you",
                self.returned_table.render("Returned to you", rows, "")?,
            )?);
        }
        Ok(body)
    }
    fn reassessments(&self) -> Result<Html> {
        let rows = self
            .requests
            .iter()
            .map(|v| {
                let r = &v.request;
                let mut controls = Html::EMPTY;
                if self.can_perform {
                    controls.push(&button("start_perform", "Perform", r.id)?);
                }
                if self.can_cancel {
                    controls.push(&button("cancel_reassessment", "Cancel request", r.id)?);
                }
                Ok(row("reassessment", r.id)
                    .slot("employee", Html::text(&v.employee_name))
                    .slot(
                        "skill",
                        Html::text(v.skill_name.as_deref().unwrap_or_default()),
                    )
                    .slot("due", Html::text(&shown(r.due_on)))
                    .slot("reason", Html::text(&r.reason))
                    .slot("controls", controls))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut body = self.requests_table.render(
            "Reassessment requests",
            rows,
            "No reassessment is waiting.",
        )?;
        if let Some(v) = self
            .requests
            .iter()
            .find(|v| Some(v.request.id) == self.performing)
        {
            let get = |k: &str| self.perform_draft.get(k).cloned().unwrap_or_default();
            let mut f = form(
                &format!("perform-form-{}", v.request.id),
                "perform_reassessment",
                vec![
                    field("perform[assessed_level]", "Reassessed level")
                        .kind(InputKind::Number)
                        .attr("min", "0")
                        .attr("max", "20")
                        .required()
                        .value(get("assessed_level")),
                    field("perform[evidence]", "Reassessment evidence")
                        .attr("maxlength", "4000")
                        .required()
                        .value(get("evidence")),
                ],
                "Perform",
            )?;
            f.push(&button("dismiss_perform", "Cancel", 0)?);
            body.push(&section(
                "perform-reassessment-section",
                &format!(
                    "{} · {}",
                    v.employee_name,
                    v.skill_name.as_deref().unwrap_or_default()
                ),
                f,
            )?);
        }
        body.push(&self.request_form()?);
        Ok(body)
    }
    pub(super) fn render(&self) -> Result<String> {
        use askama::Template;
        let mut body = Html::EMPTY;
        if self.company.selected.is_none() {
            body.push(&empty(
                "assessments-no-company",
                "No active company is available for assessments.",
            )?);
        } else if !self.available {
            body.push(&empty(
                "assessments-unavailable",
                "Assessments are unavailable while this company's workforce is not current.",
            )?);
        } else {
            body.push(&section(
                "assessment-queue-section",
                "Waiting on you",
                self.queue()?,
            )?);
            body.push(&render(&AssessmentForms {
                assessment: section(
                    "assessment-form-section",
                    "Assess an employee",
                    self.assessment_form()?,
                )?,
                reassessment: section(
                    "reassessment-section",
                    "Reassessment requests",
                    self.reassessments()?,
                )?,
            })?);
            let mut register = render(&Form {
                id: "register-filter",
                event: "filter_register",
                change: true,
                body: render(
                    &select(
                        "status",
                        "Status",
                        ["", "pending_review", "verified", "returned", "finalized"]
                            .into_iter()
                            .map(|s| (if s.is_empty() { "All".into() } else { label(s) }, s.into()))
                            .collect(),
                    )
                    .value(&self.status),
                )?,
                ..Form::default()
            })?;
            let rows = self
                .register
                .iter()
                .map(|v| {
                    let a = &v.assessment;
                    row("assessment", a.id)
                        .slot("employee", Html::text(&v.employee_name))
                        .slot(
                            "skill",
                            Html::text(v.skill_name.as_deref().unwrap_or_default()),
                        )
                        .slot(
                            "level",
                            Html::text(&format!("{} / {}", a.assessed_level, a.required_level)),
                        )
                        .slot("result", Html::text(&label(&a.result_band)))
                        .slot("assessed", Html::text(&shown(a.assessed_on)))
                        .slot("due", Html::text(&shown(a.next_due_on)))
                        .slot("status", Html::text(&label(&a.status)))
                })
                .collect();
            register.push(&self.register_table.render(
                "Register",
                rows,
                "No assessments have been recorded within your reach.",
            )?);
            body.push(&section(
                "assessment-register-section",
                "Register",
                register,
            )?);
            let rows = self
                .gaps
                .iter()
                .map(|v| {
                    let s = &v.score;
                    row("gap", s.id)
                        .slot("employee", Html::text(&v.employee_name))
                        .slot(
                            "skill",
                            Html::text(v.skill_name.as_deref().unwrap_or_default()),
                        )
                        .slot(
                            "level",
                            Html::text(&format!("{} / {}", s.current_level, s.required_level)),
                        )
                        .slot("priority", Html::text(&s.priority_score.to_string()))
                        .slot(
                            "mandatory",
                            Html::text(if s.mandatory { "Yes" } else { "No" }),
                        )
                        .slot("state", Html::text(&label(&v.state)))
                })
                .collect();
            body.push(&section(
                "team-gaps-section",
                "Gaps",
                self.gaps_table
                    .render("Gaps", rows, "No current skill gaps within your reach.")?,
            )?);
            if let Some(coverage) = &self.coverage {
                let rows = coverage
                    .iter()
                    .map(|r| {
                        row("coverage", r.skill_id)
                            .slot("skill", Html::text(&r.name))
                            .slot("holders", Html::text(&r.holders.to_string()))
                            .slot("minimum", Html::text(&r.minimum.to_string()))
                            .slot("covered", Html::text(if r.covered { "Yes" } else { "No" }))
                    })
                    .collect();
                body.push(&section(
                    "coverage-section",
                    "Critical-skill coverage",
                    self.coverage_table.render(
                        "Critical-skill coverage",
                        rows,
                        "No skill is marked critical for this company.",
                    )?,
                )?);
            }
        }
        let confirmation = match &self.pending {
            Some(p) => confirm(
                true,
                &self.company,
                &format!("{}?", p.command.verb()),
                &p.detail,
                p.command.verb(),
            )?,
            None => Html::EMPTY,
        };
        View {
            flash: self.flash.map(),
            page_id: "assessments-page",
            title: "Assessments",
            subtitle: self
                .company
                .selected
                .as_ref()
                .map(|c| {
                    format!(
                        "{} · Evidence-backed skill assessments and reassessment",
                        c.name
                    )
                })
                .unwrap_or_default(),
            has_company: !self.company.companies.is_empty(),
            company_form: "assessments-company-form",
            company: self.company.field("assessments"),
            body,
            confirm: confirmation,
        }
        .render()
        .map_err(|_| Error::Internal)
    }
}
impl LiveState<Scope, Error> for Assessments {
    fn update<'a>(&'a mut self, request: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            match request.input {
                LiveInput::Event(p) => self.event(request.scope, p).await?,
                LiveInput::Params(q) => {
                    let company = params::positive_id(q.get("company_id").map(String::as_str));
                    if self.company.id != company {
                        self.reset();
                    }
                    self.company.id = company;
                    self.status = status(q.get("status").map(String::as_str));
                    self.pending = None;
                }
                LiveInput::Refresh => (),
            }
            self.load(request.scope).await?;
            Ok(LiveBody {
                html: self.render()?,
                query: self.query(),
                rows: self.register.len(),
                ..LiveBody::default()
            })
        })
    }
}

// Evidence permits 4,000 characters; params::form_values deliberately only
// accepts 1 KiB per field. Keep these bounded long values for owner validation.
fn form_values(payload: &Value, namespace: &str) -> HashMap<String, String> {
    payload
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(k, v)| {
            let key = k
                .strip_prefix(namespace)?
                .strip_prefix('[')?
                .strip_suffix(']')?;
            let value = v.as_str().filter(|v| v.len() <= 16_000)?;
            Some((key.to_owned(), value.to_owned()))
        })
        .take(32)
        .collect()
}

#[derive(askama::Template)]
#[template(path = "pk5_assessment_forms.html")]
struct AssessmentForms {
    assessment: Html,
    reassessment: Html,
}
