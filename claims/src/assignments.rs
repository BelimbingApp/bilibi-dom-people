//! Assignments: `people_claim_employee_enrolments` and its member tables
//! `people_claim_assignment_types` and `people_claim_assignment_employees`.
//! Rows, SQL, validation and writes behind the facade. Private.
use crate::{
    date_column, period_field, required_date, text_field, Assignment, AssignmentMembers,
    ClaimsError, FieldError, NewAssignment, Result, Tx,
};
use bilibi_database::{Change, Error};
use jiff::civil::Date;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use tokio_postgres::{GenericClient, Row};

const COLUMNS: &str = "id, code, name, to_char(effective_from, 'YYYY-MM-DD') AS effective_from, \
    to_char(effective_to, 'YYYY-MM-DD') AS effective_to";

fn read(row: &Row) -> Result<Assignment> {
    Ok(Assignment {
        id: row.get("id"),
        code: row.get("code"),
        name: row.get("name"),
        effective_from: required_date(row, "effective_from")?,
        effective_to: date_column(row, "effective_to")?,
        claim_type_ids: Vec::new(),
        employee_ids: Vec::new(),
    })
}

impl Assignment {
    /// The row as audit records it; members are their own changes.
    pub(crate) fn audit_values(&self) -> Value {
        json!({
            "code": self.code,
            "name": self.name,
            "effective_from": self.effective_from.to_string(),
            "effective_to": self.effective_to.map(|date| date.to_string()),
        })
    }
}

/// Every assignment of the company with its member ids (Bilimbi
/// `assignments/2`).
pub(crate) async fn list(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
) -> Result<Vec<Assignment>> {
    let mut assignments: Vec<Assignment> = client
        .query(
            &format!("SELECT {COLUMNS} FROM people_claim_employee_enrolments WHERE tenant_id = $1 AND company_id = $2 ORDER BY name, id"),
            &[&tenant, &company_id],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(read)
        .collect::<Result<_>>()?;
    if assignments.is_empty() {
        return Ok(assignments);
    }
    let ids: Vec<i64> = assignments.iter().map(|assignment| assignment.id).collect();
    let types = member_ids(
        client,
        tenant,
        company_id,
        "people_claim_assignment_types",
        "claim_type_id",
        &ids,
    )
    .await?;
    let employees = member_ids(
        client,
        tenant,
        company_id,
        "people_claim_assignment_employees",
        "employee_id",
        &ids,
    )
    .await?;
    for assignment in &mut assignments {
        assignment.claim_type_ids = types.get(&assignment.id).cloned().unwrap_or_default();
        assignment.employee_ids = employees.get(&assignment.id).cloned().unwrap_or_default();
    }
    Ok(assignments)
}

async fn member_ids(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    table: &str,
    column: &str,
    assignment_ids: &[i64],
) -> Result<BTreeMap<i64, Vec<i64>>> {
    let rows = client
        .query(
            &format!("SELECT assignment_id, {column} AS member FROM {table} WHERE tenant_id = $1 AND company_id = $2 AND assignment_id = ANY($3::bigint[]) ORDER BY {column}"),
            &[&tenant, &company_id, &assignment_ids],
        )
        .await
        .map_err(Error::from)?;
    let mut members: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
    for row in rows {
        members
            .entry(row.get("assignment_id"))
            .or_default()
            .push(row.get("member"));
    }
    Ok(members)
}

/// One assignment's member ids, sorted (for a row read outside `list`).
pub(crate) async fn members_of(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    assignment: &mut Assignment,
) -> Result<()> {
    let ids = [assignment.id];
    let types = member_ids(
        client,
        tenant,
        company_id,
        "people_claim_assignment_types",
        "claim_type_id",
        &ids,
    )
    .await?;
    let employees = member_ids(
        client,
        tenant,
        company_id,
        "people_claim_assignment_employees",
        "employee_id",
        &ids,
    )
    .await?;
    assignment.claim_type_ids = types.get(&assignment.id).cloned().unwrap_or_default();
    assignment.employee_ids = employees.get(&assignment.id).cloned().unwrap_or_default();
    Ok(())
}

/// One assignment of the company, locked `FOR UPDATE`, without its members.
pub(crate) async fn lock(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    id: i64,
) -> Result<Option<Assignment>> {
    if id <= 0 {
        return Ok(None);
    }
    client
        .query_opt(
            &format!("SELECT {COLUMNS} FROM people_claim_employee_enrolments WHERE tenant_id = $1 AND company_id = $2 AND id = $3 FOR UPDATE"),
            &[&tenant, &company_id, &id],
        )
        .await
        .map_err(Error::from)?
        .as_ref()
        .map(read)
        .transpose()
}

/// An assignment once validated.
pub(crate) struct Valid {
    pub code: String,
    pub name: String,
    pub effective_from: Date,
    pub effective_to: Option<Date>,
}

/// Bilimbi `Assignment.changeset/2`, in its words.
pub(crate) fn validate(new: &NewAssignment) -> std::result::Result<Valid, Vec<FieldError>> {
    let mut errors = Vec::new();
    let code = text_field(&mut errors, "code", &new.code, 60);
    let name = text_field(&mut errors, "name", &new.name, 120);
    period_field(&mut errors, new.effective_from, new.effective_to);
    if errors.is_empty() {
        Ok(Valid {
            code,
            name,
            effective_from: new.effective_from,
            effective_to: new.effective_to,
        })
    } else {
        Err(errors)
    }
}

pub(crate) async fn check_code(
    client: &impl GenericClient,
    company_id: i64,
    code: &str,
) -> Result<()> {
    let taken = client
        .query_opt(
            "SELECT 1 FROM people_claim_employee_enrolments WHERE company_id = $1 AND code = $2",
            &[&company_id, &code],
        )
        .await
        .map_err(Error::from)?;
    if taken.is_some() {
        return Err(ClaimsError::Invalid(vec![FieldError::new(
            "code",
            "has already been taken",
        )]));
    }
    Ok(())
}

pub(crate) async fn insert(
    tx: &mut Tx<'_>,
    tenant: i64,
    company_id: i64,
    valid: Valid,
) -> Result<Assignment> {
    tx.mutate(move |sql| {
        Box::pin(async move {
            let from = valid.effective_from.to_string();
            let to = valid.effective_to.map(|date| date.to_string());
            let row = sql
                .query_one(
                    &format!(
                        "WITH a AS (INSERT INTO people_claim_employee_enrolments \
                         (tenant_id, company_id, code, name, effective_from, effective_to) \
                         VALUES ($1, $2, $3, $4, $5::text::date, $6::text::date) RETURNING *) \
                         SELECT {COLUMNS} FROM a"
                    ),
                    &[&tenant, &company_id, &valid.code, &valid.name, &from, &to],
                )
                .await?;
            let assignment = read(&row).map_err(|_| Error::Internal)?;
            let change = Change {
                kind: "people_claim_employee_enrolment".into(),
                id: assignment.id,
                old: json!({}),
                new: assignment.audit_values(),
            };
            Ok((assignment, vec![change]))
        })
    })
    .await
    .map_err(Into::into)
}

pub(crate) async fn end(
    tx: &mut Tx<'_>,
    assignment: Assignment,
    effective_to: Date,
) -> Result<Assignment> {
    tx.mutate(move |sql| {
        Box::pin(async move {
            let to = effective_to.to_string();
            let row = sql
                .query_one(
                    &format!(
                        "WITH a AS (UPDATE people_claim_employee_enrolments SET effective_to = $2::text::date, \
                         updated_at = now() AT TIME ZONE 'UTC' WHERE id = $1 RETURNING *) \
                         SELECT {COLUMNS} FROM a"
                    ),
                    &[&assignment.id, &to],
                )
                .await?;
            let mut ended = read(&row).map_err(|_| Error::Internal)?;
            ended.claim_type_ids = assignment.claim_type_ids.clone();
            ended.employee_ids = assignment.employee_ids.clone();
            let change = Change {
                kind: "people_claim_employee_enrolment".into(),
                id: ended.id,
                old: assignment.audit_values(),
                new: ended.audit_values(),
            };
            Ok((ended, vec![change]))
        })
    })
    .await
    .map_err(Into::into)
}

/// Replaces both member lists under the assignment's lock (Bilimbi
/// `replace_members/4`): stale rows are deleted and missing ones inserted,
/// each its own `Change`. Returns the sorted members and whether anything
/// changed; when nothing did, no write is made.
pub(crate) async fn replace_members(
    tx: &mut Tx<'_>,
    assignment: &Assignment,
    claim_type_ids: Vec<i64>,
    employee_ids: Vec<i64>,
) -> Result<(AssignmentMembers, bool)> {
    let mut claim_type_ids = claim_type_ids;
    let mut employee_ids = employee_ids;
    claim_type_ids.sort_unstable();
    employee_ids.sort_unstable();
    let plans = [
        MemberPlan::new(
            tx.sql(),
            assignment,
            "people_claim_assignment_types",
            "people_claim_assignment_type",
            "claim_type_id",
            &claim_type_ids,
        )
        .await?,
        MemberPlan::new(
            tx.sql(),
            assignment,
            "people_claim_assignment_employees",
            "people_claim_assignment_employee",
            "employee_id",
            &employee_ids,
        )
        .await?,
    ];
    let members = AssignmentMembers {
        claim_type_ids,
        employee_ids,
    };
    if plans.iter().all(MemberPlan::is_noop) {
        return Ok((members, false));
    }
    let assignment_id = assignment.id;
    tx.mutate(move |sql| {
        Box::pin(async move {
            let mut changes = Vec::new();
            for plan in plans {
                for (row_id, member) in &plan.stale {
                    sql.execute(
                        &format!("DELETE FROM {} WHERE id = $1", plan.table),
                        &[row_id],
                    )
                    .await?;
                    changes.push(Change {
                        kind: plan.kind.into(),
                        id: *row_id,
                        old: json!({ "assignment_id": assignment_id, plan.column: member }),
                        new: json!({}),
                    });
                }
                for member in &plan.missing {
                    let row = sql
                        .query_one(
                            &format!(
                                "INSERT INTO {} (tenant_id, company_id, assignment_id, {}) \
                                 SELECT tenant_id, company_id, id, $2 FROM people_claim_employee_enrolments WHERE id = $1 \
                                 RETURNING id",
                                plan.table, plan.column
                            ),
                            &[&assignment_id, member],
                        )
                        .await?;
                    let row_id: i64 = row.get(0);
                    changes.push(Change {
                        kind: plan.kind.into(),
                        id: row_id,
                        old: json!({}),
                        new: json!({ "assignment_id": assignment_id, plan.column: member }),
                    });
                }
            }
            Ok(((), changes))
        })
    })
    .await?;
    Ok((members, true))
}

/// What one member table needs to match the wanted list.
struct MemberPlan {
    table: &'static str,
    kind: &'static str,
    column: &'static str,
    /// `(row id, member id)` of rows to delete.
    stale: Vec<(i64, i64)>,
    /// Member ids to insert.
    missing: Vec<i64>,
}
impl MemberPlan {
    async fn new(
        client: &impl GenericClient,
        assignment: &Assignment,
        table: &'static str,
        kind: &'static str,
        column: &'static str,
        wanted: &[i64],
    ) -> Result<Self> {
        let existing: Vec<(i64, i64)> = client
            .query(
                &format!("SELECT id, {column} AS member FROM {table} WHERE assignment_id = $1 ORDER BY id"),
                &[&assignment.id],
            )
            .await
            .map_err(Error::from)?
            .iter()
            .map(|row| (row.get("id"), row.get("member")))
            .collect();
        let stale = existing
            .iter()
            .copied()
            .filter(|(_, member)| !wanted.contains(member))
            .collect();
        let missing = wanted
            .iter()
            .copied()
            .filter(|member| !existing.iter().any(|(_, present)| present == member))
            .collect();
        Ok(Self {
            table,
            kind,
            column,
            stale,
            missing,
        })
    }
    fn is_noop(&self) -> bool {
        self.stale.is_empty() && self.missing.is_empty()
    }
}

/// Claim types an assignment in effect on `on_date` opens to the employee
/// (Bilimbi `assigned_type_ids/4`), distinct.
pub(crate) async fn assigned_type_ids(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    employee_id: i64,
    on_date: Date,
) -> Result<Vec<i64>> {
    if employee_id <= 0 {
        return Ok(Vec::new());
    }
    let on_date = on_date.to_string();
    Ok(client
        .query(
            "SELECT DISTINCT at.claim_type_id FROM people_claim_employee_enrolments a \
             JOIN people_claim_assignment_types at ON at.assignment_id = a.id AND at.tenant_id = a.tenant_id \
             JOIN people_claim_assignment_employees ae ON ae.assignment_id = a.id AND ae.tenant_id = a.tenant_id \
             WHERE a.tenant_id = $1 AND a.company_id = $2 AND ae.employee_id = $3 \
               AND a.effective_from <= $4::text::date AND (a.effective_to IS NULL OR a.effective_to >= $4::text::date)",
            &[&tenant, &company_id, &employee_id, &on_date],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(|row| row.get(0))
        .collect())
}
