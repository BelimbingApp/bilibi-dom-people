//! Bilimbi Progression page fragments, using shared UI components.
use super::*;
use askama::Template;

impl State {
    fn table(
        &self,
        id: &str,
        rows: Vec<FlexRow>,
        empty: EmptyState,
        actions: bool,
    ) -> Result<Html> {
        let columns = self.tables.get(id).ok_or(Error::Internal)?;
        let event = format!("grid-{id}");
        let mut table = FlexTable::new(id, columns.flex_columns(), rows);
        table.event = &event;
        table.zoom = columns.zoom;
        table.suggestions = columns.flex_suggestions();
        table.add_query = &columns.add_query;
        table.actions = actions;
        table.empty = Some(TableEmpty::State(empty));
        render(&table)
    }
    fn policies(&self) -> Result<Html> {
        let rows = self
            .rows
            .iter()
            .map(|p| {
                let mut actions = action("select", "Read", p.id)?;
                if self.can_manage && p.status == "draft" {
                    actions.push(&action("request_publish", "Publish", p.id)?);
                }
                let mut row = FlexRow::new(format!("policy-{}", p.id))
                    .slot("policy", Html::text(&format!("{} · {}", p.name, p.code)))
                    .slot("version", Html::text(&p.version.to_string()))
                    .slot("effective", Html::text(&p.effective_from))
                    .slot("status", Html::text(&p.status))
                    .slot(
                        "criteria",
                        Html::text(&format!(
                            "{} skill requirements{}",
                            p.rules.competence.len(),
                            if p.rules.performance.is_some() {
                                "; performance required"
                            } else {
                                ""
                            }
                        )),
                    );
                row.actions = actions;
                Ok(row)
            })
            .collect::<Result<Vec<_>>>()?;
        let mut body = self.table(
            "progression-policies",
            rows,
            EmptyState::missing(
                "No progression policies",
                "An authorized policy operator must draft and publish company criteria.",
            )
            .id("progression-empty"),
            true,
        )?;
        let mut pagination = Pagination::new(
            "progression-pagination",
            PageCounts::new(self.list.page, self.list.page_size, self.total),
            &[10, 25, 50, 100],
        );
        pagination.page_event = "paginate";
        pagination.filters_event = "filter";
        body.push(&render(&pagination)?);
        if let Some(p) = &self.selected {
            body.push(&self.detail(p)?);
        }
        Ok(body)
    }
    fn detail(&self, policy: &Policy) -> Result<Html> {
        let publication = match policy.published_at.as_deref().and_then(|s| s.parse().ok()) {
            Some(instant) => {
                let r = self.clock.render(
                    instant,
                    bilibi_datetime::Format::DateTime,
                    bilibi_datetime::Precision::Minute,
                );
                render(&Publication {
                    value: bilibi_ui::Instant::new(
                        &r.iso,
                        &r.text,
                        &r.company,
                        &r.utc,
                        r.mode.as_str(),
                    ),
                })?
            }
            None => Html::text("Draft"),
        };
        let mut body = facts(vec![
            (
                "Version",
                Html::text(&format!("{} · {}", policy.code, policy.version)),
            ),
            ("Effective from", Html::text(&policy.effective_from)),
            ("Publication", publication),
        ])?;
        let rows = policy
            .rules
            .competence
            .iter()
            .enumerate()
            .map(|(i, r)| {
                FlexRow::new(i.to_string())
                    .slot("skill", Html::text(&r.code))
                    .slot("level", Html::text(&r.required_level.to_string()))
                    .slot(
                        "profile",
                        Html::text(&format!("{} · {}", r.profile_id, r.profile_version)),
                    )
            })
            .collect();
        body.push(&self.table(
            &format!("policy-criteria-{}", policy.id),
            rows,
            EmptyState::missing(
                "No skill criteria",
                "This policy declares performance criteria only.",
            ),
            false,
        )?);
        if let Some(rule) = &policy.rules.performance {
            body.push(&facts(vec![
                (
                    "Performance periods",
                    Html::text(
                        &rule
                            .periods
                            .iter()
                            .map(|p| format!("{} – {}", p.start, p.end))
                            .collect::<Vec<_>>()
                            .join("; "),
                    ),
                ),
                (
                    "Accepted outcomes",
                    Html::text(&rule.accepted_outcomes.join(", ")),
                ),
                (
                    "Missing released evidence",
                    Html::text(match rule.missing_evidence {
                        MissingEvidence::Unknown => "unknown",
                        MissingEvidence::NotMet => "not_met",
                    }),
                ),
            ])?);
        }
        card("progression-detail", &policy.name, "", body)
    }
    fn draft_form(&self) -> Result<Html> {
        let field = |key: &str, label: &str, kind: InputKind| {
            Field::in_form("policy", key)
                .label(label)
                .kind(kind)
                .value(self.value(key))
        };
        let fields = vec![
            field("code", "Policy code", InputKind::Text).required(),
            field("name", "Policy name", InputKind::Text).required(),
            field("version", "Version", InputKind::Number)
                .required()
                .attr("min", "1"),
            field("effective_from", "Effective from", InputKind::Date).required(),
            field(
                "profile_id",
                "Published competency profile",
                InputKind::Select,
            )
            .prompt("No competency criteria")
            .options(
                self.profiles
                    .iter()
                    .map(|p| {
                        Choice::new(
                            format!("{} · version {}", p.name, p.version),
                            p.id.to_string(),
                        )
                    })
                    .collect(),
            ),
            field(
                "performance",
                "Require released performance evidence",
                InputKind::Checkbox,
            )
            .checked(self.value("performance") == "true"),
            field("period_start", "Performance period start", InputKind::Date),
            field("period_end", "Performance period end", InputKind::Date),
            field(
                "outcomes",
                "Accepted performance outcomes (separated by commas)",
                InputKind::Text,
            ),
            field(
                "missing_evidence",
                "Missing performance evidence",
                InputKind::Select,
            )
            .options(vec![
                Choice::new("Unknown", "unknown"),
                Choice::new("Not met", "not_met"),
            ]),
        ];
        let mut body = Html::EMPTY;
        if let Some(message) = &self.draft_error {
            let mut summary = ErrorSummary::new("progression-draft-errors");
            summary.push("policy_code", message);
            body.push(&render(&summary)?);
        }
        for field in fields {
            body.push(&render(&field)?);
        }
        let form = render(&Form {
            id: "progression-draft",
            event: "draft",
            body,
            actions: render(&Button {
                button_type: "submit",
                working: "Recording…",
                body: Html::text("Record draft"),
                ..Button::default()
            })?,
            ..Form::default()
        })?;
        render(&bilibi_ui::Page {
            id: "progression-draft-page",
            variant: PageVariant::Form,
            body: card(
                "progression-draft-section",
                "Draft a policy version",
                "",
                form,
            )?,
            ..bilibi_ui::Page::default()
        })
    }
    fn my_standing(&self) -> Result<Html> {
        let mut body = Html::EMPTY;
        if self.can_performance {
            body.push(&card(
                "my-standing-performance",
                "My performance",
                "Communicated targets, released reviews and your responses.",
                render(&ActionLink {
                    href: "/people/performance/my",
                    body: Html::text("Open my performance"),
                    ..ActionLink::default()
                })?,
            )?);
        }
        if self.can_progression {
            if let Some(reason) = &self.unavailable {
                body.push(&render(
                    &EmptyState::missing("Your progression explanation is unavailable", reason)
                        .id("my-progression-unavailable"),
                )?);
            }
            if let Some(standing) = &self.standing {
                for e in &standing.explanations {
                    let mut content = facts(vec![
                        (
                            "Policy version",
                            Html::text(&format!("{} · {}", e.policy.code, e.policy.version)),
                        ),
                        ("Effective from", Html::text(&e.policy.effective_from)),
                        ("Eligibility criteria", Html::text(label(e.status))),
                    ])?;
                    let rows = e
                        .rules
                        .iter()
                        .enumerate()
                        .map(|(i, r)| {
                            let (code, evidence) = match r {
                                Criterion::Skill {
                                    code,
                                    required_level,
                                    observed_level,
                                    ..
                                } => (
                                    code,
                                    format!(
                                        "Required level {required_level} · {}",
                                        observed_level.map_or_else(
                                            || "current assessment unavailable".into(),
                                            |level| format!("observed {level}")
                                        )
                                    ),
                                ),
                                Criterion::Performance {
                                    code,
                                    review_id: Some(id),
                                    review_version,
                                    period_start,
                                    period_end,
                                    supersedes_id,
                                    ..
                                } => (
                                    code,
                                    format!(
                                        "Released review {id} · version {} · {} – {}{}",
                                        review_version.unwrap_or_default(),
                                        period_start.as_deref().unwrap_or(""),
                                        period_end.as_deref().unwrap_or(""),
                                        if supersedes_id.is_some() {
                                            " · corrected evidence requires reevaluation"
                                        } else {
                                            ""
                                        }
                                    ),
                                ),
                                Criterion::Performance { code, .. } => (
                                    code,
                                    "No released review in the policy's declared period.".into(),
                                ),
                            };
                            FlexRow::new(i.to_string())
                                .slot("criterion", Html::text(code))
                                .slot("result", Html::text(label(r.status())))
                                .slot("evidence", Html::text(&evidence))
                        })
                        .collect();
                    content.push(&self.table(
                        &format!("progression-explanation-{}", e.policy.id),
                        rows,
                        EmptyState::missing("No criteria", ""),
                        false,
                    )?);
                    body.push(&card(
                        &format!("my-progression-{}", e.policy.id),
                        &e.policy.name,
                        "",
                        content,
                    )?);
                }
            }
        }
        Ok(body)
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
    pub(super) fn render(&self) -> Result<String> {
        let mut body = if self.my {
            self.my_standing()?
        } else {
            let mut body = if let Some(reason) = &self.unavailable {
                render(
                    &EmptyState::missing("Policies unavailable", reason)
                        .id("progression-unavailable"),
                )?
            } else {
                self.policies()?
            };
            if !self.can_manage {
                body.push(&render(
                    &EmptyState::missing(
                        "Policy history is read-only",
                        "A policy operator with manage permission can create and publish versions.",
                    )
                    .id("progression-read-only"),
                )?);
            } else if self.unavailable.is_none() {
                body.push(&self.draft_form()?);
            }
            body
        };
        if self.my && !self.can_progression && !self.can_performance {
            body.push(&render(&EmptyState::missing(
                "Your standing is unavailable",
                self.unavailable.as_deref().unwrap_or("You cannot do that for this company. Check your progression and evidence permissions."),
            ))?);
        }
        let confirmation = if let Some(policy) = &self.pending {
            render(&ConfirmDialog {
                id: "progression-confirm", consequence: "Confirm publication",
                detail: &format!("Publish {} version {}, effective {}? This version and its criteria become permanent. The latest effective publication will apply to employees.", policy.name, policy.version, policy.effective_from),
                confirm: "Confirm publication", working: "Publishing…", confirm_event: "confirm_publish", cancel_event: "cancel_publish",
            })?
        } else {
            Html::EMPTY
        };
        View {
            id: if self.my { "my-standing" } else { "progression-page" },
            title: if self.my { "My standing" } else { "Progression policies" },
            subtitle: if self.my { "" } else { "Published criteria explain eligibility. They do not authorize a promotion or change pay." },
            variant: if self.my { PageVariant::Detail } else { PageVariant::List },
            flash: self.flash(), body, confirmation,
        }.render().map_err(|_| Error::Internal)
    }
}
#[derive(Template)]
#[template(path = "page.html")]
struct View<'a> {
    id: &'a str,
    title: &'a str,
    subtitle: &'a str,
    variant: PageVariant,
    flash: FlashMap<'a>,
    body: Html,
    confirmation: Html,
}
#[derive(Template)]
#[template(
    source = "{% import \"flex_table.html\" as table %}{% call table::instant(id=\"progression-publication-time\", i=value) %}{% endcall %}",
    ext = "html"
)]
struct Publication {
    value: bilibi_ui::Instant,
}

fn render(template: &impl Template) -> Result<Html> {
    Html::render(template).map_err(|_| Error::Internal)
}
fn action(event: &str, label: &str, id: i64) -> Result<Html> {
    render(&Button {
        button_type: "button",
        event,
        name: "id",
        value: &id.to_string(),
        body: Html::text(label),
        ..Button::default()
    })
}
fn card(id: &str, title: &str, description: &str, mut body: Html) -> Result<Html> {
    let mut content = render(&SectionHeading {
        title,
        description,
        ..SectionHeading::default()
    })?;
    content.push(&body);
    body = content;
    render(&Card {
        id,
        body,
        ..Card::default()
    })
}
fn facts(items: Vec<(&str, Html)>) -> Result<Html> {
    render(&Facts {
        id: "",
        items: items
            .into_iter()
            .map(|(title, value)| Fact {
                title,
                value,
                ..Fact::default()
            })
            .collect(),
    })
}

fn label(status: Status) -> &'static str {
    match status {
        Status::Met => "Met",
        Status::NotMet => "Not met",
        Status::Unknown => "Unknown",
    }
}
