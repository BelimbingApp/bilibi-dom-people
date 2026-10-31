//! Bilimbi Passport.rows_query and Insights.facts; all SQL stays private.
use crate::{PassportEvidence, PassportRow, Result, TrainingError, TrainingPage};
use bilibi_database::Error;
use std::collections::HashMap;
use tokio_postgres::{GenericClient, Row};

pub(crate) const FACTS: &str = "WITH latest AS (
 SELECT DISTINCT ON (session_id,employee_id) id,session_id,employee_id,status,revision
 FROM people_training_attendance_facts WHERE tenant_id=$1 AND company_id=$2
 ORDER BY session_id,employee_id,revision DESC
), facts AS (
 SELECT f.id AS fact_id,f.employee_id,f.status,f.revision,s.id AS session_id,s.name AS session,s.ends_at,c.id AS course_id,c.name AS course
 FROM latest f JOIN people_training_session_runs s ON s.id=f.session_id AND s.tenant_id=$1 AND s.company_id=$2
 JOIN people_training_events e ON e.id=s.event_id AND e.tenant_id=$1 AND e.company_id=$2
 JOIN people_training_courses c ON c.id=e.course_id AND c.tenant_id=$1 AND c.company_id=$2
)";
fn passport_row(r: &Row) -> PassportRow {
    PassportRow {
        fact_id: r.get("fact_id"),
        employee_id: r.get("employee_id"),
        course: r.get("course"),
        session: r.get("session"),
        ends_at: r.get("completed"),
        status: r.get("status"),
        revision: r.get("revision"),
        evidence: Vec::new(),
    }
}
pub(crate) async fn passport(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company: i64,
    employee: i64,
    params: &HashMap<String, String>,
) -> Result<TrainingPage<PassportRow>> {
    let count: i64 = client
        .query_one(
            &format!("{FACTS} SELECT count(*) FROM facts WHERE employee_id=$3"),
            &[&tenant, &company, &employee],
        )
        .await
        .map_err(Error::from)?
        .get(0);
    let mut page = TrainingPage::empty(params, count);
    page.entries = client.query(&format!("{FACTS} SELECT *,to_char(ends_at,'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS completed FROM facts WHERE employee_id=$3 ORDER BY ends_at DESC,fact_id DESC LIMIT $4 OFFSET $5"), &[&tenant,&company,&employee,&page.page_size,&page.offset()]).await.map_err(Error::from)?.iter().map(passport_row).collect();
    let ids: Vec<i64> = page.entries.iter().map(|r| r.fact_id).collect();
    let evidence = client.query("SELECT fact_id,artifact_id::text FROM people_training_evidence WHERE tenant_id=$1 AND company_id=$2 AND fact_id=ANY($3) ORDER BY id", &[&tenant,&company,&ids]).await.map_err(Error::from)?;
    for row in &mut page.entries {
        row.evidence = evidence
            .iter()
            .filter(|e| e.get::<_, i64>(0) == row.fact_id)
            .map(|e| PassportEvidence {
                fact_id: e.get(0),
                artifact_id: e.get(1),
            })
            .collect();
    }
    Ok(page)
}
pub(crate) async fn all_passport(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company: i64,
    employee: i64,
) -> Result<Vec<PassportRow>> {
    let rows = client.query(&format!("{FACTS} SELECT *,to_char(ends_at,'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS completed FROM facts WHERE employee_id=$3 ORDER BY ends_at DESC,fact_id DESC LIMIT 1001"), &[&tenant,&company,&employee]).await.map_err(Error::from)?;
    if rows.len() > 1000 {
        return Err(TrainingError::PassportTooLarge);
    }
    Ok(rows.iter().map(passport_row).collect())
}
