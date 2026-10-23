//! Bilimbi Skills.Web.MySkillsLive; self linkage is resolved on every update.
use super::*;
use crate::{Reminder, Standing, SELF_VIEW};
use bilibi_ui::page::{LiveBody, LiveFuture, LiveInput, LiveRequest};

pub(super) struct MySkills {
    core: Arc<Core>,
    company: Company,
    standing: Option<Standing>,
    inbox: Vec<Reminder>,
    scores: Table,
    actions: Table,
    requests: Table,
    reminders: Table,
    flash: Flash,
}
impl MySkills {
    pub(super) fn new(core: Arc<Core>, query: &HashMap<String, String>) -> Result<Self> {
        Ok(Self {
            core,
            company: Company::new(query),
            standing: None,
            inbox: vec![],
            flash: Flash::default(),
            scores: Table::new(
                "my-skills",
                "scores_grid",
                &[
                    ("skill", "Skill"),
                    ("level", "Level"),
                    ("result", "Result"),
                    ("assessed", "Assessed"),
                    ("due", "Next due"),
                    ("state", "State"),
                ],
            )?,
            actions: Table::new(
                "my-actions",
                "actions_grid",
                &[
                    ("skill", "Skill"),
                    ("levels", "Levels"),
                    ("status", "Status"),
                    ("due", "Due"),
                    ("objective", "Objective"),
                ],
            )?,
            requests: Table::new(
                "my-reassessments",
                "requests_grid",
                &[("skill", "Skill"), ("due", "Reassessed by")],
            )?,
            reminders: Table::new(
                "my-reminders",
                "reminders_grid",
                &[("rule", "Reminder"), ("due", "Due")],
            )?,
        })
    }
    pub(super) async fn load(&mut self, scope: &Scope) -> Result<()> {
        self.company.load(&self.core, scope, SELF_VIEW).await?;
        self.standing = None;
        self.inbox.clear();
        if let Some(c) = &self.company.selected {
            if let Ok(s) = self.core.skills.standing(scope, c.id).await {
                self.inbox = self
                    .core
                    .skills
                    .reminder_inbox(scope, c.id, 100)
                    .await
                    .unwrap_or_default();
                self.standing = Some(s);
            }
        }
        let scores = self
            .standing
            .as_ref()
            .map(|s| s.scores.as_slice())
            .unwrap_or_default();
        let actions = self
            .standing
            .as_ref()
            .map(|s| s.actions.as_slice())
            .unwrap_or_default();
        let requests = self
            .standing
            .as_ref()
            .map(|s| s.reassessments.as_slice())
            .unwrap_or_default();
        self.scores.load(scores, |s| s.score.id.to_string());
        self.actions.load(actions, |a| a.id.to_string());
        self.requests.load(requests, |r| r.id.to_string());
        self.reminders.load(&self.inbox, |r| r.id.to_string());
        Ok(())
    }
    pub(super) fn render(&self) -> Result<String> {
        use askama::Template;
        let mut body = Html::EMPTY;
        if self.company.selected.is_none() {
            body.push(&empty(
                "my-skills-no-company",
                "No active company is available for skills.",
            )?);
        } else if let Some(standing) = &self.standing {
            let rows = standing
                .scores
                .iter()
                .map(|v| {
                    let s = &v.score;
                    row("my-skill", s.skill_id)
                        .slot(
                            "skill",
                            Html::text(v.skill_name.as_deref().unwrap_or_default()),
                        )
                        .slot(
                            "level",
                            Html::text(&format!("{} / {}", s.current_level, s.required_level)),
                        )
                        .slot("result", Html::text(&label(&v.result_band)))
                        .slot("assessed", Html::text(&shown(s.assessed_on)))
                        .slot("due", Html::text(&shown(s.next_due_on)))
                        .slot("state", Html::text(&label(&v.state)))
                })
                .collect();
            body.push(&section(
                "my-skills-section",
                "Assessed skills",
                self.scores.render(
                    "Assessed skills",
                    rows,
                    "No assessment of yours has been finalized yet.",
                )?,
            )?);
            let rows = standing
                .actions
                .iter()
                .map(|a| {
                    row("my-action", a.id)
                        .slot(
                            "skill",
                            Html::text(a.skill_name.as_deref().unwrap_or_default()),
                        )
                        .slot(
                            "levels",
                            Html::text(&format!("{} to {}", a.starting_level, a.target_level)),
                        )
                        .slot("status", Html::text(&label(&a.status)))
                        .slot("due", Html::text(&shown(a.due_on)))
                        .slot("objective", Html::text(&a.objective))
                })
                .collect();
            body.push(&section(
                "my-actions-section",
                "Development actions",
                self.actions.render(
                    "Development actions",
                    rows,
                    "No development action has been approved for you.",
                )?,
            )?);
            let rows = standing
                .reassessments
                .iter()
                .map(|r| {
                    row("my-reassessment", r.id)
                        .slot(
                            "skill",
                            Html::text(r.skill_name.as_deref().unwrap_or_default()),
                        )
                        .slot("due", Html::text(&shown(r.due_on)))
                })
                .collect();
            let mut reminders = if !standing.reassessments.is_empty() || self.inbox.is_empty() {
                self.requests
                    .render("Reassessments", rows, "Nothing is waiting for you.")?
            } else {
                Html::EMPTY
            };
            let rows = self
                .inbox
                .iter()
                .map(|r| {
                    row("my-reminder", r.id)
                        .slot("rule", Html::text(&label(&r.rule)))
                        .slot("due", Html::text(&r.due_on.to_string()))
                })
                .collect();
            if !self.inbox.is_empty() {
                reminders.push(&self.reminders.render("Reminders", rows, "")?);
            }
            body.push(&section(
                "my-reminders-section",
                "Reassessments and reminders",
                reminders,
            )?);
        } else {
            body.push(&empty(
                "my-skills-unavailable",
                "Your skills are unavailable: your account needs a link to a current employee.",
            )?);
        }
        View {
            flash: self.flash.map(),
            page_id: "my-skills-page",
            title: "My skills",
            subtitle: self
                .company
                .selected
                .as_ref()
                .map(|c| {
                    format!(
                        "{} · Your assessed levels against your requirements",
                        c.name
                    )
                })
                .unwrap_or_default(),
            has_company: !self.company.companies.is_empty(),
            company_form: "my-skills-company-form",
            company: self.company.field("my-skills"),
            body,
            confirm: Html::EMPTY,
        }
        .render()
        .map_err(|_| Error::Internal)
    }
}
fn shown<T: std::fmt::Display>(v: Option<T>) -> String {
    v.map(|v| v.to_string()).unwrap_or_default()
}
impl LiveState<Scope, Error> for MySkills {
    fn update<'a>(&'a mut self, r: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            match r.input {
                LiveInput::Params(q) => {
                    self.company.id = params::positive_id(q.get("company_id").map(String::as_str))
                }
                LiveInput::Event(p) => match text(p, "type") {
                    "select_company" => self.company.id = id(p, "company_id"),
                    "clear-flash" => self.flash.clear(p),
                    "scores_grid" => self.scores.event(p),
                    "actions_grid" => self.actions.event(p),
                    "requests_grid" => self.requests.event(p),
                    "reminders_grid" => self.reminders.event(p),
                    _ => (),
                },
                LiveInput::Refresh => (),
            }
            self.load(r.scope).await?;
            Ok(LiveBody {
                html: self.render()?,
                query: self.company.query(),
                rows: self.standing.as_ref().map_or(0, |s| s.scores.len()),
                ..LiveBody::default()
            })
        })
    }
}
