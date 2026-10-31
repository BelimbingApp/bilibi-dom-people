//! The `people_leave_catalog_types` table: its rows, the SQL that reads and
//! locks them, and the validation and writes behind `Leave::create_type`
//! and `Leave::set_type_status`. Private to the facade.
use crate::{FieldError, LeaveType, NewLeaveType, Result, TYPE_STATUSES, UNITS};
use bilibi_database::{Change, Error, MutationTransaction};
use serde_json::{json, Value};
use tokio_postgres::{GenericClient, Row};

const COLUMNS: &str = "id, code, name, unit, paid, balance_required, status";

pub(crate) fn read(row: &Row) -> LeaveType {
    LeaveType {
        id: row.get("id"),
        code: row.get("code"),
        name: row.get("name"),
        unit: row.get("unit"),
        paid: row.get("paid"),
        balance_required: row.get("balance_required"),
        status: row.get("status"),
    }
}

/// The company's types as the Policies page lists them: active first, then
/// by name and id.
pub(crate) async fn list(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
) -> Result<Vec<LeaveType>> {
    Ok(client
        .query(
            &format!("SELECT {COLUMNS} FROM people_leave_catalog_types WHERE tenant_id = $1 AND company_id = $2 ORDER BY status, name, id"),
            &[&tenant, &company_id],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(read)
        .collect())
}

/// The company's types by name and id, for balances.
pub(crate) async fn by_name(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
) -> Result<Vec<LeaveType>> {
    Ok(client
        .query(
            &format!("SELECT {COLUMNS} FROM people_leave_catalog_types WHERE tenant_id = $1 AND company_id = $2 ORDER BY name, id"),
            &[&tenant, &company_id],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(read)
        .collect())
}

/// One type of the company at any status, locked for the transaction.
pub(crate) async fn lock(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    type_id: i64,
) -> Result<Option<LeaveType>> {
    Ok(client
        .query_opt(
            &format!("SELECT {COLUMNS} FROM people_leave_catalog_types WHERE tenant_id = $1 AND company_id = $2 AND id = $3 FOR UPDATE"),
            &[&tenant, &company_id, &type_id],
        )
        .await
        .map_err(Error::from)?
        .as_ref()
        .map(read))
}

/// One active type of the company, locked for the transaction; an archived
/// or missing type is `None`.
pub(crate) async fn lock_active(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    type_id: i64,
) -> Result<Option<LeaveType>> {
    Ok(client
        .query_opt(
            &format!("SELECT {COLUMNS} FROM people_leave_catalog_types WHERE tenant_id = $1 AND company_id = $2 AND id = $3 AND status = 'active' FOR UPDATE"),
            &[&tenant, &company_id, &type_id],
        )
        .await
        .map_err(Error::from)?
        .as_ref()
        .map(read))
}

/// Every type of the company by id, locked for the transaction: what a grant
/// run and a leave-year change serialize on.
pub(crate) async fn lock_all(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
) -> Result<Vec<LeaveType>> {
    Ok(client
        .query(
            &format!("SELECT {COLUMNS} FROM people_leave_catalog_types WHERE tenant_id = $1 AND company_id = $2 ORDER BY id FOR UPDATE"),
            &[&tenant, &company_id],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(read)
        .collect())
}

/// Whether the company already has a type with this code.
pub(crate) async fn code_taken(
    client: &(impl GenericClient + Sync),
    company_id: i64,
    code: &str,
) -> Result<bool> {
    Ok(client
        .query_opt(
            "SELECT 1 FROM people_leave_catalog_types WHERE company_id = $1 AND code = $2",
            &[&company_id, &code],
        )
        .await
        .map_err(Error::from)?
        .is_some())
}

/// A new type once validated: code and name trimmed, unit one of the two.
pub(crate) struct Valid {
    pub code: String,
    pub name: String,
    pub unit: String,
    pub paid: bool,
    pub balance_required: bool,
}

/// Bilimbi `LeaveType.changeset/2`, in its words: a code of 1 to 40
/// characters matching `^[a-z0-9][a-z0-9_-]*$`, a name of 1 to 120, a unit
/// of `day` or `hour`.
pub(crate) fn validate(new: &NewLeaveType) -> std::result::Result<Valid, Vec<FieldError>> {
    let mut errors = Vec::new();
    let code = new.code.trim().to_owned();
    if code.is_empty() {
        errors.push(FieldError::new("code", "can't be blank"));
    } else if code.chars().count() > 40 {
        errors.push(FieldError::new("code", "should be at most 40 character(s)"));
    } else if !code_format(&code) {
        errors.push(FieldError::new("code", "has invalid format"));
    }
    let name = new.name.trim().to_owned();
    if name.is_empty() {
        errors.push(FieldError::new("name", "can't be blank"));
    } else if name.chars().count() > 120 {
        errors.push(FieldError::new(
            "name",
            "should be at most 120 character(s)",
        ));
    }
    let unit = new.unit.trim().to_owned();
    if unit.is_empty() {
        errors.push(FieldError::new("unit", "can't be blank"));
    } else if !UNITS.contains(&unit.as_str()) {
        errors.push(FieldError::new("unit", "is invalid"));
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(Valid {
        code,
        name,
        unit,
        paid: new.paid,
        balance_required: new.balance_required,
    })
}

fn code_format(code: &str) -> bool {
    let mut bytes = code.bytes();
    bytes
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && bytes.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-')
}

pub(crate) fn status_valid(status: &str) -> bool {
    TYPE_STATUSES.contains(&status)
}

impl LeaveType {
    pub(crate) fn audit_values(&self) -> Value {
        json!({
            "code": self.code,
            "name": self.name,
            "unit": self.unit,
            "paid": self.paid,
            "balance_required": self.balance_required,
            "status": self.status,
        })
    }
}

/// Inserts an active type, recorded as a created `people_leave_catalog_type`.
pub(crate) async fn insert(
    tx: &mut MutationTransaction<'_>,
    tenant: i64,
    company_id: i64,
    valid: Valid,
) -> Result<LeaveType> {
    tx.mutate(move |sql| {
        Box::pin(async move {
            let row = sql
                .query_one(
                    &format!(
                        "INSERT INTO people_leave_catalog_types \
                         (tenant_id, company_id, code, name, unit, paid, balance_required, status) \
                         VALUES ($1, $2, $3, $4, $5, $6, $7, 'active') RETURNING {COLUMNS}"
                    ),
                    &[
                        &tenant,
                        &company_id,
                        &valid.code,
                        &valid.name,
                        &valid.unit,
                        &valid.paid,
                        &valid.balance_required,
                    ],
                )
                .await?;
            let leave_type = read(&row);
            let change = Change {
                kind: "people_leave_catalog_type".into(),
                id: leave_type.id,
                old: json!({}),
                new: leave_type.audit_values(),
            };
            Ok((leave_type, vec![change]))
        })
    })
    .await
    .map_err(Into::into)
}

/// Sets a type's status, recorded as an updated `people_leave_catalog_type`.
pub(crate) async fn set_status(
    tx: &mut MutationTransaction<'_>,
    before: LeaveType,
    status: String,
) -> Result<LeaveType> {
    tx.mutate(move |sql| {
        Box::pin(async move {
            let row = sql
                .query_one(
                    &format!(
                        "UPDATE people_leave_catalog_types SET status = $2, updated_at = now() AT TIME ZONE 'UTC' \
                         WHERE id = $1 RETURNING {COLUMNS}"
                    ),
                    &[&before.id, &status],
                )
                .await?;
            let after = read(&row);
            let change = Change {
                kind: "people_leave_catalog_type".into(),
                id: after.id,
                old: before.audit_values(),
                new: after.audit_values(),
            };
            Ok((after, vec![change]))
        })
    })
    .await
    .map_err(Into::into)
}
