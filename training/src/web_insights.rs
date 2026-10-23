//! Mirrors Bilimbi Training.Web.InsightsLive; aggregate privacy stays in the facade.
use crate::{web_evaluation::*, *};
use askama::Template;
use bilibi_database::{Error, Result as DbResult};
use bilibi_menu::{MenuItem, Requirement};
use bilibi_module_registry::{
    contributions::{Access, Method, Route, ROUTES},
    Contribution, Provenance,
};
use bilibi_ui::page::Page;
use bilibi_ui::{page::*, params, *};
use serde_json::Value;
use std::collections::HashMap;
pub(crate) const PAGES: &str = "people/training.insights_pages";
const PROVENANCE: Provenance = Provenance {
    module: MODULE,
    source: file!(),
    line: line!(),
};
inventory::submit! {Contribution::new(ROUTES,"people/training.insights",PROVENANCE,&Route{method:Method::Get,path:"/people/training/insights",service:PAGES,operation:"insights",access:Access::Capability(INSIGHTS_VIEW),framed:true})}
bilibi_menu::contribute!(
    "people/training",
    MenuItem::page(
        "people.reports.learning_insights",
        "Learning insights",
        "/people/training/insights"
    )
    .icon("chart-bar")
    .under("people.reports")
    .requires(Requirement::Capability(INSIGHTS_VIEW))
    .order(20)
);
pub(crate) struct Pages(pub Arc<Training>);
impl Page<Scope, Error> for Pages {
    fn title(&self, _: &str) -> Option<&'static str> {
        Some("Learning insights")
    }
    fn render<'a>(&'a self, req: PageRequest<'a, Scope>) -> PageFuture<'a, Error> {
        Box::pin(async move {
            let mut state = Insights::new(self.0.clone(), req.query)?;
            state.load(req.scope.ok_or(Error::Unauthorized)?).await?;
            Ok(PageBody {
                html: state.render()?,
                live: true,
                title: None,
            })
        })
    }
    fn mount<'a>(&'a self, req: PageRequest<'a, Scope>) -> MountFuture<'a, Scope, Error> {
        Box::pin(async move {
            Ok(Some(Box::new(Insights::new(self.0.clone(), req.query)?)
                as Box<dyn LiveState<Scope, Error>>))
        })
    }
}
fn list_spec() -> DbResult<Arc<ListSpec>> {
    spec(&["company_id", "from", "until", "period", "course_id"], "")
}
struct Insights {
    training: Arc<Training>,
    list: ListState,
    company: Option<i64>,
    companies: Vec<bilibi_people_workforce::CompanySummary>,
    can_drill: bool,
    periods: Vec<InsightPeriod>,
    totals: Vec<InsightRow>,
    drill: Vec<InsightDrillRow>,
    counts: PageCounts,
    columns: ListColumns,
    drill_columns: ListColumns,
    error: Option<String>,
    clock: Option<bilibi_datetime::DisplayContext>,
}
impl Insights {
    fn new(training: Arc<Training>, query: &HashMap<String, String>) -> DbResult<Self> {
        let mut query = query.clone();
        let today = Timestamp::now().to_zoned(jiff::tz::TimeZone::UTC).date();
        query
            .entry("from".into())
            .or_insert_with(|| format!("{}-01-01", today.year()));
        query
            .entry("until".into())
            .or_insert_with(|| today.to_string());
        Ok(Self {
            training,
            list: ListState::parse(&query, list_spec()?).map_err(|_| Error::Invalid)?,
            company: None,
            companies: vec![],
            can_drill: false,
            periods: vec![],
            totals: vec![],
            drill: vec![],
            counts: PageCounts::new(1, 25, 0),
            columns: columns(
                "learning-insights",
                &[
                    ("course", "Course"),
                    ("employees", "Employees"),
                    ("confirmed", "Confirmed"),
                    ("absent", "Absent"),
                    ("records", "Records"),
                ],
            )?,
            drill_columns: columns(
                "insight-drill",
                &[
                    ("employee", "Employee"),
                    ("course", "Course"),
                    ("session", "Session"),
                    ("completed", "Completed"),
                    ("attendance", "Attendance"),
                ],
            )?,
            error: None,
            clock: None,
        })
    }
    fn value(&self, key: &str) -> &str {
        self.list.filters.get(key).map(String::as_str).unwrap_or("")
    }
    fn drilling(&self) -> bool {
        !self.value("course_id").is_empty()
    }
    async fn load(&mut self, scope: &Scope) -> DbResult<()> {
        let client = self.training.db.get().await?;
        self.companies = authorization::selectable_companies(&**client, scope, INSIGHTS_VIEW)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|c| !c.archived)
            .collect();
        self.company = if self.value("company_id").is_empty() {
            self.companies.first().map(|c| c.id)
        } else {
            params::positive_id(Some(self.value("company_id")))
                .filter(|id| self.companies.iter().any(|c| c.id == *id))
        };
        self.can_drill = false;
        self.error = None;
        self.periods.clear();
        self.totals.clear();
        self.drill.clear();
        self.counts = PageCounts::new(1, self.list.page_size, 0);
        let Some(company) = self.company else {
            self.error = Some(
                "No company is available. Ask an operator to check your company access.".into(),
            );
            return Ok(());
        };
        self.list
            .filters
            .insert("company_id".into(), company.to_string());
        self.can_drill = self.training.allowed(scope, company, RECORDS_VIEW).await;
        self.clock = Some(
            bilibi_datetime::DisplayPolicy::new(self.training.settings.clone())
                .display(
                    &**client,
                    scope,
                    Some(OrganisationId::new(company).map_err(|_| Error::Invalid)?),
                    "en",
                )
                .await?,
        );
        drop(client);
        if !self.can_drill {
            match self.training.learning_insight_periods(scope, company).await {
                Ok(p) => self.periods = p,
                Err(e) => self.error = Some(e.to_string()),
            }
            if self.value("period").is_empty() {
                if let Some(period) = self.periods.first() {
                    self.list
                        .filters
                        .insert("period".into(), period.period_start.clone());
                }
            }
        }
        let query = self.list.to_params();
        if self.drilling() {
            let course = params::positive_id(Some(self.value("course_id"))).unwrap_or(0);
            match self
                .training
                .learning_insight_drill(scope, company, course, &query)
                .await
            {
                Ok(page) => {
                    self.list.page = page.page as u64;
                    self.counts = PageCounts::new(
                        page.page as u64,
                        page.page_size as u64,
                        page.total_entries as u64,
                    );
                    self.drill = page.entries;
                }
                Err(e) => self.error = Some(e.to_string()),
            }
        } else {
            match self
                .training
                .learning_insights(scope, company, &query)
                .await
            {
                Ok(page) => {
                    self.list.page = page.page as u64;
                    self.counts = PageCounts::new(
                        page.page as u64,
                        page.page_size as u64,
                        page.total_entries as u64,
                    );
                    self.totals = page.entries;
                }
                Err(e) => self.error = Some(e.to_string()),
            }
        }
        Ok(())
    }
    fn event(&mut self, payload: &Value) {
        let event = payload.get("type").and_then(Value::as_str).unwrap_or("");
        match event {
            "grid" => {
                self.columns.handle(&params::string_values(payload));
            }
            "drill_grid" => {
                self.drill_columns.handle(&params::string_values(payload));
            }
            "page" => self
                .list
                .put_page(payload.get("page").and_then(Value::as_str)),
            "filter" | "filters" => {
                let values = params::form_values(payload, "filters");
                let changed = values
                    .get("company_id")
                    .is_some_and(|v| v != self.value("company_id"));
                self.list.apply_filters(&values);
                if changed {
                    self.list.filters.insert("period".into(), String::new());
                    self.list.filters.insert("course_id".into(), String::new());
                    self.list.page = 1;
                }
            }
            _ => {}
        }
    }
    fn render(&self) -> DbResult<String> {
        let mut controls = vec![ToolbarControl::select(
            "insights-company",
            "company_id",
            "Company",
            &self.company.map(|n| n.to_string()).unwrap_or_default(),
            self.companies
                .iter()
                .map(|c| Choice::new(&c.name, c.id.to_string()))
                .collect(),
        )];
        if self.can_drill {
            controls.push(ToolbarControl::date(
                "insights-from",
                "from",
                "From",
                self.value("from"),
            ));
            controls.push(ToolbarControl::date(
                "insights-until",
                "until",
                "Through",
                self.value("until"),
            ));
        } else {
            controls.push(ToolbarControl::select(
                "insights-period",
                "period",
                "Reporting period",
                self.value("period"),
                self.periods
                    .iter()
                    .map(|p| {
                        Choice::new(
                            format!("{} – {}", p.period_start, p.period_end),
                            &p.period_start,
                        )
                    })
                    .collect(),
            ));
        }
        let toolbar = render(&FilterToolbar {
            id: "insights-filters",
            event: "filter",
            controls,
            class: "",
            target: "",
        })?;
        let mut body = Html::EMPTY;
        if self.drilling() {
            let mut query = self.list.to_params();
            query.remove("course_id");
            query.remove("page");
            body.push(&render(&BackLink {
                href: &params::address("/people/training/insights", &query),
                title: "Back to course totals",
                ..BackLink::default()
            })?);
            let rows = self
                .drill
                .iter()
                .map(|r| {
                    let mut row = FlexRow::new(r.fact_id.to_string())
                        .cell("employee", Cell::text(r.employee_id.to_string()))
                        .cell("course", Cell::text(&r.course))
                        .cell("session", Cell::text(&r.session))
                        .cell("attendance", Cell::text(&r.status));
                    if let (Some(clock), Ok(instant)) =
                        (&self.clock, r.ends_at.parse::<Timestamp>())
                    {
                        let date = clock.render(
                            instant,
                            bilibi_datetime::Format::DateTime,
                            bilibi_datetime::Precision::Minute,
                        );
                        row = row.cell(
                            "completed",
                            Cell::instant(Instant::new(
                                &date.iso,
                                &date.text,
                                &date.company,
                                &date.utc,
                                date.mode.as_str(),
                            )),
                        );
                    }
                    row
                })
                .collect();
            body.push(&table(
                "insight-drill",
                &self.drill_columns,
                rows,
                "drill_grid",
                "No accessible attendance",
                "Check your record-view access, company and date range.",
            )?);
        } else {
            let rows = self
                .totals
                .iter()
                .map(|r| {
                    let records = if self.can_drill {
                        let mut query = self.list.to_params();
                        query.insert("course_id".into(), r.course_id.to_string());
                        query.insert("page".into(), "1".into());
                        render(&Button {
                            href: &params::address("/people/training/insights", &query),
                            body: Html::text("View attendance"),
                            ..Button::default()
                        })?
                    } else {
                        Html::text("Record-view access required")
                    };
                    let count = |n: Option<i64>| {
                        Cell::text(
                            n.map(|n| n.to_string())
                                .unwrap_or_else(|| "Suppressed".into()),
                        )
                    };
                    Ok(FlexRow::new(r.course_id.to_string())
                        .cell("course", Cell::text(&r.course))
                        .cell("employees", count(r.employees))
                        .cell("confirmed", count(r.confirmed))
                        .cell("absent", count(r.absent))
                        .slot("records", records))
                })
                .collect::<DbResult<Vec<_>>>()?;
            body.push(&table(
                "learning-insights",
                &self.columns,
                rows,
                "grid",
                "No attendance in this period",
                "Choose another completion period or confirm session attendance.",
            )?);
        }
        if self.error.is_none() {
            let mut pagination =
                Pagination::new("insights-pagination", self.counts, &[25, 50, 100, 300]);
            pagination.filters_event = "filter";
            body.push(&render(&pagination)?);
        }
        InsightsView {
            toolbar,
            can_drill: self.can_drill,
            error: self.error.as_deref().unwrap_or(""),
            body,
        }
        .render()
        .map_err(|_| Error::Internal)
    }
}
impl LiveState<Scope, Error> for Insights {
    fn update<'a>(&'a mut self, req: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            match req.input {
                LiveInput::Refresh => {}
                LiveInput::Params(query) => {
                    self.list = Self::new(self.training.clone(), query)?.list
                }
                LiveInput::Event(payload) => self.event(payload),
            }
            self.load(req.scope).await?;
            Ok(LiveBody {
                html: self.render()?,
                query: self.list.to_params(),
                rows: self.totals.len() + self.drill.len(),
                ..LiveBody::default()
            })
        })
    }
}
#[derive(Template)]
#[template(path = "learning_insights.html")]
struct InsightsView<'a> {
    toolbar: Html,
    can_drill: bool,
    error: &'a str,
    body: Html,
}
