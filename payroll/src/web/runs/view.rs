//! Shared UI presentation for Bilimbi Payroll.Web.RunsLive.
use super::super::setup::{card, heading};
use super::*;
use askama::Template;
use bilibi_ui::{ActionLink, ConfirmDialog, Fact, Facts};
#[derive(Template)]
#[template(path = "runs.html")]
struct View<'a> {
    flash: FlashMap<'a>,
    has_companies: bool,
    available: bool,
    selected: bool,
    read_only: bool,
    unlocked: bool,
    calculated: bool,
    undecided: bool,
    approved: bool,
    company: Field,
    chooser: Html,
    runs: Html,
    facts: Html,
    contributions: Html,
    intake: Html,
    actions: Html,
    results: Html,
    totals: Html,
    digest: String,
    decision: Html,
    decision_form: Html,
    report: Html,
    documents: Html,
    confirm: Html,
    setup_url: String,
}
impl RunState {
    pub(super) fn render(&self) -> Result<String> {
        let mut view = View {
            flash: self.flash.map(),
            has_companies: !self.companies.is_empty(),
            available: self.setup.is_some(),
            selected: self.output.is_some(),
            read_only: !self.can_manage,
            unlocked: false,
            calculated: false,
            undecided: false,
            approved: false,
            company: Field::named("company_id")
                .id("runs-company-id")
                .label("Company")
                .kind(InputKind::Select)
                .wrapper_class("mb-0")
                .value(
                    self.company_id
                        .map_or_else(String::new, |id| id.to_string()),
                )
                .options(
                    self.companies
                        .iter()
                        .map(|c| Choice::new(&c.name, c.id.to_string()))
                        .collect(),
                ),
            chooser: Html::EMPTY,
            runs: Html::EMPTY,
            facts: Html::EMPTY,
            contributions: Html::EMPTY,
            intake: Html::EMPTY,
            actions: Html::EMPTY,
            results: Html::EMPTY,
            totals: Html::EMPTY,
            digest: String::new(),
            decision: Html::EMPTY,
            decision_form: Html::EMPTY,
            report: Html::EMPTY,
            documents: Html::EMPTY,
            confirm: self.confirmation()?,
            setup_url: format!(
                "/people/payroll/setup?company_id={}",
                self.company_id.unwrap_or_default()
            ),
        };
        if let Some(setup) = &self.setup {
            let company = self.company_id.unwrap_or_default();
            let mut select = Field::named("run_id")
                .id("runs-run-id")
                .label("Run")
                .kind(InputKind::Select)
                .wrapper_class("mb-0")
                .value(self.run_id.map_or_else(String::new, |id| id.to_string()))
                .options(
                    setup
                        .runs
                        .iter()
                        .map(|r| {
                            Choice::new(
                                format!("Run {} · period {} · {}", r.id, r.period_id, r.currency),
                                r.id.to_string(),
                            )
                        })
                        .collect(),
                );
            select.prompt = "Choose a run".into();
            view.chooser = render(&Form {
                id: "runs-chooser",
                event: "select_run",
                body: render(&select)?,
                actions: Html::EMPTY,
                change: true,
                class: "mt-3",
            })?;
            view.runs = self.tables["runs"].render(
                "Runs",
                setup
                    .runs
                    .iter()
                    .map(|r| {
                        Ok(row("payroll-run", r.id)
                            .slot(
                                "run",
                                link(
                                    &format!(
                                        "/people/payroll/runs?company_id={company}&run_id={}",
                                        r.id
                                    ),
                                    &format!("Run {}", r.id),
                                )?,
                            )
                            .slot("period", Html::text(&text(&r.snapshot["period"], "code")))
                            .slot("currency", Html::text(&r.currency))
                            .slot(
                                "locked",
                                Html::text(if r.locked_at.is_some() { "Yes" } else { "No" }),
                            ))
                    })
                    .collect::<Result<_>>()?,
                "No frozen payroll runs yet.",
                "payroll-no-runs",
            )?;
        }
        if let Some(output) = &self.output {
            let run = &output.run;
            let period = &run.snapshot["period"];
            view.unlocked = run.locked_at.is_none();
            view.calculated = output.calculation.is_some();
            view.undecided = output.calculation.is_some() && output.decision.is_none();
            view.approved = output
                .decision
                .as_ref()
                .is_some_and(|d| d.outcome == "approved");
            view.facts = render(&Facts {
                id: "payroll-run-facts",
                items: vec![
                    fact("Run", run.id.to_string()),
                    fact("Period", text(period, "code")),
                    fact("From", text(period, "starts_on")),
                    fact("To", text(period, "ends_on")),
                    fact("Pay on", text(period, "pay_on")),
                    fact("Country", run.country.clone()),
                    fact("Currency", run.currency.clone()),
                    fact("Locked", if view.unlocked { "No" } else { "Yes" }.into()),
                ],
            })?;
            view.contributions = self.tables["contributions"].render(
                "Contributions",
                output
                    .contributions
                    .iter()
                    .map(|c| {
                        row("payroll-contribution", c.id)
                            .slot("key", Html::text(&c.source_key))
                            .slot("employee", Html::text(&c.employee_id.to_string()))
                            .slot("item", Html::text(&item_code(run, c.item_id)))
                            .slot("date", Html::text(&c.on_date))
                            .slot("direction", Html::text(&c.direction))
                            .slot("units", Html::text(&c.units))
                            .slot("evidence", Html::text(&c.evidence))
                    })
                    .collect(),
                "No contributions accepted.",
                "no-contributions",
            )?;
            if self.can_manage && !view.calculated {
                view.intake = self.intake_forms(output)?;
                view.actions = action(
                    if view.unlocked {
                        "Lock setup"
                    } else {
                        "Calculate permanently"
                    },
                    if view.unlocked {
                        "prepare_lock"
                    } else {
                        "calculate"
                    },
                    "",
                    "",
                )?;
            }
            if let Some(calculation) = &output.calculation {
                view.digest = calculation.digest.clone();
                view.results = self.tables["results"].render(
                    "Frozen results",
                    array(&calculation.snapshot["result"], "lines")
                        .iter()
                        .enumerate()
                        .map(|(i, line)| {
                            let mut r = row("payroll-result-line", i as i64);
                            for (column, key) in [
                                ("employee", "employee_id"),
                                ("item", "item_code"),
                                ("direction", "direction"),
                                ("units", "units"),
                                ("rate", "rate"),
                                ("amount", "amount"),
                            ] {
                                r = r.slot(column, Html::text(&scalar(line, key)));
                            }
                            r
                        })
                        .collect(),
                    "No result lines.",
                    "payroll-no-results",
                )?;
                view.totals = self.tables["totals"].render(
                    "Employee totals",
                    array(&calculation.snapshot["result"], "totals")
                        .iter()
                        .map(|total| {
                            let employee = total["employee_id"].as_i64().unwrap_or_default();
                            let mut r = row("payroll-total", employee);
                            for (column, key) in [
                                ("employee", "employee_id"),
                                ("earning", "earning"),
                                ("deduction", "deduction"),
                                ("net", "net"),
                                ("employer", "employer"),
                            ] {
                                r = r.slot(column, Html::text(&scalar(total, key)));
                            }
                            if view.approved && self.can_manage {
                                r = r.slot(
                                    "actions",
                                    action(
                                        &format!("Payslip for employee {employee}"),
                                        "generate_payslip",
                                        "employee",
                                        &employee.to_string(),
                                    )?,
                                );
                            }
                            Ok(r)
                        })
                        .collect::<Result<_>>()?,
                    "No employee totals.",
                    "payroll-no-totals",
                )?;
                if self.can_approve && view.undecided {
                    view.decision_form = self.form(
                        "decision-form",
                        "decide",
                        vec![
                            self.select(
                                "decide",
                                "decision",
                                "outcome",
                                "Decision",
                                vec![
                                    Choice::new("Approve", "approved"),
                                    Choice::new("Reject", "rejected"),
                                ],
                            ),
                            self.field("decide", "decision", "reason", "Decision reason")
                                .required(),
                        ],
                        "Record final decision",
                    )?;
                }
                if let Some(decision) = &output.decision {
                    view.decision = render(&Facts {
                        id: "payroll-decision",
                        items: vec![fact(
                            "Decision",
                            format!(
                                "{} by actor {}: {}",
                                decision.outcome, decision.created_by_actor_id, decision.reason
                            ),
                        )],
                    })?;
                }
                if self.can_manage && view.approved {
                    view.report = action("Generate report PDF", "generate_report", "", "")?;
                }
            }
            view.documents =
                self.tables["documents"].render(
                    "Documents",
                    output
                        .documents
                        .iter()
                        .map(|d| {
                            Ok(row("payroll-document", d.id)
                                .slot("kind", Html::text(&d.kind))
                                .slot(
                                    "employee",
                                    Html::text(&d.employee_id.map_or_else(
                                        || "all employees".into(),
                                        |id| id.to_string(),
                                    )),
                                )
                                .slot(
                                    "download",
                                    link(
                                        &format!(
                                            "/people/payroll/documents/{}?company_id={}",
                                            d.artifact_id,
                                            self.company_id.unwrap_or_default()
                                        ),
                                        "Download PDF",
                                    )?,
                                ))
                        })
                        .collect::<Result<_>>()?,
                    "No documents generated.",
                    "payroll-no-documents",
                )?;
        }
        view.render().map_err(|_| Error::Internal)
    }
    fn intake_forms(&self, output: &RunOutput) -> Result<Html> {
        let mut body = Html::EMPTY;
        for (event, id, title) in [
            ("intake", "contribution-form", "Direct pay item"),
            (
                "intake_mapped",
                "mapped-contribution-form",
                "Leave and Claims mappings",
            ),
            (
                "intake_attendance",
                "attendance-contribution-form",
                "Attendance allowance",
            ),
        ] {
            let mut section = heading("", title, "")?;
            let mut fields = vec![
                self.field(event, "input", "source_key", "Unique contribution key")
                    .required(),
                self.field(event, "input", "employee_id", "Employee ID")
                    .kind(InputKind::Number)
                    .required(),
            ];
            if event == "intake" {
                fields.push(
                    self.select(
                        event,
                        "input",
                        "item_id",
                        "Direct pay item",
                        array(&output.run.snapshot, "items")
                            .iter()
                            .map(|item| Choice::new(text(item, "code"), scalar(item, "id")))
                            .collect(),
                    ),
                );
            } else {
                if event == "intake_mapped" {
                    fields.push(self.select(
                        event,
                        "input",
                        "source_kind",
                        "Source",
                        vec![
                            Choice::new("Leave mapping", "leave"),
                            Choice::new("Claims mapping", "claims"),
                        ],
                    ));
                }
                fields.push(
                    self.field(
                        event,
                        "input",
                        "mapping_source",
                        if event == "intake_attendance" {
                            "Attendance rule code"
                        } else {
                            "Mapped source code or ID"
                        },
                    )
                    .required(),
                );
            }
            fields.extend([
                self.field(event, "input", "on_date", "Contribution date")
                    .kind(InputKind::Date)
                    .required(),
                self.field(event, "input", "units", "Exact units")
                    .required(),
            ]);
            if event != "intake_attendance" {
                fields.push(self.select(
                    event,
                    "input",
                    "direction",
                    "Direction",
                    vec![
                        Choice::new("Earning", "earning"),
                        Choice::new("Deduction", "deduction"),
                        Choice::new("Employer contribution", "employer"),
                    ],
                ));
            }
            fields.push(
                self.field(event, "input", "evidence", "Source evidence reference")
                    .required(),
            );
            section.push(&self.form(id, event, fields, "Accept contribution")?);
            body.push(&card(&format!("{id}-section"), section)?);
        }
        Ok(body)
    }
    fn confirmation(&self) -> Result<Html> {
        let Some(pending) = &self.pending else {
            return Ok(Html::EMPTY);
        };
        let (consequence, detail, confirm, id) = match &pending.action {
            Action::Lock => (format!("Run {} will be locked permanently.", pending.run), "Its frozen setup remains available. The run cannot be changed or deleted after locking.".into(), "Lock permanently", "payroll-lock-confirm"),
            Action::Calculate => (format!("Calculate run {} permanently?", pending.run), "Contributions and result lines cannot be changed after calculation.".into(), "Calculate permanently", "payroll-calculate-confirm"),
            Action::Decide { outcome, reason } => (format!("{} run {} permanently?", if outcome == "approved" { "Approve" } else { "Reject" }, pending.run), format!("Decision reason: {reason}"), if outcome == "approved" { "Approve" } else { "Reject" }, "payroll-decision-confirm"),
        };
        render(&ConfirmDialog {
            id,
            consequence: &consequence,
            detail: &detail,
            confirm,
            working: "Working…",
            confirm_event: "confirm_action",
            cancel_event: "cancel_action",
        })
    }
}
fn fact(title: &'static str, value: String) -> Fact<'static> {
    Fact {
        title,
        id: "",
        value: Html::text(&value),
    }
}
fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value[key].as_array().map(Vec::as_slice).unwrap_or_default()
}
fn scalar(value: &Value, key: &str) -> String {
    match &value[key] {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}
fn item_code(run: &crate::Run, id: i64) -> String {
    array(&run.snapshot, "items")
        .iter()
        .find(|i| i["id"].as_i64() == Some(id))
        .map(|i| text(i, "code"))
        .unwrap_or_else(|| id.to_string())
}
fn action(label: &str, event: &str, name: &str, value: &str) -> Result<Html> {
    render(&Button {
        event,
        name,
        value,
        body: Html::text(label),
        ..Button::default()
    })
}
fn link(href: &str, label: &str) -> Result<Html> {
    render(&ActionLink {
        href,
        body: Html::text(label),
        ..ActionLink::default()
    })
}
