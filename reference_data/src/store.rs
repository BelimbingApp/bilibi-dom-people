//! The three `people_reference_data_*` tables: their rows, the SQL that reads
//! them and the validation and inserts behind the facade's writes (Bilimbi
//! `Entry`, `Alias` and `CalendarException` changesets). Private to the facade.
use crate::{Alias, CalendarException, Entry, NewCalendarException, NewEntry};
use bilibi_database::{Change, Error, MutationTransaction};
use bilibi_people_workforce::{FieldError, Result, WorkforceError};
use jiff::civil::Date;
use serde_json::json;
use tokio_postgres::{GenericClient, Row};

/// The longest each field may be, as Bilimbi's changesets bound them.
const KIND_MAX: usize = 80;
const CODE_MAX: usize = 100;
const LABEL_MAX: usize = 200;

/// What a validated entry carries to its insert.
pub(crate) struct ValidEntry {
    kind: String,
    code: String,
    label: String,
    active: bool,
}
/// What a validated calendar exception carries to its insert.
pub(crate) struct ValidException {
    on_date: Date,
    label: String,
}

/// A trimmed, required, bounded text field, in the words an Ecto changeset
/// gives (`can't be blank`, `should be at most N character(s)`).
fn text(
    errors: &mut Vec<FieldError>,
    field: &'static str,
    value: &str,
    max: usize,
) -> String {
    let value = value.trim().to_owned();
    if value.is_empty() {
        errors.push(FieldError::new(field, "can't be blank"));
    } else if value.chars().count() > max {
        errors.push(FieldError::new(
            field,
            format!("should be at most {max} character(s)"),
        ));
    }
    value
}

pub(crate) fn validate_entry(new: &NewEntry) -> Result<ValidEntry> {
    let mut errors = Vec::new();
    let kind = text(&mut errors, "kind", &new.kind, KIND_MAX);
    let code = text(&mut errors, "code", &new.code, CODE_MAX);
    let label = text(&mut errors, "label", &new.label, LABEL_MAX);
    if !errors.is_empty() {
        return Err(WorkforceError::Invalid(errors));
    }
    Ok(ValidEntry {
        kind,
        code,
        label,
        active: new.active,
    })
}

pub(crate) fn validate_alias(label: &str) -> Result<String> {
    let mut errors = Vec::new();
    let label = text(&mut errors, "label", label, LABEL_MAX);
    if !errors.is_empty() {
        return Err(WorkforceError::Invalid(errors));
    }
    Ok(label)
}

pub(crate) fn validate_exception(new: &NewCalendarException) -> Result<ValidException> {
    let mut errors = Vec::new();
    let label = text(&mut errors, "label", &new.label, LABEL_MAX);
    if !errors.is_empty() {
        return Err(WorkforceError::Invalid(errors));
    }
    Ok(ValidException {
        on_date: new.on_date,
        label,
    })
}

fn invalid(field: &'static str, message: &str) -> WorkforceError {
    WorkforceError::Invalid(vec![FieldError::new(field, message)])
}

fn date(row: &Row, column: &str) -> Result<Date> {
    row.get::<_, String>(column)
        .parse::<Date>()
        .map_err(|_| Error::Internal.into())
}

fn entry(row: &Row) -> Entry {
    Entry {
        id: row.get("id"),
        kind: row.get("kind"),
        code: row.get("code"),
        label: row.get("label"),
        active: row.get("active"),
    }
}

fn alias(row: &Row) -> Alias {
    Alias {
        id: row.get("id"),
        entry_id: row.get("entry_id"),
        label: row.get("label"),
    }
}

fn exception(row: &Row) -> Result<CalendarException> {
    Ok(CalendarException {
        id: row.get("id"),
        on_date: date(row, "on_date")?,
        label: row.get("label"),
    })
}

const ENTRY_COLUMNS: &str = "id, kind, code, label, active";
const ALIAS_COLUMNS: &str = "id, entry_id, label";
/// Dates are ISO text: the workspace's `tokio-postgres` has no date feature,
/// and a Domain must not turn one on.
const EXCEPTION_COLUMNS: &str = "id, to_char(on_date, 'YYYY-MM-DD') AS on_date, label";

/// A company's entries ordered by kind, label and id, narrowed to one kind
/// when given (Bilimbi `list_entries/2`).
pub(crate) async fn entries(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    kind: Option<&str>,
) -> Result<Vec<Entry>> {
    let rows = client
        .query(
            &format!(
                "SELECT {ENTRY_COLUMNS} FROM people_reference_data_entries \
                 WHERE tenant_id = $1 AND company_id = $2 AND ($3::text IS NULL OR kind = $3) \
                 ORDER BY kind, label, id"
            ),
            &[&tenant, &company_id, &kind],
        )
        .await
        .map_err(Error::from)?;
    Ok(rows.iter().map(entry).collect())
}

/// A company's aliases ordered by entry and label (Bilimbi `list_aliases/2`).
pub(crate) async fn aliases(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
) -> Result<Vec<Alias>> {
    let rows = client
        .query(
            &format!(
                "SELECT {ALIAS_COLUMNS} FROM people_reference_data_aliases \
                 WHERE tenant_id = $1 AND company_id = $2 ORDER BY entry_id, label"
            ),
            &[&tenant, &company_id],
        )
        .await
        .map_err(Error::from)?;
    Ok(rows.iter().map(alias).collect())
}

/// A company's calendar exceptions ordered by date and id (Bilimbi
/// `list_calendar_exceptions/2`).
pub(crate) async fn exceptions(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
) -> Result<Vec<CalendarException>> {
    let rows = client
        .query(
            &format!(
                "SELECT {EXCEPTION_COLUMNS} FROM people_reference_data_calendar_overrides \
                 WHERE tenant_id = $1 AND company_id = $2 ORDER BY on_date, id"
            ),
            &[&tenant, &company_id],
        )
        .await
        .map_err(Error::from)?;
    rows.iter().map(exception).collect()
}

/// An entry's kind, when the entry belongs to the company; the alias copies it.
pub(crate) async fn entry_kind(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    entry_id: i64,
) -> Result<Option<String>> {
    let row = client
        .query_opt(
            "SELECT kind FROM people_reference_data_entries \
             WHERE tenant_id = $1 AND company_id = $2 AND id = $3",
            &[&tenant, &company_id, &entry_id],
        )
        .await
        .map_err(Error::from)?;
    Ok(row.map(|row| row.get(0)))
}

pub(crate) async fn insert_entry(
    tx: &mut MutationTransaction<'_>,
    tenant: i64,
    company_id: i64,
    valid: ValidEntry,
) -> Result<Entry> {
    let taken = tx
        .sql()
        .query_opt(
            "SELECT 1 FROM people_reference_data_entries \
             WHERE company_id = $1 AND kind = $2 AND code = $3",
            &[&company_id, &valid.kind, &valid.code],
        )
        .await
        .map_err(Error::from)?;
    if taken.is_some() {
        return Err(invalid("code", "has already been taken"));
    }
    let entry = tx
        .mutate(move |sql| {
            Box::pin(async move {
                let row = sql
                    .query_one(
                        &format!(
                            "INSERT INTO people_reference_data_entries \
                             (tenant_id, company_id, kind, code, label, active) \
                             VALUES ($1, $2, $3, $4, $5, $6) RETURNING {ENTRY_COLUMNS}"
                        ),
                        &[
                            &tenant,
                            &company_id,
                            &valid.kind,
                            &valid.code,
                            &valid.label,
                            &valid.active,
                        ],
                    )
                    .await?;
                let entry = entry(&row);
                // An empty object on the absent side is what names the audit
                // event `created`.
                let change = Change {
                    kind: "people_reference_data_entry".into(),
                    id: entry.id,
                    old: json!({}),
                    new: json!({
                        "company_id": company_id,
                        "kind": entry.kind,
                        "code": entry.code,
                        "label": entry.label,
                        "active": entry.active,
                    }),
                };
                Ok((entry, vec![change]))
            })
        })
        .await?;
    Ok(entry)
}

pub(crate) async fn insert_alias(
    tx: &mut MutationTransaction<'_>,
    tenant: i64,
    company_id: i64,
    entry_id: i64,
    label: String,
) -> Result<Alias> {
    let Some(kind) = entry_kind(tx.sql(), tenant, company_id, entry_id).await? else {
        return Err(invalid("entry_id", "is invalid"));
    };
    let taken = tx
        .sql()
        .query_opt(
            "SELECT 1 FROM people_reference_data_aliases \
             WHERE company_id = $1 AND kind = $2 AND label = $3",
            &[&company_id, &kind, &label],
        )
        .await
        .map_err(Error::from)?;
    if taken.is_some() {
        return Err(invalid("label", "has already been taken"));
    }
    let alias = tx
        .mutate(move |sql| {
            Box::pin(async move {
                let row = sql
                    .query_one(
                        &format!(
                            "INSERT INTO people_reference_data_aliases \
                             (tenant_id, company_id, entry_id, kind, label) \
                             VALUES ($1, $2, $3, $4, $5) RETURNING {ALIAS_COLUMNS}"
                        ),
                        &[&tenant, &company_id, &entry_id, &kind, &label],
                    )
                    .await?;
                let alias = alias(&row);
                let change = Change {
                    kind: "people_reference_data_alias".into(),
                    id: alias.id,
                    old: json!({}),
                    new: json!({
                        "company_id": company_id,
                        "entry_id": alias.entry_id,
                        "kind": kind,
                        "label": alias.label,
                    }),
                };
                Ok((alias, vec![change]))
            })
        })
        .await?;
    Ok(alias)
}

pub(crate) async fn insert_exception(
    tx: &mut MutationTransaction<'_>,
    tenant: i64,
    company_id: i64,
    valid: ValidException,
) -> Result<CalendarException> {
    let on_date = valid.on_date.to_string();
    let taken = tx
        .sql()
        .query_opt(
            "SELECT 1 FROM people_reference_data_calendar_overrides \
             WHERE company_id = $1 AND on_date = $2::text::date AND label = $3",
            &[&company_id, &on_date, &valid.label],
        )
        .await
        .map_err(Error::from)?;
    if taken.is_some() {
        return Err(invalid("label", "has already been taken"));
    }
    let exception = tx
        .mutate(move |sql| {
            Box::pin(async move {
                let row = sql
                    .query_one(
                        &format!(
                            "INSERT INTO people_reference_data_calendar_overrides \
                             (tenant_id, company_id, on_date, label) \
                             VALUES ($1, $2, $3::text::date, $4) RETURNING {EXCEPTION_COLUMNS}"
                        ),
                        &[&tenant, &company_id, &on_date, &valid.label],
                    )
                    .await?;
                let exception = exception(&row).map_err(|_| Error::Internal)?;
                let change = Change {
                    kind: "people_reference_data_calendar_exception".into(),
                    id: exception.id,
                    old: json!({}),
                    new: json!({
                        "company_id": company_id,
                        "on_date": exception.on_date.to_string(),
                        "label": exception.label,
                    }),
                };
                Ok((exception, vec![change]))
            })
        })
        .await?;
    Ok(exception)
}
