//! Bilimbi Skills.Access: reach and subject independence. No form supplies
//! the account or its employment link; Workforce re-proves both on each call.
use crate::{Result, Skills, SkillsError, ASSESSMENTS_MANAGE};
use bilibi_database::Error;
use bilibi_people_workforce::{authorization, Employee};
use bilibi_tenancy::Scope;
use std::collections::{HashMap, HashSet};
use tokio_postgres::GenericClient;

pub(crate) enum Reach {
    Company,
    Team(HashSet<i64>),
}
impl Reach {
    pub(crate) fn contains(&self, id: i64) -> bool {
        match self {
            Self::Company => true,
            Self::Team(ids) => ids.contains(&id),
        }
    }
    /// None means company-wide; Some(empty) means no reach.
    pub(crate) fn ids(&self) -> Option<Vec<i64>> {
        match self {
            Self::Company => None,
            Self::Team(ids) => Some(ids.iter().copied().collect()),
        }
    }
}

pub(crate) fn employee_id(employee: &Employee) -> Result<i64> {
    employee
        .reference
        .stable_id
        .parse()
        .map_err(|_| SkillsError::Failed(Error::Internal))
}

pub(crate) async fn authorize_any(
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company: i64,
    capabilities: &[&str],
) -> Result<()> {
    for capability in capabilities {
        if authorization::allowed(client, scope, company, capability).await {
            return Ok(());
        }
    }
    Err(SkillsError::Unauthorized)
}

pub(crate) async fn reach(
    skills: &Skills,
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company: i64,
) -> Result<Reach> {
    if authorization::allowed(client, scope, company, ASSESSMENTS_MANAGE).await {
        return Ok(Reach::Company);
    }
    let own = skills
        .workforce
        .linked_employee(client, scope, company)
        .await?;
    let Some(manager) = own else {
        return Ok(Reach::Team(HashSet::new()));
    };
    let employees = skills
        .workforce
        .employees(scope, company)
        .await?
        .require_current()?;
    if !employees
        .iter()
        .any(|e| e.reference.stable_id == manager.to_string())
    {
        return Ok(Reach::Team(HashSet::new()));
    }
    let mut reports: HashMap<i64, Vec<i64>> = HashMap::new();
    for employee in employees {
        if let Some(supervisor) = &employee.supervisor_reference {
            let supervisor: i64 = supervisor
                .stable_id
                .parse()
                .map_err(|_| SkillsError::Failed(Error::Internal))?;
            reports
                .entry(supervisor)
                .or_default()
                .push(employee_id(&employee)?);
        }
    }
    let mut seen = HashSet::from([manager]);
    let mut found = HashSet::new();
    let mut frontier = vec![manager];
    for _ in 0..bilibi_people_workforce::CHAIN_DEPTH {
        let mut next = Vec::new();
        for supervisor in frontier {
            for &id in reports.get(&supervisor).into_iter().flatten() {
                if seen.insert(id) {
                    found.insert(id);
                    next.push(id);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    Ok(Reach::Team(found))
}

pub(crate) async fn independent(
    skills: &Skills,
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company: i64,
    employee: i64,
    assessor: i64,
) -> Result<()> {
    if scope.user() == assessor
        || skills
            .workforce
            .linked_employee(client, scope, company)
            .await?
            == Some(employee)
    {
        return Err(SkillsError::SelfApproval);
    }
    Ok(())
}

/// Names are Core summaries read only through Workforce. Former employees
/// retain the reference fallback, matching Bilimbi's current-only lookup.
pub(crate) async fn names(
    skills: &Skills,
    scope: &Scope,
    company: i64,
    ids: &[i64],
) -> Result<HashMap<i64, String>> {
    let unique: HashSet<i64> = ids.iter().copied().collect();
    let ids: Vec<i64> = unique.into_iter().collect();
    let mut names = HashMap::new();
    for batch in ids.chunks(bilibi_people_workforce::LOOKUP_LIMIT) {
        for employee in skills
            .workforce
            .employees_by_ids(scope, company, batch)
            .await?
            .require_current()?
        {
            names.insert(
                employee_id(&employee)?,
                format!("{} ({})", employee.display_name, employee.employee_number),
            );
        }
    }
    Ok(names)
}
