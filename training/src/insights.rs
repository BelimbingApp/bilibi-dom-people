//! Mirrors Bilimbi Training.Insights. Attendance counts never expose evaluation scores.
use crate::{passport_store::FACTS, *};
use jiff::ToSpan;
use serde::Serialize;
use std::collections::HashMap;

pub const INSIGHTS_VIEW: &str = "people.training.insights.view";
#[derive(Clone, Debug, Serialize)]
pub struct InsightRow {
    pub course_id: i64,
    pub course: String,
    pub employees: Option<i64>,
    pub confirmed: Option<i64>,
    pub absent: Option<i64>,
    pub suppressed: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct InsightDrillRow {
    pub fact_id: i64,
    pub session_id: i64,
    pub session: String,
    pub employee_id: i64,
    pub ends_at: String,
    pub course_id: i64,
    pub course: String,
    pub status: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct InsightPeriod {
    pub period_start: String,
    pub period_end: String,
}

fn date(text: Option<&String>) -> Result<Date> {
    let text = text.ok_or(TrainingError::InvalidInsightRange)?;
    let date = text
        .parse::<Date>()
        .map_err(|_| TrainingError::InvalidInsightRange)?;
    if date.to_string() != *text {
        return Err(TrainingError::InvalidInsightRange);
    }
    Ok(date)
}
fn bounds(first: Date, last: Date) -> Result<(String, String)> {
    let next = last
        .checked_add(1.days())
        .map_err(|_| TrainingError::InvalidInsightRange)?;
    Ok((
        day_start(first, "UTC")?.to_string(),
        day_start(next, "UTC")?.to_string(),
    ))
}
fn window(params: &HashMap<String, String>) -> Result<(String, String)> {
    let first = date(params.get("from"))?;
    let last = date(params.get("until"))?;
    if last < first
        || last
            >= first
                .checked_add(366.days())
                .map_err(|_| TrainingError::InvalidInsightRange)?
    {
        return Err(TrainingError::InvalidInsightRange);
    }
    bounds(first, last)
}
const GROUPS: &str = ", groups AS (SELECT course_id,course,count(DISTINCT employee_id) AS employees,
 count(*) FILTER (WHERE status='confirmed') AS confirmed,count(*) FILTER (WHERE status='absent') AS absent
 FROM facts WHERE ends_at >= $3::text::timestamp AND ends_at < $4::text::timestamp GROUP BY course_id,course)";
impl Training {
    /// Frozen periods are the only date choices offered to aggregate-only readers.
    pub async fn learning_insight_periods(
        &self,
        scope: &Scope,
        company: i64,
    ) -> Result<Vec<InsightPeriod>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company, INSIGHTS_VIEW)
            .await?;
        Ok(client.query("SELECT to_char(period_start,'YYYY-MM-DD'),to_char(period_end,'YYYY-MM-DD') FROM people_training_effectiveness_summaries WHERE tenant_id=$1 AND company_id=$2 ORDER BY period_start DESC", &[&scope.tenant(),&company]).await.map_err(Error::from)?.iter().map(|r| InsightPeriod {period_start:r.get(0),period_end:r.get(1)}).collect())
    }
    /// Suppression applies even for record viewers. An aggregate-only viewer must
    /// supply `period`, matching one immutable frozen period; from/until are ignored.
    pub async fn learning_insights(
        &self,
        scope: &Scope,
        company: i64,
        params: &HashMap<String, String>,
    ) -> Result<TrainingPage<InsightRow>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company, INSIGHTS_VIEW)
            .await?;
        let (first, last) = if authorization::allowed(&**client, scope, company, RECORDS_VIEW).await
        {
            window(params)?
        } else {
            let period = params
                .get("period")
                .ok_or(TrainingError::ReportPeriodUnavailable)?;
            let row = client.query_opt("SELECT to_char(period_start,'YYYY-MM-DD'),to_char(period_end,'YYYY-MM-DD') FROM people_training_effectiveness_summaries WHERE tenant_id=$1 AND company_id=$2 AND to_char(period_start,'YYYY-MM-DD')=$3", &[&scope.tenant(),&company,&period]).await.map_err(Error::from)?.ok_or(TrainingError::ReportPeriodUnavailable)?;
            let start: String = row.get(0);
            let end: String = row.get(1);
            bounds(date(Some(&start))?, date(Some(&end))?)?
        };
        let minimum = self
            .settings
            .get(
                &**client,
                scope,
                Target::Company(OrganisationId::new(company)?),
                "people.training.evaluation.minimum_cohort",
            )
            .await?
            .value
            .as_i64()
            .filter(|n| *n >= 2)
            .ok_or(TrainingError::ReportNotConfigured)?;
        let count: i64 = client
            .query_one(
                &format!("{FACTS}{GROUPS} SELECT count(*) FROM groups"),
                &[&scope.tenant(), &company, &first, &last],
            )
            .await
            .map_err(Error::from)?
            .get(0);
        let mut page = TrainingPage::empty(params, count);
        page.entries=client.query(&format!("{FACTS}{GROUPS} SELECT * FROM groups ORDER BY course,course_id LIMIT $5 OFFSET $6"), &[&scope.tenant(),&company,&first,&last,&page.page_size,&page.offset()]).await.map_err(Error::from)?.iter().map(|r| {
            let employees:i64=r.get("employees"); let confirmed:i64=r.get("confirmed"); let absent:i64=r.get("absent");
            let suppressed=employees<minimum; let hide_counts=suppressed || confirmed<minimum || absent<minimum;
            InsightRow { course_id:r.get("course_id"),course:r.get("course"),employees:(!suppressed).then_some(employees),confirmed:(!hide_counts).then_some(confirmed),absent:(!hide_counts).then_some(absent),suppressed }
        }).collect();
        Ok(page)
    }
    /// Named attendance rows require both insights.view and records.view, every call.
    pub async fn learning_insight_drill(
        &self,
        scope: &Scope,
        company: i64,
        course: i64,
        params: &HashMap<String, String>,
    ) -> Result<TrainingPage<InsightDrillRow>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company, INSIGHTS_VIEW)
            .await?;
        self.authorize(&**client, scope, company, RECORDS_VIEW)
            .await?;
        if course <= 0 {
            return Err(TrainingError::NotFound);
        }
        let (first, last) = window(params)?;
        let filter="FROM facts WHERE ends_at >= $3::text::timestamp AND ends_at < $4::text::timestamp AND course_id=$5";
        let count: i64 = client
            .query_one(
                &format!("{FACTS} SELECT count(*) {filter}"),
                &[&scope.tenant(), &company, &first, &last, &course],
            )
            .await
            .map_err(Error::from)?
            .get(0);
        let mut page = TrainingPage::empty(params, count);
        page.entries=client.query(&format!("{FACTS} SELECT *,to_char(ends_at,'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS completed {filter} ORDER BY ends_at DESC,employee_id,fact_id DESC LIMIT $6 OFFSET $7"), &[&scope.tenant(),&company,&first,&last,&course,&page.page_size,&page.offset()]).await.map_err(Error::from)?.iter().map(|r|InsightDrillRow {fact_id:r.get("fact_id"),session_id:r.get("session_id"),session:r.get("session"),employee_id:r.get("employee_id"),ends_at:r.get("completed"),course_id:r.get("course_id"),course:r.get("course"),status:r.get("status")}).collect();
        Ok(page)
    }
}
