//! Bilimbi Payroll.Web.SetupLive, including the mappings focus variant.
use super::*;
use crate::{NewAttendanceAllowanceMapping, NewClassification, NewItem, NewMapping, NewPeriod};
use askama::Template;
use bilibi_ui::{ActionLink, Card, ConfirmDialog, ErrorSummary, Fact, Facts, SectionHeading};
#[derive(Template)]
#[template(path = "setup.html")]
struct View<'a> {
    flash: FlashMap<'a>,
    available: bool,
    company: Field,
    withheld: bool,
    policy: Html,
    classifications: Html,
    items: Html,
    periods: Html,
    mappings: Html,
    runs: Html,
    confirm: Html,
    mappings_focus: bool,
}
impl State {
    pub(super) async fn write(
        &self,
        scope: &Scope,
        company: i64,
        event: &str,
    ) -> crate::Result<()> {
        let d = &self.drafts[event];
        let p = &self.core.payroll;
        match event {
            "save_settings" => {
                p.put_settings(
                    scope,
                    company,
                    d.get("country"),
                    &d.get("currencies")
                        .split([',', ' '])
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .collect::<Vec<_>>(),
                )
                .await
            }
            "create_classification" => p
                .create_classification(
                    scope,
                    company,
                    &NewClassification {
                        code: d.get("code").into(),
                        name: d.get("name").into(),
                        effective_from: d.date("effective_from")?,
                        effective_to: d.until()?,
                    },
                )
                .await
                .map(|_| ()),
            "create_item" => p
                .create_item(
                    scope,
                    company,
                    &NewItem {
                        code: d.get("code").into(),
                        name: d.get("name").into(),
                        classification_id: d
                            .id("classification_id")
                            .ok_or(PayrollError::InvalidItem)?,
                        currency: d.get("currency").into(),
                        amount: d
                            .get("amount")
                            .parse()
                            .map_err(|_| PayrollError::InvalidItem)?,
                        effective_from: d.date("effective_from")?,
                        effective_to: d.until()?,
                    },
                )
                .await
                .map(|_| ()),
            "create_period" => p
                .create_period(
                    scope,
                    company,
                    &NewPeriod {
                        code: d.get("code").into(),
                        starts_on: d.date("starts_on")?,
                        ends_on: d.date("ends_on")?,
                        pay_on: d.date("pay_on")?,
                    },
                )
                .await
                .map(|_| ()),
            "create_mapping" => {
                let (kind, key) = d
                    .get("source")
                    .split_once(':')
                    .ok_or(PayrollError::InvalidMapping)?;
                p.create_mapping(
                    scope,
                    company,
                    &NewMapping {
                        source_kind: kind.into(),
                        source_key: key.into(),
                        item_id: d.id("item_id").ok_or(PayrollError::InvalidMapping)?,
                        effective_from: d.date("effective_from")?,
                        effective_to: d.until()?,
                    },
                )
                .await
                .map(|_| ())
            }
            "create" => p
                .create_attendance_allowance_mapping(
                    scope,
                    company,
                    &NewAttendanceAllowanceMapping {
                        attendance_rule_code: d.get("attendance_rule_code").into(),
                        item_id: d
                            .id("item_id")
                            .ok_or(PayrollError::InvalidAttendanceMapping)?,
                        effective_from: d.date("effective_from")?,
                        effective_to: d.until()?,
                    },
                )
                .await
                .map(|_| ()),
            _ => Err(PayrollError::RunUnavailable),
        }
    }
    pub(super) fn render_setup(&self) -> Result<String> {
        let mut view = View {
            flash: self.flash.map(),
            available: self.setup.is_some(),
            company: self.company_field("payroll-company-id"),
            withheld: !self.can_manage,
            policy: Html::EMPTY,
            classifications: Html::EMPTY,
            items: Html::EMPTY,
            periods: Html::EMPTY,
            mappings: Html::EMPTY,
            runs: Html::EMPTY,
            confirm: self.confirm()?,
            mappings_focus: self.operation == "mappings",
        };
        if let Some(data) = &self.setup {
            view.policy = self.policy(data)?;
            view.classifications = self.classifications(data)?;
            view.items = self.items(data)?;
            view.periods = self.periods(data)?;
            view.mappings = self.mappings(data)?;
            view.runs = self.runs(data)?;
        }
        view.render().map_err(|_| Error::Internal)
    }
    fn policy(&self, data: &Setup) -> Result<Html> {
        let mut body = heading(
            "payroll-policy",
            "Country and currencies",
            if data.country.is_empty() || data.currencies.is_empty() {
                "Choose a country and allowed currencies before creating runs. No statutory rules are activated here."
            } else {
                ""
            },
        )?;
        body.push(&render(&Facts {
            items: vec![
                Fact {
                    title: "Country",
                    value: Html::text(if data.country.is_empty() {
                        "Not configured"
                    } else {
                        &data.country
                    }),
                    ..Fact::default()
                },
                Fact {
                    title: "Currencies",
                    value: Html::text(&data.currencies.join(", ")),
                    ..Fact::default()
                },
            ],
            ..Facts::default()
        })?);
        if self.can_manage {
            body.push(&self.form(
                "payroll-settings",
                "save_settings",
                vec![
                        self.field("save_settings", "", "country", "Country identifier")
                            .value(self.value("save_settings", "country", &data.country)),
                        self.field(
                            "save_settings",
                            "",
                            "currencies",
                            "Currency codes, comma separated",
                        )
                        .value(self.value(
                            "save_settings",
                            "currencies",
                            &data.currencies.join(", "),
                        )),
                    ],
                "Save settings",
            )?);
        }
        card("", body)
    }
    fn classifications(&self, data: &Setup) -> Result<Html> {
        let mut body = heading(
            "payroll-classifications",
            "Classifications",
            "Versions cannot be edited. Use bounded dates to leave room for future versions.",
        )?;
        body.push(
            &self.tables["classifications"].render(
                "Classifications",
                data.classifications
                    .iter()
                    .map(|r| {
                        row("classification", r.id)
                            .slot("code", Html::text(&r.code))
                            .slot("name", Html::text(&r.name))
                            .slot(
                                "effective",
                                Html::text(&effective(
                                    &r.effective_from,
                                    r.effective_to.as_deref(),
                                )),
                            )
                    })
                    .collect(),
                "No classifications yet.",
                "classifications-empty",
            )?,
        );
        if self.can_manage {
            body.push(&self.form(
                "classification-form",
                "create_classification",
                self.catalog_fields("create_classification"),
                "Add classification version",
            )?);
        }
        card("", body)
    }
    fn items(&self, data: &Setup) -> Result<Html> {
        let mut body = heading("payroll-items", "Pay items", "")?;
        body.push(
            &self.tables["items"].render(
                "Pay items",
                data.items
                    .iter()
                    .map(|r| {
                        row("item", r.id)
                            .slot("code", Html::text(&r.code))
                            .slot("name", Html::text(&r.name))
                            .slot("amount", Html::text(&r.amount))
                            .slot("currency", Html::text(&r.currency))
                            .slot(
                                "effective",
                                Html::text(&effective(
                                    &r.effective_from,
                                    r.effective_to.as_deref(),
                                )),
                            )
                    })
                    .collect(),
                "No pay items yet.",
                "items-empty",
            )?,
        );
        if data.classifications.is_empty() || data.currencies.is_empty() {
            body.push(&heading(
                "",
                "",
                "Add classifications and currency settings to create pay items.",
            )?);
        } else if self.can_manage {
            let e = "create_item";
            let mut fields = self.catalog_fields(e);
            fields.truncate(2);
            fields.push(
                self.select(
                    e,
                    "record",
                    "classification_id",
                    "Classification",
                    data.classifications
                        .iter()
                        .map(|r| {
                            (
                                r.id.to_string(),
                                format!("{} · {}", r.name, r.effective_from),
                            )
                        })
                        .collect(),
                ),
            );
            fields.push(self.select(
                e,
                "record",
                "currency",
                "Item currency",
                currency_options(&data.currencies),
            ));
            fields.push(self.field(e, "record", "amount", "Exact amount").required());
            fields.extend(self.dates(e, "record"));
            body.push(&self.form("item-form", e, fields, "Add pay-item version")?);
        }
        card("", body)
    }
    fn periods(&self, data: &Setup) -> Result<Html> {
        let mut body = heading("payroll-periods", "Periods", "")?;
        body.push(
            &self.tables["periods"].render(
                "Periods",
                data.periods
                    .iter()
                    .map(|r| {
                        row("period", r.id)
                            .slot("code", Html::text(&r.code))
                            .slot("starts_on", Html::text(&r.starts_on))
                            .slot("ends_on", Html::text(&r.ends_on))
                            .slot("pay_on", Html::text(&r.pay_on))
                    })
                    .collect(),
                "No payroll periods yet.",
                "periods-empty",
            )?,
        );
        if self.can_manage {
            let e = "create_period";
            body.push(&self.form(
                "period-form",
                e,
                vec![
                        self.field(e, "record", "code", "Period code").required(),
                        self.field(e, "record", "starts_on", "From")
                            .kind(InputKind::Date)
                            .required(),
                        self.field(e, "record", "ends_on", "To")
                            .kind(InputKind::Date)
                            .required(),
                        self.field(e, "record", "pay_on", "Pay on")
                            .kind(InputKind::Date)
                            .required(),
                    ],
                "Add period",
            )?);
        }
        card("", body)
    }
    fn mappings(&self, data: &Setup) -> Result<Html> {
        let mut body = heading("payroll-mappings", "Pay-item mappings", "")?;
        if self.can_map {
            body.push(&render(&ActionLink {
                id: "attendance-mapping-link",
                href: &format!(
                    "/people/payroll/attendance-mappings?company_id={}",
                    self.company_id.unwrap_or_default()
                ),
                body: Html::text("Attendance mappings"),
                ..ActionLink::default()
            })?);
        }
        body.push(
            &self.tables["mappings"].render(
                "Pay-item mappings",
                data.mappings
                    .iter()
                    .map(|r| {
                        row("mapping", r.id)
                            .slot(
                                "source",
                                Html::text(&format!("{} · {}", r.source_kind, r.source_key)),
                            )
                            .slot("item", Html::text(&item_name(&data.items, r.item_id)))
                            .slot(
                                "effective",
                                Html::text(&effective(
                                    &r.effective_from,
                                    r.effective_to.as_deref(),
                                )),
                            )
                    })
                    .collect(),
                "No Leave or Claims mappings yet.",
                "mappings-empty",
            )?,
        );
        let sources: Vec<_> = self
            .sources
            .iter()
            .flat_map(|(kind, rows)| {
                rows.iter().map(move |r| {
                    (
                        format!("{kind}:{}", r.key),
                        format!(
                            "{} · {}",
                            if kind == "leave" { "Leave" } else { "Claims" },
                            r.name
                        ),
                    )
                })
            })
            .collect();
        if data.items.is_empty() || sources.is_empty() {
            body.push(&heading(
                "",
                "",
                "Add pay items and available Leave or Claims types before mapping.",
            )?);
        } else if self.can_manage {
            let e = "create_mapping";
            let mut fields = vec![
                self.select(e, "record", "source", "Mapping source", sources),
                self.select(
                    e,
                    "record",
                    "item_id",
                    "Mapped pay item",
                    data.items
                        .iter()
                        .map(|r| {
                            (
                                r.id.to_string(),
                                format!("{} · {}", r.name, r.effective_from),
                            )
                        })
                        .collect(),
                ),
            ];
            fields.extend(self.dates(e, "record"));
            body.push(&self.form("mapping-form", e, fields, "Add mapping version")?);
        }
        card("", body)
    }
    fn runs(&self, data: &Setup) -> Result<Html> {
        let mut body = heading("payroll-runs","Frozen setup","Freeze a period's setup for a currency. Locking is permanent. Review contributions, calculation, approval and documents on the Payroll runs page.")?;
        body.push(&render(&ActionLink {
            href: &format!(
                "/people/payroll/runs?company_id={}",
                self.company_id.unwrap_or_default()
            ),
            body: Html::text("Open payroll runs"),
            ..ActionLink::default()
        })?);
        let rows = data
            .runs
            .iter()
            .map(|r| {
                let unmapped = r
                    .snapshot
                    .get("unmapped_sources")
                    .and_then(Value::as_array)
                    .map(|v| {
                        v.iter()
                            .map(|s| {
                                format!(
                                    "{} · {}",
                                    s.get("source_kind")
                                        .and_then(Value::as_str)
                                        .unwrap_or_default(),
                                    s.get("name").and_then(Value::as_str).unwrap_or_default()
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                let locked = r
                    .locked_at
                    .as_ref()
                    .and_then(|s| s.parse::<jiff::Timestamp>().ok())
                    .map(|t| {
                        let value = self.display.render(
                            t,
                            bilibi_datetime::Format::DateTime,
                            bilibi_datetime::Precision::Minute,
                        );
                        bilibi_ui::Cell::instant(bilibi_ui::Instant::new(
                            &value.iso,
                            &value.text,
                            &value.company,
                            &value.utc,
                            value.mode.as_str(),
                        ))
                    })
                    .unwrap_or_else(|| bilibi_ui::Cell::text(""));
                let action = if self.can_manage && r.locked_at.is_none() {
                    render(&Button {
                        button_type: "button",
                        event: "prepare_lock",
                        name: "id",
                        value: &r.id.to_string(),
                        body: Html::text("Lock permanently"),
                        ..Button::default()
                    })?
                } else {
                    Html::EMPTY
                };
                Ok(row("run", r.id)
                    .slot("period", Html::text(&r.period_id.to_string()))
                    .slot("currency", Html::text(&r.currency))
                    .slot("country", Html::text(&r.country))
                    .slot("unmapped", Html::text(&unmapped))
                    .cell("locked", locked)
                    .slot("actions", action))
            })
            .collect::<Result<Vec<_>>>()?;
        body.push(&self.tables["runs"].render(
            "Frozen setup",
            rows,
            "No frozen runs yet.",
            "runs-empty",
        )?);
        if self.can_manage
            && !data.periods.is_empty()
            && !data.currencies.is_empty()
            && !data.country.is_empty()
        {
            body.push(&self.form(
                "run-form",
                "create_run",
                vec![
                        self.select(
                            "create_run",
                            "",
                            "period_id",
                            "Run period",
                            data.periods
                                .iter()
                                .map(|p| (p.id.to_string(), p.code.clone()))
                                .collect(),
                        ),
                        self.select(
                            "create_run",
                            "",
                            "currency",
                            "Run currency",
                            currency_options(&data.currencies),
                        ),
                    ],
                "Freeze setup",
            )?);
        }
        card("", body)
    }
    fn confirm(&self) -> Result<Html> {
        match &self.pending {
            Some(Pending::Lock { run,.. }) => render(&ConfirmDialog { id:"payroll-lock-confirm", consequence:&format!("Run {run} will be locked permanently."), detail:"Its frozen setup remains available. The run cannot be changed or deleted after locking.", confirm:"Lock permanently", working:"Locking…", confirm_event:"lock_run", cancel_event:"cancel_lock" }),
            Some(Pending::Freeze { code,currency,.. }) => render(&ConfirmDialog { id:"payroll-freeze-confirm", consequence:&format!("Freeze setup for {code} · {currency}?"), detail:"This preserves the period's current pay definitions and mappings. A period and currency can be frozen only once.", confirm:"Freeze setup", working:"Freezing…", confirm_event:"confirm_freeze", cancel_event:"cancel_freeze" }),
            None => Ok(Html::EMPTY)
        }
    }
    pub(super) fn value(&self, event: &str, key: &str, default: &str) -> String {
        self.drafts
            .get(event)
            .map_or(default, |d| d.get(key))
            .into()
    }
    pub(super) fn field(&self, event: &str, ns: &str, key: &str, label: &str) -> Field {
        let name = if ns.is_empty() {
            key.into()
        } else {
            format!("{ns}[{key}]")
        };
        let mut field = Field::named(name)
            .id(format!("payroll-{event}-{key}"))
            .label(label)
            .wrapper_class("mb-0")
            .value(self.value(event, key, ""));
        if let Some(draft) = self.drafts.get(event) {
            for error in &draft.errors {
                if error.field == key {
                    field = field.error(bilibi_ui::error_sentence(label, &error.message));
                }
            }
        }
        field
    }
    pub(super) fn select(
        &self,
        event: &str,
        ns: &str,
        key: &str,
        label: &str,
        options: Vec<(String, String)>,
    ) -> Field {
        self.field(event, ns, key, label)
            .kind(InputKind::Select)
            .required()
            .options(
                options
                    .into_iter()
                    .map(|(value, label)| bilibi_ui::Choice::new(label, value))
                    .collect(),
            )
    }
    pub(super) fn dates(&self, event: &str, ns: &str) -> Vec<Field> {
        vec![
            self.field(event, ns, "effective_from", "Effective from")
                .kind(InputKind::Date)
                .required(),
            self.field(event, ns, "effective_to", "Effective to")
                .kind(InputKind::Date),
        ]
    }
    fn catalog_fields(&self, event: &str) -> Vec<Field> {
        let mut fields = vec![
            self.field(event, "record", "code", "Code").required(),
            self.field(event, "record", "name", "Name").required(),
        ];
        fields.extend(self.dates(event, "record"));
        fields
    }
    pub(super) fn form(
        &self,
        id: &str,
        event: &str,
        fields: Vec<Field>,
        label: &str,
    ) -> Result<Html> {
        let mut body = Html::EMPTY;
        if let Some(draft) = self.drafts.get(event) {
            let mut summary = ErrorSummary::new(format!("{id}-errors"));
            for error in &draft.errors {
                let field_id = format!("payroll-{event}-{}", error.field);
                let label = fields
                    .iter()
                    .find(|f| f.id == field_id)
                    .map(|f| f.label.as_str())
                    .unwrap_or(error.field);
                summary.push(field_id, bilibi_ui::error_sentence(label, &error.message));
            }
            body.push(&render(&summary)?);
        }
        for field in fields {
            body.push(&render(&field)?);
        }
        render(&Form {
            id,
            event,
            body,
            actions: submit(label)?,
            class: "mt-3 grid gap-3 sm:grid-cols-2",
            change: false,
        })
    }
}
pub(super) fn heading(id: &str, title: &str, description: &str) -> Result<Html> {
    render(&SectionHeading {
        id,
        title,
        description,
        ..SectionHeading::default()
    })
}
pub(super) fn card(id: &str, body: Html) -> Result<Html> {
    render(&Card {
        id,
        body,
        ..Card::default()
    })
}
pub(super) fn item_name(items: &[crate::Item], id: i64) -> String {
    items
        .iter()
        .find(|i| i.id == id)
        .map(|i| i.name.clone())
        .unwrap_or_else(|| "Unavailable item".into())
}
fn currency_options(currencies: &[String]) -> Vec<(String, String)> {
    currencies.iter().map(|c| (c.clone(), c.clone())).collect()
}
