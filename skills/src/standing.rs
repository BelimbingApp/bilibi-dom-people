//! Bilimbi Skills.Standing: projections over finalized scores. Required
//! levels and provenance remain those of the assessment, never today's profile.
use crate::{
    access, assessments, catalog, policy, rows, Coverage, Result, ScoreView, Skills, Standing,
    StandingAction, StandingReassessment,
};
use bilibi_database::Error;
use bilibi_tenancy::Scope;
use jiff::civil::Date;
use tokio_postgres::{GenericClient, Row};

fn columns() -> String {
    format!("{},a.profile_id AS assessment_profile_id,p.version AS assessment_profile_version,a.scale_id AS assessment_scale_id FROM people_skill_scores s JOIN people_skill_assessments a ON a.id=s.assessment_id AND a.tenant_id=s.tenant_id AND a.company_id=s.company_id JOIN people_skill_profiles p ON p.id=a.profile_id AND p.tenant_id=a.tenant_id AND p.company_id=a.company_id",assessments::score_columns("s."))
}

async fn present(
    skills: &Skills,
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company: i64,
    records: Vec<Row>,
) -> Result<Vec<ScoreView>> {
    let names = access::names(
        skills,
        scope,
        company,
        &records
            .iter()
            .map(|r| r.get("employee_id"))
            .collect::<Vec<_>>(),
    )
    .await?;
    let catalog = catalog::skills_by_ids(
        client,
        scope.tenant(),
        company,
        &records
            .iter()
            .map(|r| r.get("skill_id"))
            .collect::<Vec<_>>(),
    )
    .await?;
    let today = assessments::today();
    records
        .into_iter()
        .map(|r| {
            let score = assessments::score(&r)?;
            let skill = catalog.iter().find(|s| s.id == score.skill_id);
            let state = if score.valid_until.is_some_and(|d| d < today) {
                "expired"
            } else if score.next_due_on.is_some_and(|d| d <= today) {
                "overdue"
            } else {
                "current"
            };
            Ok(ScoreView {
                result_band: assessments::band(
                    score.gap,
                    score.current_level,
                    score.required_level,
                )
                .into(),
                employee_name: names
                    .get(&score.employee_id)
                    .cloned()
                    .unwrap_or_else(|| format!("Employee #{}", score.employee_id)),
                skill_code: skill.map(|s| s.code.clone()),
                skill_name: skill.map(|s| s.name.clone()),
                state: state.into(),
                assessment_profile_id: r.get("assessment_profile_id"),
                assessment_profile_version: r.get("assessment_profile_version"),
                assessment_scale_id: r.get("assessment_scale_id"),
                score,
            })
        })
        .collect()
}

pub(crate) async fn gaps(
    skills: &Skills,
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company: i64,
    reach: &access::Reach,
) -> Result<Vec<ScoreView>> {
    let ids = reach.ids();
    let records=client.query(&format!("SELECT {} WHERE s.tenant_id=$1 AND s.company_id=$2 AND s.gap>0 AND ($3::bigint[] IS NULL OR s.employee_id=ANY($3)) ORDER BY s.mandatory DESC,s.priority_score DESC,s.next_due_on,s.id LIMIT 500",columns()), &[&scope.tenant(),&company,&ids]).await.map_err(Error::from)?;
    present(skills, client, scope, company, records).await
}

pub(crate) async fn own(
    skills: &Skills,
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company: i64,
    employee: i64,
) -> Result<Standing> {
    let records=client.query(&format!("SELECT {} WHERE s.tenant_id=$1 AND s.company_id=$2 AND s.employee_id=$3 ORDER BY s.skill_id LIMIT 500",columns()), &[&scope.tenant(),&company,&employee]).await.map_err(Error::from)?;
    let scores = present(skills, client, scope, company, records).await?;
    let actions=client.query(&format!("SELECT a.id,a.skill_id,k.name AS skill_name,a.status,a.closure,a.starting_level,a.target_level,a.objective,{},{},a.post_level,a.improvement FROM people_skill_actions a LEFT JOIN people_skills k ON k.id=a.skill_id AND k.tenant_id=a.tenant_id AND k.company_id=a.company_id WHERE a.tenant_id=$1 AND a.company_id=$2 AND a.employee_id=$3 AND a.status NOT IN ('proposed','cancelled') ORDER BY a.due_on,a.id LIMIT 500",rows::date_text("a.start_on"),rows::date_text("a.due_on")), &[&scope.tenant(),&company,&employee]).await.map_err(Error::from)?.into_iter().map(|r| {
        Ok(StandingAction{id:r.get("id"),skill_id:r.get("skill_id"),skill_name:r.get("skill_name"),status:r.get("status"),closure:r.get("closure"),starting_level:r.get("starting_level"),target_level:r.get("target_level"),objective:r.get("objective"),start_on:rows::date(&r,"start_on")?,due_on:rows::date(&r,"due_on")?,post_level:r.get("post_level"),improvement:r.get("improvement")})
    }).collect::<Result<Vec<_>>>()?;
    let reassessments=client.query(&format!("SELECT r.id,r.skill_id,k.name AS skill_name,{} FROM people_skill_reassessment_requests r LEFT JOIN people_skills k ON k.id=r.skill_id AND k.tenant_id=r.tenant_id AND k.company_id=r.company_id WHERE r.tenant_id=$1 AND r.company_id=$2 AND r.employee_id=$3 AND r.status='pending' ORDER BY r.due_on,r.id",rows::date_text("r.due_on")), &[&scope.tenant(),&company,&employee]).await.map_err(Error::from)?.into_iter().map(|r| {
        Ok(StandingReassessment{id:r.get("id"),skill_id:r.get("skill_id"),skill_name:r.get("skill_name"),due_on:rows::date(&r,"due_on")?})
    }).collect::<Result<Vec<_>>>()?;
    Ok(Standing {
        employee_id: employee,
        scores,
        actions,
        reassessments,
    })
}

pub(crate) async fn coverage(
    skills: &Skills,
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company: i64,
    as_of: Date,
) -> Result<Vec<Coverage>> {
    let policy = policy::get(&skills.settings, client, scope, company).await?;
    let working = skills
        .workforce
        .employees(scope, company)
        .await?
        .require_current()?
        .iter()
        .map(access::employee_id)
        .collect::<Result<Vec<_>>>()?;
    let rows=client.query("SELECT k.id AS skill_id,k.code,k.name,COUNT(s.id) AS holders FROM people_skills k LEFT JOIN people_skill_scores s ON s.skill_id=k.id AND s.tenant_id=k.tenant_id AND s.company_id=k.company_id AND s.current_level>=s.required_level AND (s.valid_until IS NULL OR s.valid_until >= $3::text::date) AND s.employee_id=ANY($4::bigint[]) WHERE k.tenant_id=$1 AND k.company_id=$2 AND k.active AND k.critical GROUP BY k.id,k.code,k.name ORDER BY k.name,k.id", &[&scope.tenant(),&company,&as_of.to_string(),&working]).await.map_err(Error::from)?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let holders = r.get("holders");
            Coverage {
                skill_id: r.get("skill_id"),
                code: r.get("code"),
                name: r.get("name"),
                holders,
                minimum: policy.backup_minimum,
                covered: holders >= policy.backup_minimum,
            }
        })
        .collect())
}
