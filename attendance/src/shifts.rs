//! Shift templates (Bilimbi `Bilimbi.People.Attendance.ShiftTemplate` and
//! the template functions of `Rosters`): the
//! `people_attendance_shift_definitions` table. Private to the facade.
use crate::{AttendanceError, FieldError, Result};
use bilibi_database::{Change, Error, MutationTransaction};
use bilibi_tenancy::Scope;
use jiff::civil::Time;
use serde::Serialize;
use serde_json::{json, Value};
use tokio_postgres::{GenericClient, Row};

/// One shift template: start and end as minutes past local midnight in the
/// company's attendance time zone; an end at or before the start crosses
/// midnight.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ShiftTemplate {
    pub id: i64,
    pub code: String,
    pub name: String,
    pub start_minute: i32,
    pub end_minute: i32,
    pub break_minutes: i32,
    pub status: String,
}
impl ShiftTemplate {
    /// Scheduled minutes; an end at or before the start crosses midnight.
    pub fn span_minutes(&self) -> i32 {
        span_minutes(self.start_minute, self.end_minute)
    }
    /// The start as `HH:MM`.
    pub fn starts_at(&self) -> String {
        clock(self.start_minute)
    }
    /// The end as `HH:MM`.
    pub fn ends_at(&self) -> String {
        clock(self.end_minute)
    }
    fn audit_values(&self) -> Value {
        json!({
            "code": self.code,
            "name": self.name,
            "start_minute": self.start_minute,
            "end_minute": self.end_minute,
            "break_minutes": self.break_minutes,
            "status": self.status,
        })
    }
}

/// `HH:MM` for a minute past midnight.
pub fn clock(minute: i32) -> String {
    format!("{:02}:{:02}", minute.div_euclid(60), minute.rem_euclid(60))
}

fn span_minutes(start_minute: i32, end_minute: i32) -> i32 {
    let minutes = end_minute - start_minute;
    if minutes > 0 {
        minutes
    } else {
        minutes + 24 * 60
    }
}

/// What adds a shift template: a code and name, local start and end times
/// and break minutes.
#[derive(Clone, Debug)]
pub struct NewShiftTemplate {
    pub code: String,
    pub name: String,
    pub starts_at: Time,
    pub ends_at: Time,
    pub break_minutes: i32,
}

/// The facts once validated.
pub(crate) struct Valid {
    pub code: String,
    pub name: String,
    pub start_minute: i32,
    pub end_minute: i32,
    pub break_minutes: i32,
}

/// Bilimbi's code rule: `^[A-Za-z0-9][A-Za-z0-9_.-]*$`.
pub(crate) fn code_format(code: &str) -> bool {
    let mut chars = code.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// Trimmed code (1 to 40, in Bilimbi's format) and name (1 to 120), in a
/// changeset's words.
pub(crate) fn code_and_name(
    code: &str,
    name: &str,
    errors: &mut Vec<FieldError>,
) -> (String, String) {
    let code = code.trim().to_owned();
    if code.is_empty() {
        errors.push(FieldError::new("code", "can't be blank"));
    } else if code.chars().count() > 40 {
        errors.push(FieldError::new("code", "should be at most 40 character(s)"));
    } else if !code_format(&code) {
        errors.push(FieldError::new("code", "has invalid format"));
    }
    let name = name.trim().to_owned();
    if name.is_empty() {
        errors.push(FieldError::new("name", "can't be blank"));
    } else if name.chars().count() > 120 {
        errors.push(FieldError::new(
            "name",
            "should be at most 120 character(s)",
        ));
    }
    (code, name)
}

/// Bilimbi `ShiftTemplate.changeset/2`, in its words.
pub(crate) fn validate(new: &NewShiftTemplate) -> std::result::Result<Valid, Vec<FieldError>> {
    let mut errors = Vec::new();
    let (code, name) = code_and_name(&new.code, &new.name, &mut errors);
    let start_minute = i32::from(new.starts_at.hour()) * 60 + i32::from(new.starts_at.minute());
    let end_minute = i32::from(new.ends_at.hour()) * 60 + i32::from(new.ends_at.minute());
    if new.break_minutes < 0 {
        errors.push(FieldError::new(
            "break_minutes",
            "must be greater than or equal to 0",
        ));
    } else if start_minute == end_minute {
        errors.push(FieldError::new("ends_at", "must differ from the start"));
    } else if new.break_minutes >= span_minutes(start_minute, end_minute) {
        errors.push(FieldError::new(
            "break_minutes",
            "must be shorter than the shift",
        ));
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(Valid {
        code,
        name,
        start_minute,
        end_minute,
        break_minutes: new.break_minutes,
    })
}

const COLUMNS: &str = "id, code, name, start_minute, end_minute, break_minutes, status";

fn read(row: &Row) -> ShiftTemplate {
    ShiftTemplate {
        id: row.get("id"),
        code: row.get("code"),
        name: row.get("name"),
        start_minute: row.get("start_minute"),
        end_minute: row.get("end_minute"),
        break_minutes: row.get("break_minutes"),
        status: row.get("status"),
    }
}

/// The company's templates, active first then by code.
pub(crate) async fn list(
    client: &impl GenericClient,
    scope: &Scope,
    company_id: i64,
) -> Result<Vec<ShiftTemplate>> {
    Ok(client
        .query(
            &format!("SELECT {COLUMNS} FROM people_attendance_shift_definitions WHERE tenant_id = $1 AND company_id = $2 ORDER BY status, code"),
            &[&scope.tenant(), &company_id],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(read)
        .collect())
}

/// A taken code, decided on the transaction before the insert.
pub(crate) async fn check_code(
    client: &impl GenericClient,
    company_id: i64,
    code: &str,
) -> Result<()> {
    let taken = client
        .query_opt(
            "SELECT 1 FROM people_attendance_shift_definitions WHERE company_id = $1 AND code = $2",
            &[&company_id, &code],
        )
        .await
        .map_err(Error::from)?;
    if taken.is_some() {
        return Err(AttendanceError::Invalid(vec![FieldError::new(
            "code",
            "has already been taken",
        )]));
    }
    Ok(())
}

/// Inserts the template, recorded as a created change.
pub(crate) async fn insert(
    tx: &mut MutationTransaction<'_>,
    tenant: i64,
    company_id: i64,
    valid: Valid,
) -> Result<ShiftTemplate> {
    tx.mutate(move |sql| {
        Box::pin(async move {
            let row = sql
                .query_one(
                    &format!("INSERT INTO people_attendance_shift_definitions (tenant_id, company_id, code, name, start_minute, end_minute, break_minutes, status) \
                              VALUES ($1, $2, $3, $4, $5, $6, $7, 'active') RETURNING {COLUMNS}"),
                    &[
                        &tenant,
                        &company_id,
                        &valid.code,
                        &valid.name,
                        &valid.start_minute,
                        &valid.end_minute,
                        &valid.break_minutes,
                    ],
                )
                .await?;
            let template = read(&row);
            let change = Change {
                kind: "people_attendance_shift_definition".into(),
                id: template.id,
                old: json!({}),
                new: template.audit_values(),
            };
            Ok((template, vec![change]))
        })
    })
    .await
    .map_err(AttendanceError::from)
}

/// Sets the template's status, recorded as an updated change; a template
/// of another company or missing is `NotFound`.
pub(crate) async fn set_status(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company_id: i64,
    template_id: i64,
    status: &'static str,
) -> Result<ShiftTemplate> {
    let current = tx
        .sql()
        .query_opt(
            &format!("SELECT {COLUMNS} FROM people_attendance_shift_definitions WHERE tenant_id = $1 AND company_id = $2 AND id = $3 FOR UPDATE"),
            &[&scope.tenant(), &company_id, &template_id],
        )
        .await
        .map_err(Error::from)?
        .as_ref()
        .map(read)
        .ok_or(AttendanceError::NotFound)?;
    tx.mutate(move |sql| {
        Box::pin(async move {
            let row = sql
                .query_one(
                    &format!("UPDATE people_attendance_shift_definitions SET status = $2, updated_at = now() WHERE id = $1 RETURNING {COLUMNS}"),
                    &[&current.id, &status],
                )
                .await?;
            let template = read(&row);
            let change = Change {
                kind: "people_attendance_shift_definition".into(),
                id: template.id,
                old: current.audit_values(),
                new: template.audit_values(),
            };
            Ok((template, vec![change]))
        })
    })
    .await
    .map_err(AttendanceError::from)
}
