//! The `people_workforce_employments` table: its rows, the SQL that reads
//! them and the validation and insert behind `Workforce::create_employment`.
//! Private to the facade.
use crate::{
    Employment, FieldError, NewEmployment, Reference, Result, WorkforceError, EMPLOYEE_STATUSES,
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

/// The employment holding one affiliation in the company, at any status,
/// or `None` (an affiliation is employed at most once).
pub(crate) async fn by_affiliation(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    affiliation_id: i64,
) -> Result<Option<Row_>> {
    client
        .query_opt(
            &format!("SELECT {COLUMNS} {FROM} WHERE e.tenant_id = $1 AND e.company_id = $2 AND e.affiliation_id = $3"),
            &[&tenant, &company_id, &affiliation_id],
        )
        .await
        .map_err(Error::from)?
        .as_ref()
        .map(read)
        .transpose()
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

/// The facts a new employment carries once validated: texts trimmed, dates
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

/// Bilimbi `EmploymentRecord.update_changeset/2`, in its words.
pub(crate) fn validate(new: &NewEmployment) -> std::result::Result<Valid, Vec<FieldError>> {
    let mut errors = Vec::new();
    let employee_number = new.employee_number.trim().to_owned();
    if employee_number.is_empty() {
        errors.push(FieldError::new("employee_number", "can't be blank"));
    } else if employee_number.chars().count() > 255 {
        errors.push(FieldError::new(
            "employee_number",
            "should be at most 255 character(s)",
        ));
    }
    let employment_type = new.employment_type.trim().to_owned();
    if employment_type.is_empty() {
        errors.push(FieldError::new("employment_type", "can't be blank"));
    } else if employment_type.chars().count() > 255 {
        errors.push(FieldError::new(
            "employment_type",
            "should be at most 255 character(s)",
        ));
    }
    let status = new.status.trim().to_owned();
    if status.is_empty() {
        errors.push(FieldError::new("status", "can't be blank"));
    } else if !EMPLOYEE_STATUSES.contains(&status.as_str()) {
        errors.push(FieldError::new("status", "is invalid"));
    }
    if let (Some(start), Some(end)) = (new.employment_start, new.employment_end) {
        if end < start {
            errors.push(FieldError::new(
                "employment_end",
                "must be on or after employment start",
            ));
        }
    }
    if new.person_id <= 0 {
        errors.push(FieldError::new("person_id", "is invalid"));
    }
    if new.affiliation_id <= 0 {
        errors.push(FieldError::new("affiliation_id", "is invalid"));
    }
    if new.supervisor_employment_id.is_some_and(|id| id <= 0) {
        errors.push(FieldError::new("supervisor_employment_id", "is invalid"));
    }
    if new.department_id.is_some_and(|id| id <= 0) {
        errors.push(FieldError::new("department_id", "is invalid"));
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(Valid {
        employee_number,
        employment_type,
        status,
        employment_start: new.employment_start,
        employment_end: new.employment_end,
        supervisor_employment_id: new.supervisor_employment_id,
        department_id: new.department_id,
        metadata: new.metadata.clone(),
    })
}

/// The refusals decided on the transaction before the insert: a taken
/// number, an affiliation already employed, a supervisor who is not an
/// employment of the company.
pub(crate) async fn check_new(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    affiliation_id: i64,
    valid: &Valid,
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
    let number = client
        .query_opt(
            "SELECT 1 FROM people_workforce_employments WHERE company_id = $1 AND employee_number = $2",
            &[&company_id, &valid.employee_number],
        )
        .await
        .map_err(Error::from)?;
    if number.is_some() {
        errors.push(FieldError::new("employee_number", "has already been taken"));
    }
    if let Some(supervisor) = valid.supervisor_employment_id {
        if find(client, tenant, company_id, supervisor)
            .await?
            .is_none()
        {
            errors.push(FieldError::new("supervisor_employment_id", "is invalid"));
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
