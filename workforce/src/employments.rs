//! The `people_workforce_employments` table: its rows, the SQL that reads
//! and locks them, and the validation, insert and update behind
//! `Workforce::create_employment` and `Workforce::update_employment`.
//! Private to the facade.
use crate::{
    Employment, EmploymentChange, EmploymentSummary, FieldError, NewEmployment, Reference, Result,
    StatusCount, WorkforceError, CHAIN_DEPTH, EMPLOYEE_STATUSES,
};
use bilibi_database::{Change, Error, MutationTransaction};
use jiff::civil::Date;
use serde_json::{json, Value};
use tokio_postgres::{GenericClient, Row};

/// The columns every read selects, dates as ISO text (the workspace's
/// `tokio-postgres` has no date feature, and a Domain must not turn one on).
const COLUMNS: &str = "e.id, e.company_id, e.person_id, e.affiliation_id, e.employee_number, e.employment_type, e.status, \
    to_char(e.employment_start, 'YYYY-MM-DD') AS employment_start, to_char(e.employment_end, 'YYYY-MM-DD') AS employment_end, \
    e.supervisor_employment_id, e.department_id, e.metadata, e.version, s.status AS supervisor_status";
/// Every read joins the supervisor's row, so a supervisor reference is shown
/// only while the supervisor is a working employee of the same company.
const FROM: &str = "FROM people_workforce_employments e \
    LEFT JOIN people_workforce_employments s ON s.id = e.supervisor_employment_id AND s.company_id = e.company_id";

/// One row with the supervisor's status beside it.
pub(crate) struct Row_ {
    pub employment: Employment,
    pub supervisor_status: Option<String>,
}

fn date(row: &Row, column: &str) -> Result<Option<Date>> {
    row.get::<_, Option<String>>(column)
        .map(|text| text.parse::<Date>().map_err(|_| Error::Internal))
        .transpose()
        .map_err(WorkforceError::Failed)
}

fn read(row: &Row) -> Result<Row_> {
    Ok(Row_ {
        employment: Employment {
            id: row.get("id"),
            platform_company_id: row.get("company_id"),
            person_id: row.get("person_id"),
            affiliation_id: row.get("affiliation_id"),
            employee_number: row.get("employee_number"),
            employment_type: row.get("employment_type"),
            status: row.get("status"),
            employment_start: date(row, "employment_start")?,
            employment_end: date(row, "employment_end")?,
            supervisor_employment_id: row.get("supervisor_employment_id"),
            department_id: row.get("department_id"),
            metadata: row.get("metadata"),
            version: row.get("version"),
        },
        supervisor_status: row.get("supervisor_status"),
    })
}

/// The company's employments in the given statuses, by id.
pub(crate) async fn in_statuses(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    statuses: &[String],
) -> Result<Vec<Row_>> {
    client
        .query(
            &format!("SELECT {COLUMNS} {FROM} WHERE e.tenant_id = $1 AND e.company_id = $2 AND e.status = ANY($3::text[]) ORDER BY e.id"),
            &[&tenant, &company_id, &statuses],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(read)
        .collect()
}

/// The employments among `ids` in the given statuses, by id.
pub(crate) async fn by_ids(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    ids: &[i64],
    statuses: &[String],
) -> Result<Vec<Row_>> {
    client
        .query(
            &format!("SELECT {COLUMNS} {FROM} WHERE e.tenant_id = $1 AND e.company_id = $2 AND e.id = ANY($3::bigint[]) AND e.status = ANY($4::text[]) ORDER BY e.id"),
            &[&tenant, &company_id, &ids, &statuses],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(read)
        .collect()
}

/// The working employments reporting to one employment, by id.
pub(crate) async fn reporting_to(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    supervisor_id: i64,
    statuses: &[String],
) -> Result<Vec<Row_>> {
    client
        .query(
            &format!("SELECT {COLUMNS} {FROM} WHERE e.tenant_id = $1 AND e.company_id = $2 AND e.supervisor_employment_id = $3 AND e.status = ANY($4::text[]) ORDER BY e.id"),
            &[&tenant, &company_id, &supervisor_id, &statuses],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(read)
        .collect()
}

/// One employment of the company at any status, or `None`.
pub(crate) async fn find(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    id: i64,
) -> Result<Option<Row_>> {
    client
        .query_opt(
            &format!("SELECT {COLUMNS} {FROM} WHERE e.tenant_id = $1 AND e.company_id = $2 AND e.id = $3"),
            &[&tenant, &company_id, &id],
        )
        .await
        .map_err(Error::from)?
        .as_ref()
        .map(read)
        .transpose()
}

/// The company's employment holding one affiliation, in the given statuses.
pub(crate) async fn by_affiliation(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    affiliation_id: i64,
    statuses: &[String],
) -> Result<Option<Row_>> {
    client
        .query_opt(
            &format!("SELECT {COLUMNS} {FROM} WHERE e.tenant_id = $1 AND e.company_id = $2 AND e.affiliation_id = $3 AND e.status = ANY($4::text[])"),
            &[&tenant, &company_id, &affiliation_id, &statuses],
        )
        .await
        .map_err(Error::from)?
        .as_ref()
        .map(read)
        .transpose()
}

/// One employment of the company at any status, locked for the rest of the
/// transaction (`FOR NO KEY UPDATE`: a concurrent change or self-service
/// write waits; a read does not), or `None`. The supervisor join is left out
/// so the lock lands on this row alone.
pub(crate) async fn lock(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    id: i64,
) -> Result<Option<Employment>> {
    let locked = client
        .query_opt(
            "SELECT id FROM people_workforce_employments WHERE tenant_id = $1 AND company_id = $2 AND id = $3 FOR NO KEY UPDATE",
            &[&tenant, &company_id, &id],
        )
        .await
        .map_err(Error::from)?;
    if locked.is_none() {
        return Ok(None);
    }
    Ok(find(client, tenant, company_id, id)
        .await?
        .map(|row| row.employment))
}

/// The employment's working supervisors, nearest first, at most
/// `CHAIN_DEPTH` levels; `None` when the employee is not a working employee
/// of the company. A supervisor outside the working statuses ends the chain,
/// as the reads hide such a supervisor.
pub(crate) async fn supervisor_chain(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    employee_id: i64,
    statuses: &[String],
) -> Result<Option<Vec<i64>>> {
    let rows = client
        .query(
            "WITH RECURSIVE chain(id, depth) AS ( \
               SELECT s.id, 1 FROM people_workforce_employments e \
                 JOIN people_workforce_employments s ON s.id = e.supervisor_employment_id AND s.company_id = e.company_id \
                WHERE e.tenant_id = $1 AND e.company_id = $2 AND e.id = $3 \
                  AND e.status = ANY($4::text[]) AND s.status = ANY($4::text[]) \
               UNION ALL \
               SELECT s.id, chain.depth + 1 FROM chain \
                 JOIN people_workforce_employments e ON e.id = chain.id \
                 JOIN people_workforce_employments s ON s.id = e.supervisor_employment_id AND s.company_id = e.company_id \
                WHERE chain.depth < $5 AND s.status = ANY($4::text[]) \
             ) SELECT id FROM chain ORDER BY depth",
            &[&tenant, &company_id, &employee_id, &statuses, &CHAIN_DEPTH],
        )
        .await
        .map_err(Error::from)?;
    let mut chain: Vec<i64> = Vec::with_capacity(rows.len());
    for row in &rows {
        let id: i64 = row.get(0);
        if id != employee_id && !chain.contains(&id) {
            chain.push(id);
        }
    }
    if chain.is_empty() {
        // No supervisor, or not a working employee: the reads tell which.
        let working = by_ids(client, tenant, company_id, &[employee_id], statuses).await?;
        if working.is_empty() {
            return Ok(None);
        }
    }
    Ok(Some(chain))
}

/// Whether `supervisor_id` reports to `employee_id`, directly or through
/// others, at any status: making it the employee's supervisor would close a
/// cycle. Bounded, so a chain already damaged by hand cannot loop.
async fn reports_to(
    client: &impl GenericClient,
    company_id: i64,
    supervisor_id: i64,
    employee_id: i64,
) -> Result<bool> {
    let found = client
        .query_opt(
            "WITH RECURSIVE chain(id, depth) AS ( \
               SELECT supervisor_employment_id, 1 FROM people_workforce_employments WHERE id = $1 AND company_id = $2 \
               UNION ALL \
               SELECT e.supervisor_employment_id, chain.depth + 1 FROM chain \
                 JOIN people_workforce_employments e ON e.id = chain.id AND e.company_id = $2 \
                WHERE chain.depth < 64 AND chain.id IS NOT NULL \
             ) SELECT 1 FROM chain WHERE id = $3 LIMIT 1",
            &[&supervisor_id, &company_id, &employee_id],
        )
        .await
        .map_err(Error::from)?;
    Ok(found.is_some())
}

/// Counts by status, every status listed, plus the working total.
pub(crate) async fn summary(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    working: &[String],
) -> Result<EmploymentSummary> {
    let rows = client
        .query(
            "SELECT status, count(*) FROM people_workforce_employments WHERE tenant_id = $1 AND company_id = $2 GROUP BY status",
            &[&tenant, &company_id],
        )
        .await
        .map_err(Error::from)?;
    let counted: Vec<(String, i64)> = rows
        .iter()
        .map(|row| (row.get::<_, String>(0), row.get::<_, i64>(1)))
        .collect();
    let by_status: Vec<StatusCount> = EMPLOYEE_STATUSES
        .iter()
        .map(|status| StatusCount {
            status: (*status).to_owned(),
            count: counted
                .iter()
                .find(|(counted, _)| counted == status)
                .map_or(0, |(_, count)| *count),
        })
        .collect();
    Ok(EmploymentSummary {
        total: by_status.iter().map(|count| count.count).sum(),
        working: by_status
            .iter()
            .filter(|count| working.contains(&count.status))
            .map(|count| count.count)
            .sum(),
        by_status,
    })
}

/// The facts an employment carries once validated: texts trimmed, dates
/// in order, ids positive, the status one of the five.
pub(crate) struct Valid {
    pub employee_number: String,
    pub employment_type: String,
    pub status: String,
    pub employment_start: Option<Date>,
    pub employment_end: Option<Date>,
    pub supervisor_employment_id: Option<i64>,
    pub department_id: Option<i64>,
    pub metadata: Option<Value>,
}

/// Bilimbi `EmploymentRecord.update_changeset/2`, in its words, over the
/// facts of one row.
fn check_facts(facts: &Valid, errors: &mut Vec<FieldError>) {
    if facts.employee_number.is_empty() {
        errors.push(FieldError::new("employee_number", "can't be blank"));
    } else if facts.employee_number.chars().count() > 255 {
        errors.push(FieldError::new(
            "employee_number",
            "should be at most 255 character(s)",
        ));
    }
    if facts.employment_type.is_empty() {
        errors.push(FieldError::new("employment_type", "can't be blank"));
    } else if facts.employment_type.chars().count() > 255 {
        errors.push(FieldError::new(
            "employment_type",
            "should be at most 255 character(s)",
        ));
    }
    if facts.status.is_empty() {
        errors.push(FieldError::new("status", "can't be blank"));
    } else if !EMPLOYEE_STATUSES.contains(&facts.status.as_str()) {
        errors.push(FieldError::new("status", "is invalid"));
    }
    if let (Some(start), Some(end)) = (facts.employment_start, facts.employment_end) {
        if end < start {
            errors.push(FieldError::new(
                "employment_end",
                "must be on or after employment start",
            ));
        }
    }
    if facts.supervisor_employment_id.is_some_and(|id| id <= 0) {
        errors.push(FieldError::new("supervisor_employment_id", "is invalid"));
    }
    if facts.department_id.is_some_and(|id| id <= 0) {
        errors.push(FieldError::new("department_id", "is invalid"));
    }
}

/// A new employment's facts, or the field errors a form shows.
pub(crate) fn validate(new: &NewEmployment) -> std::result::Result<Valid, Vec<FieldError>> {
    let mut errors = Vec::new();
    let valid = Valid {
        employee_number: new.employee_number.trim().to_owned(),
        employment_type: new.employment_type.trim().to_owned(),
        status: new.status.trim().to_owned(),
        employment_start: new.employment_start,
        employment_end: new.employment_end,
        supervisor_employment_id: new.supervisor_employment_id,
        department_id: new.department_id,
        metadata: new.metadata.clone(),
    };
    check_facts(&valid, &mut errors);
    if new.person_id <= 0 {
        errors.push(FieldError::new("person_id", "is invalid"));
    }
    if new.affiliation_id <= 0 {
        errors.push(FieldError::new("affiliation_id", "is invalid"));
    }
    if errors.is_empty() {
        Ok(valid)
    } else {
        Err(errors)
    }
}

/// The row as it will be once `change` is applied to `current`, validated
/// as a whole, or the field errors a form shows.
pub(crate) fn validate_change(
    current: &Employment,
    change: &EmploymentChange,
) -> std::result::Result<Valid, Vec<FieldError>> {
    let mut errors = Vec::new();
    let valid = Valid {
        employee_number: change
            .employee_number
            .as_deref()
            .map_or(current.employee_number.as_str(), str::trim)
            .to_owned(),
        employment_type: change
            .employment_type
            .as_deref()
            .map_or(current.employment_type.as_str(), str::trim)
            .to_owned(),
        status: change
            .status
            .as_deref()
            .map_or(current.status.as_str(), str::trim)
            .to_owned(),
        employment_start: change.employment_start.unwrap_or(current.employment_start),
        employment_end: change.employment_end.unwrap_or(current.employment_end),
        supervisor_employment_id: change
            .supervisor_employment_id
            .unwrap_or(current.supervisor_employment_id),
        department_id: change.department_id.unwrap_or(current.department_id),
        metadata: change
            .metadata
            .clone()
            .unwrap_or_else(|| current.metadata.clone()),
    };
    check_facts(&valid, &mut errors);
    if valid.supervisor_employment_id == Some(current.id) {
        errors.push(FieldError::new(
            "supervisor_employment_id",
            "cannot be the employee themselves",
        ));
    }
    if errors.is_empty() {
        Ok(valid)
    } else {
        Err(errors)
    }
}

/// A taken number (other than `except`'s own), a supervisor who is not a
/// working employee of the company, and a department that is not the
/// company's, as field errors.
async fn check_references(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    except: Option<i64>,
    valid: &Valid,
    statuses: &[String],
    errors: &mut Vec<FieldError>,
) -> Result<()> {
    let number = client
        .query_opt(
            "SELECT 1 FROM people_workforce_employments WHERE company_id = $1 AND employee_number = $2 AND id IS DISTINCT FROM $3",
            &[&company_id, &valid.employee_number, &except],
        )
        .await
        .map_err(Error::from)?;
    if number.is_some() {
        errors.push(FieldError::new("employee_number", "has already been taken"));
    }
    if let Some(supervisor) = valid.supervisor_employment_id {
        match find(client, tenant, company_id, supervisor).await? {
            None => errors.push(FieldError::new("supervisor_employment_id", "is invalid")),
            Some(row) if !statuses.contains(&row.employment.status) => {
                errors.push(FieldError::new(
                    "supervisor_employment_id",
                    "must be a working employee of the company",
                ));
            }
            Some(_) => {}
        }
    }
    if let Some(department) = valid.department_id {
        // Core Company's departments table, which the row's foreign key
        // already names; a bare existence check bound to this company.
        let found = client
            .query_opt(
                "SELECT 1 FROM company_departments WHERE tenant_id = $1 AND company_id = $2 AND id = $3",
                &[&tenant, &company_id, &department],
            )
            .await
            .map_err(Error::from)?;
        if found.is_none() {
            errors.push(FieldError::new("department_id", "is invalid"));
        }
    }
    Ok(())
}

/// The refusals decided on the transaction before the insert: an
/// affiliation already employed, then the reference checks.
pub(crate) async fn check_new(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    affiliation_id: i64,
    valid: &Valid,
    statuses: &[String],
) -> Result<()> {
    let taken = client
        .query_opt(
            "SELECT 1 FROM people_workforce_employments WHERE affiliation_id = $1",
            &[&affiliation_id],
        )
        .await
        .map_err(Error::from)?;
    if taken.is_some() {
        return Err(WorkforceError::DuplicateAffiliation);
    }
    let mut errors = Vec::new();
    check_references(
        client,
        tenant,
        company_id,
        None,
        valid,
        statuses,
        &mut errors,
    )
    .await?;
    if errors.is_empty() {
        Ok(())
    } else {
        Err(WorkforceError::Invalid(errors))
    }
}

/// The refusals decided on the transaction before the update, under the
/// row's lock: the reference checks and a supervisor who reports to this
/// employee.
pub(crate) async fn check_change(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    current: &Employment,
    valid: &Valid,
    statuses: &[String],
) -> Result<()> {
    let mut errors = Vec::new();
    check_references(
        client,
        tenant,
        company_id,
        Some(current.id),
        valid,
        statuses,
        &mut errors,
    )
    .await?;
    if let Some(supervisor) = valid.supervisor_employment_id {
        if errors.is_empty() && reports_to(client, company_id, supervisor, current.id).await? {
            errors.push(FieldError::new(
                "supervisor_employment_id",
                "would create a reporting cycle",
            ));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(WorkforceError::Invalid(errors))
    }
}

/// Inserts the row through the mutation boundary, recorded as a created
/// `people_workforce_employment` change.
pub(crate) async fn insert(
    tx: &mut MutationTransaction<'_>,
    tenant: i64,
    company_id: i64,
    person_id: i64,
    affiliation_id: i64,
    valid: Valid,
) -> Result<Employment> {
    let row = tx
        .mutate(move |sql| {
            Box::pin(async move {
                let start = valid.employment_start.map(|date| date.to_string());
                let end = valid.employment_end.map(|date| date.to_string());
                let row = sql
                    .query_one(
                        "INSERT INTO people_workforce_employments \
                         (tenant_id, company_id, person_id, affiliation_id, employee_number, employment_type, status, \
                          employment_start, employment_end, supervisor_employment_id, department_id, metadata) \
                         VALUES ($1, $2, $3, $4, $5, $6, $7, $8::text::date, $9::text::date, $10, $11, $12) \
                         RETURNING id",
                        &[
                            &tenant,
                            &company_id,
                            &person_id,
                            &affiliation_id,
                            &valid.employee_number,
                            &valid.employment_type,
                            &valid.status,
                            &start,
                            &end,
                            &valid.supervisor_employment_id,
                            &valid.department_id,
                            &valid.metadata,
                        ],
                    )
                    .await?;
                let id: i64 = row.get(0);
                let row = sql
                    .query_one(
                        &format!("SELECT {COLUMNS} {FROM} WHERE e.id = $1"),
                        &[&id],
                    )
                    .await?;
                let employment = read(&row).map_err(|_| Error::Internal)?.employment;
                // An empty object on the absent side is what names the audit
                // event `created`.
                let change = Change {
                    kind: "people_workforce_employment".into(),
                    id,
                    old: json!({}),
                    new: employment.audit_values(),
                };
                Ok((employment, vec![change]))
            })
        })
        .await?;
    Ok(row)
}

/// Replaces the locked row's facts through the mutation boundary, moving
/// the version on, recorded as an updated `people_workforce_employment`
/// change with the old and new values.
pub(crate) async fn update(
    tx: &mut MutationTransaction<'_>,
    current: Employment,
    valid: Valid,
) -> Result<Employment> {
    let row = tx
        .mutate(move |sql| {
            Box::pin(async move {
                let start = valid.employment_start.map(|date| date.to_string());
                let end = valid.employment_end.map(|date| date.to_string());
                let updated = sql
                    .execute(
                        "UPDATE people_workforce_employments SET employee_number = $2, employment_type = $3, status = $4, \
                         employment_start = $5::text::date, employment_end = $6::text::date, supervisor_employment_id = $7, \
                         department_id = $8, metadata = $9, version = version + 1, updated_at = (now() AT TIME ZONE 'UTC') \
                         WHERE id = $1 AND version = $10",
                        &[
                            &current.id,
                            &valid.employee_number,
                            &valid.employment_type,
                            &valid.status,
                            &start,
                            &end,
                            &valid.supervisor_employment_id,
                            &valid.department_id,
                            &valid.metadata,
                            &current.version,
                        ],
                    )
                    .await?;
                if updated != 1 {
                    // The row is locked, so the fence cannot move; a miss
                    // here is a failure, not a refusal.
                    return Err(Error::Internal);
                }
                let row = sql
                    .query_one(
                        &format!("SELECT {COLUMNS} {FROM} WHERE e.id = $1"),
                        &[&current.id],
                    )
                    .await?;
                let employment = read(&row).map_err(|_| Error::Internal)?.employment;
                let change = Change {
                    kind: "people_workforce_employment".into(),
                    id: current.id,
                    old: current.audit_values(),
                    new: employment.audit_values(),
                };
                Ok((employment, vec![change]))
            })
        })
        .await?;
    Ok(row)
}

impl Employment {
    /// The row as audit records it.
    pub(crate) fn audit_values(&self) -> Value {
        json!({
            "company_id": self.platform_company_id,
            "person_id": self.person_id,
            "affiliation_id": self.affiliation_id,
            "employee_number": self.employee_number,
            "employment_type": self.employment_type,
            "status": self.status,
            "employment_start": self.employment_start.map(|date| date.to_string()),
            "employment_end": self.employment_end.map(|date| date.to_string()),
            "supervisor_employment_id": self.supervisor_employment_id,
            "department_id": self.department_id,
            "metadata": self.metadata,
            "version": self.version,
        })
    }
}

/// `people/native` references: a company, an employee or a position by its
/// immutable native id.
impl Reference {
    pub(crate) fn company(id: i64) -> Self {
        Self::new(crate::ReferenceType::Company, id)
    }
    pub(crate) fn employee(id: i64) -> Self {
        Self::new(crate::ReferenceType::Employee, id)
    }
    fn new(kind: crate::ReferenceType, id: i64) -> Self {
        Self {
            source_id: crate::SOURCE_ID.to_owned(),
            kind,
            stable_id: id.to_string(),
        }
    }
}

/// Minimal (affiliation, employment) identity links, without private HR facts.
pub(crate) async fn affiliation_links(
    client: &impl GenericClient,
    tenant: i64,
    company: i64,
    affiliations: &[i64],
) -> Result<Vec<(i64, i64)>> {
    Ok(client.query(
        "SELECT affiliation_id,id FROM people_workforce_employments WHERE tenant_id=$1 AND company_id=$2 AND affiliation_id=ANY($3::bigint[]) ORDER BY id",
        &[&tenant,&company,&affiliations],
    ).await.map_err(Error::from)?.iter().map(|r| (r.get(0),r.get(1))).collect())
}
