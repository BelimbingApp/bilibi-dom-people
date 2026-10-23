//! Mirrors Bilimbi Training.Web.{EffectivenessLive,LearningLive}'s review UI.
//! Both entry points use the same server-held answer and confirmation state.
use crate::*;
use askama::Template;
use bilibi_database::{Error, Result as DbResult};
use bilibi_ui::{params, *};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap};

pub(crate) fn render(t: &impl Template) -> DbResult<Html> {
    Html::render(t).map_err(|_| Error::Internal)
}
pub(crate) fn columns(id: &str, labels: &[(&str, &str)]) -> DbResult<ListColumns> {
    ListColumns::mount(
        id,
        labels
            .iter()
            .map(|(k, l)| BuiltinColumn::new(k, l))
            .collect(),
    )
    .map_err(|_| Error::Internal)
}
pub(crate) fn table(
    id: &str,
    columns: &ListColumns,
    rows: Vec<FlexRow>,
    event: &str,
    title: &str,
    reason: &str,
) -> DbResult<Html> {
    let mut table = FlexTable::new(id, columns.flex_columns(), rows);
    table.event = event;
    table.zoom = columns.zoom;
    table.suggestions = columns.flex_suggestions();
    table.add_query = &columns.add_query;
    table.empty = Some(TableEmpty::State(EmptyState::missing(title, reason)));
    render(&table)
}
pub(crate) fn button(event: &str, label: &str, value: &str) -> DbResult<Html> {
    render(&Button {
        event,
        name: "value",
        value,
        button_type: "button",
        body: Html::text(label),
        ..Button::default()
    })
}
pub(crate) fn submit(label: &str) -> DbResult<Html> {
    render(&Button {
        button_type: "submit",
        variant: ButtonVariant::Primary,
        body: Html::text(label),
        ..Button::default()
    })
}
pub(crate) fn form(id: &str, event: &str, fields: Vec<Field>, actions: Html) -> DbResult<Html> {
    render(&Fields {
        id,
        event,
        fields,
        actions,
    })
}
#[derive(Template)]
#[template(path = "learning_fields.html")]
struct Fields<'a> {
    id: &'a str,
    event: &'a str,
    fields: Vec<Field>,
    actions: Html,
}
#[derive(Template)]
#[template(path = "evaluation_section.html")]
pub(crate) struct Section<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub description: &'a str,
    pub body: Html,
}
pub(crate) fn posted(payload: &Value, name: &str) -> String {
    payload
        .get(name)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}
/// Read every posted key: a 50-criterion policy exceeds form_values' 64-field cap.
pub(crate) fn draft(payload: &Value, prefix: &str) -> HashMap<String, String> {
    payload
        .as_object()
        .into_iter()
        .flat_map(|o| o.iter())
        .filter(|(k, _)| k.starts_with(prefix))
        .filter_map(|(k, v)| {
            v.as_str()
                .filter(|s| s.len() <= 16_000)
                .map(|s| (k.clone(), s.to_owned()))
        })
        .collect()
}
pub(crate) fn spec(filters: &[&str], prefix: &str) -> DbResult<Arc<ListSpec>> {
    ListSpec {
        page_size_param: "perPage".into(),
        param_prefix: prefix.into(),
        filters: filters
            .iter()
            .map(|k| (k.to_string(), Filter::string("")))
            .collect::<BTreeMap<_, _>>(),
        omit_blank: filters.iter().map(|k| k.to_string()).collect(),
        ..ListSpec::default()
    }
    .shared()
    .map_err(|_| Error::Internal)
}
pub(crate) fn flash<'a>(value: &'a Option<(StatusKind, String)>) -> FlashMap<'a> {
    let mut result = NO_FLASH;
    if let Some((kind, text)) = value {
        match kind {
            StatusKind::Error => result.error = text,
            StatusKind::Success => result.success = text,
            StatusKind::Info => result.info = text,
            StatusKind::Warning => result.warning = text,
        }
    }
    result
}

pub(crate) struct Reviews {
    kind: EvaluationKind,
    company: Option<i64>,
    pub list: ListState,
    columns: ListColumns,
    tasks: Vec<EvaluationTask>,
    pub enabled: bool,
    error: Option<String>,
    selected: Option<EvaluationTask>,
    pending: Option<(EvaluationTask, Value, String)>,
    draft: HashMap<String, String>,
    summary: ErrorSummary,
}
impl Reviews {
    pub fn new(kind: EvaluationKind, query: &HashMap<String, String>) -> DbResult<Self> {
        let own = kind == EvaluationKind::Evaluation;
        Ok(Self {
            kind,
            company: None,
            list: ListState::parse(query, spec(&[], if own { "evaluation_" } else { "" })?)
                .map_err(|_| Error::Invalid)?,
            columns: columns(
                if own {
                    "my-evaluation-tasks"
                } else {
                    "effectiveness-reviews"
                },
                &[
                    ("employee", "Employee"),
                    ("session", "Session"),
                    ("due", "Due"),
                    ("version", "Criteria version"),
                    ("status", "Status"),
                    ("answer", "Answer"),
                ][usize::from(own)..],
            )?,
            tasks: vec![],
            enabled: false,
            error: None,
            selected: None,
            pending: None,
            draft: HashMap::new(),
            summary: ErrorSummary::new("evaluation-answer-errors"),
        })
    }
    fn own(&self) -> bool {
        self.kind == EvaluationKind::Evaluation
    }
    fn event_name(&self, own: &'static str, hod: &'static str) -> &'static str {
        if self.own() {
            own
        } else {
            hod
        }
    }
    fn refusal(&self) -> &'static str {
        self.event_name(
            "You cannot answer this company's evaluations.",
            "You cannot answer this company's reviews.",
        )
    }
    pub fn reset(&mut self) {
        self.selected = None;
        self.pending = None;
        self.draft.clear();
    }
    pub async fn load(&mut self, training: &Training, scope: &Scope, company: Option<i64>) {
        if self.company != company {
            self.reset();
            self.company = company;
        }
        self.tasks.clear();
        self.error = None;
        self.enabled = false;
        let Some(company) = company else {
            self.reset();
            return;
        };
        let caps = if self.own() {
            [REQUESTS_SUBMIT, EVALUATION_SUBMIT]
        } else {
            [EFFECTIVENESS_VIEW, EFFECTIVENESS_ANSWER]
        };
        self.enabled = training.allowed(scope, company, caps[0]).await
            && training.allowed(scope, company, caps[1]).await;
        if !self.enabled {
            self.reset();
            return;
        }
        match training.evaluation_reviews(scope, company, self.kind).await {
            Ok(tasks) => {
                self.tasks = tasks;
                self.list.page = self.list.paginate(&self.tasks).counts.page;
                let visible = |id| {
                    self.tasks
                        .iter()
                        .any(|t| t.review.id == id && t.answer.is_none())
                };
                if self
                    .selected
                    .as_ref()
                    .is_some_and(|t| !visible(t.review.id))
                    || self
                        .pending
                        .as_ref()
                        .is_some_and(|(t, _, _)| !visible(t.review.id))
                {
                    self.reset();
                }
            }
            Err(e) => {
                self.error = Some(e.to_string());
                self.reset();
            }
        }
    }
    pub fn handles(&self, event: &str) -> bool {
        if self.own() {
            matches!(
                event,
                "open_evaluation"
                    | "save_evaluation"
                    | "confirm_evaluation"
                    | "close_evaluation"
                    | "cancel_evaluation"
                    | "evaluation_grid"
                    | "evaluation_page"
                    | "evaluation_filters"
            )
        } else {
            matches!(
                event,
                "open_answer"
                    | "save_answer"
                    | "confirm_answer"
                    | "close_answer"
                    | "cancel_answer"
                    | "review_grid"
                    | "page"
                    | "review_filters"
            )
        }
    }
    pub async fn event(
        &mut self,
        training: &Training,
        scope: &Scope,
        payload: &Value,
    ) -> DbResult<Option<(StatusKind, String)>> {
        let event = payload.get("type").and_then(Value::as_str).unwrap_or("");
        if event == self.event_name("evaluation_grid", "review_grid") {
            self.columns.handle(&params::string_values(payload));
            return Ok(None);
        }
        if event == self.event_name("evaluation_page", "page") {
            self.list
                .put_page(payload.get("page").and_then(Value::as_str));
            return Ok(None);
        }
        if event == self.event_name("evaluation_filters", "review_filters") {
            self.list
                .apply_filters(&params::form_values(payload, "filters"));
            return Ok(None);
        }
        if event == self.event_name("close_evaluation", "close_answer") {
            self.selected = None;
            return Ok(None);
        }
        if event == self.event_name("cancel_evaluation", "cancel_answer") {
            self.pending = None;
            return Ok(None);
        }
        // The caller reloads current grants; recheck here before every write-shaped event too.
        let Some(company) = self.company else {
            return Ok(Some((StatusKind::Error, self.refusal().into())));
        };
        let cap = if self.own() {
            EVALUATION_SUBMIT
        } else {
            EFFECTIVENESS_ANSWER
        };
        if !self.enabled || !training.allowed(scope, company, cap).await {
            self.reset();
            return Ok(Some((StatusKind::Error, self.refusal().into())));
        }
        if event == self.event_name("open_evaluation", "open_answer") {
            let id = params::positive_id(
                payload
                    .get("value")
                    .or_else(|| payload.get("id"))
                    .and_then(Value::as_str),
            );
            self.selected = self
                .tasks
                .iter()
                .find(|r| Some(r.review.id) == id && r.answer.is_none())
                .cloned();
            self.draft.clear();
            self.summary = ErrorSummary::new("evaluation-answer-errors");
            if self.selected.is_none() {
                return Ok(Some((
                    StatusKind::Error,
                    self.event_name(
                        "That unanswered evaluation is not available to you.",
                        "That unanswered review is not available to you.",
                    )
                    .into(),
                )));
            }
        } else if event == self.event_name("save_evaluation", "save_answer") {
            let Some(selected) = self.selected.clone() else {
                return Ok(Some((
                    StatusKind::Error,
                    "Choose an unanswered review first.".into(),
                )));
            };
            let root = if self.own() { "evaluation" } else { "entry" };
            self.draft = draft(payload, &format!("{root}["));
            let mut values = Map::new();
            for criterion in &selected.criteria {
                let value = posted(payload, &format!("{root}[values][{}]", criterion.code));
                // Only an explicit blank is unknown; malformed nonblank input remains invalid.
                values.insert(
                    criterion.code.clone(),
                    if value.trim().is_empty() {
                        Value::Null
                    } else {
                        value
                            .parse::<i64>()
                            .map(Value::from)
                            .unwrap_or(Value::String(value))
                    },
                );
            }
            let reason = posted(payload, &format!("{root}[reason]"));
            self.pending = Some((selected, Value::Object(values), reason));
            self.selected = None;
        } else if event == self.event_name("confirm_evaluation", "confirm_answer") {
            let Some((task, values, reason)) = self.pending.take() else {
                return Ok(None);
            };
            match training
                .answer_evaluation(scope, company, task.review.id, &values, &reason)
                .await
            {
                Ok(_) => {
                    self.selected = None;
                    return Ok(Some((
                        StatusKind::Success,
                        self.event_name("Evaluation recorded.", "Review answer recorded.")
                            .into(),
                    )));
                }
                Err(e) => {
                    let message = e.to_string();
                    self.selected = Some(task);
                    self.summary = ErrorSummary::new("evaluation-answer-errors");
                    self.summary
                        .push("evaluation-answer-reason", message.clone());
                    return Ok(Some((StatusKind::Error, message)));
                }
            }
        }
        Ok(None)
    }
    pub fn render(&self, flash: FlashMap<'_>) -> DbResult<Html> {
        if !self.enabled {
            return Ok(Html::EMPTY);
        }
        let mut body = Html::EMPTY;
        if let Some(error) = &self.error {
            body.push(&render(&Alert {
                kind: StatusKind::Error,
                body: Html::text(error),
                ..Alert::default()
            })?);
        } else {
            // load() clamps the ListState page after refreshing its current audience.
            let counts =
                PageCounts::new(self.list.page, self.list.page_size, self.tasks.len() as u64);
            let rows = self
                .tasks
                .iter()
                .skip(self.list.offset() as usize)
                .take(self.list.page_size as usize)
                .map(|task| {
                    let mut answer = Html::EMPTY;
                    if let Some(a) = &task.answer {
                        let lines = task
                            .criteria
                            .iter()
                            .map(|c| {
                                format!(
                                    "{}: {}",
                                    c.label,
                                    a.values
                                        .get(&c.code)
                                        .filter(|v| !v.is_null())
                                        .map(|v| v.to_string())
                                        .unwrap_or_else(|| "Unknown".into())
                                )
                            })
                            .collect::<Vec<_>>();
                        let lines = if self.columns.mode() == RowMode::Compact {
                            vec![]
                        } else {
                            lines
                        };
                        answer = render(&Lines {
                            heading: &a.reason,
                            lines,
                        })?;
                    } else {
                        answer.push(&button(
                            self.event_name("open_evaluation", "open_answer"),
                            "Answer",
                            &task.review.id.to_string(),
                        )?);
                    }
                    let mut row = FlexRow::new(task.review.id.to_string())
                        .cell("employee", Cell::text(task.employee_id.to_string()))
                        .cell("session", Cell::text(task.session_id.to_string()))
                        .cell("due", Cell::text(&task.review.due_on))
                        .cell("version", Cell::text(task.policy_version.to_string()))
                        .cell(
                            "status",
                            Cell::text(if task.answer.is_some() {
                                "Answered"
                            } else if task.reminder.is_some() {
                                "Reminder available"
                            } else {
                                "Awaiting answer"
                            }),
                        )
                        .slot("answer", answer);
                    row.dom_id = Some(format!(
                        "{}-{}",
                        if self.own() { "evaluation" } else { "review" },
                        task.review.id
                    ));
                    Ok(row)
                })
                .collect::<DbResult<Vec<_>>>()?;
            body.push(&table(if self.own(){"my-evaluation-tasks"}else{"effectiveness-reviews"},&self.columns,rows,self.event_name("evaluation_grid","review_grid"),if self.own(){"No evaluations yet"}else{"No reviews yet"},if self.own(){"An evaluation appears here after an operator prepares reviews for a confirmed session you attended."}else{"An operator prepares reviews after a confirmed session ends."})?);
            let mut pagination = Pagination::new(
                if self.own() {
                    "my-evaluation-pagination"
                } else {
                    "effectiveness-pagination"
                },
                counts,
                &[25, 50, 100, 300],
            );
            pagination.page_event = self.event_name("evaluation_page", "page");
            pagination.filters_event = self.event_name("evaluation_filters", "review_filters");
            body.push(&render(&pagination)?);
        }
        let content = render(&Section {
            id: if self.own() {
                "my-evaluations"
            } else {
                "effectiveness-review-section"
            },
            title: if self.own() {
                "My evaluations"
            } else {
                "Review"
            },
            description: if self.own() {
                "Evaluate your own learning after a confirmed session. Leave a score blank when the outcome is unknown."
            } else {
                "Heads of department answer effectiveness reviews for their current direct reports."
            },
            body,
        })?;
        let mut result = content;
        if let Some(task) = &self.selected {
            let root = if self.own() { "evaluation" } else { "entry" };
            let mut fields = task
                .criteria
                .iter()
                .map(|c| {
                    let name = format!("{root}[values][{}]", c.code);
                    Field::named(&name)
                        .id(format!("evaluation-answer-{}", c.code))
                        .label(&c.label)
                        .kind(InputKind::Number)
                        .attr("min", &c.minimum.to_string())
                        .attr("max", &c.maximum.to_string())
                        .value(self.draft.get(&name).cloned().unwrap_or_default())
                })
                .collect::<Vec<_>>();
            fields.push(
                Field::in_form(root, "reason")
                    .id("evaluation-answer-reason")
                    .label("Evidence and explanation")
                    .kind(InputKind::Textarea)
                    .required()
                    .attr("maxlength", "4000")
                    .value(
                        self.draft
                            .get(&format!("{root}[reason]"))
                            .cloned()
                            .unwrap_or_default(),
                    ),
            );
            let form = form(
                if self.own() {
                    "my-evaluation-form"
                } else {
                    "evaluation-answer-form"
                },
                self.event_name("save_evaluation", "save_answer"),
                fields,
                submit("Review answer")?,
            )?;
            result.push(&render(&Modal {
                id: if self.own() {
                    "my-evaluation-modal"
                } else {
                    "evaluation-answer-modal"
                },
                title: if self.own() {
                    "Answer evaluation"
                } else {
                    "Answer review"
                },
                cancel_event: self.event_name("close_evaluation", "close_answer"),
                flash: Some(flash),
                body: render(&AnswerBody {
                    summary: &self.summary,
                    form,
                })?,
                ..Modal::default()
            })?);
        }
        if self.pending.is_some() {
            result.push(&render(&ConfirmDialog {
                id: if self.own() {
                    "my-evaluation-confirm"
                } else {
                    "evaluation-answer-confirm"
                },
                consequence: if self.own() {
                    "This evaluation will be recorded permanently."
                } else {
                    "This review answer will be recorded permanently."
                },
                detail:
                    "The criteria version and explanation are preserved. This cannot be undone.",
                confirm: "Submit",
                working: "Submitting…",
                confirm_event: self.event_name("confirm_evaluation", "confirm_answer"),
                cancel_event: self.event_name("cancel_evaluation", "cancel_answer"),
            })?);
        }
        Ok(result)
    }
}
#[derive(Template)]
#[template(path = "evaluation_answer.html")]
struct AnswerBody<'a> {
    summary: &'a ErrorSummary,
    form: Html,
}

#[derive(Template)]
#[template(path = "evaluation_lines.html")]
struct Lines<'a> {
    heading: &'a str,
    lines: Vec<String>,
}
