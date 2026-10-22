//! The `people_leave_policies` table: effective-dated entitlement versions
//! of a type, the SQL that reads them and the write behind
//! `Leave::add_policy`. Private to the facade.
use crate::{NewPolicy, Policy, Result};
use bilibi_database::{Change, Error, MutationTransaction};
use jiff::civil::Date;
use serde_json::{json, Value};
use tokio_postgres::{GenericClient, Row};

/// Dates as ISO text: the workspace's `tokio-postgres` has no date feature.
const COLUMNS: &str =
    "id, leave_type_id, version, to_char(effective_from, 'YYYY-MM-DD') AS effective_from, \
    to_char(effective_to, 'YYYY-MM-DD') AS effective_to, entitlement, carry_forward_cap";

pub(crate) fn date(row: &Row, column: &str) -> Result<Option<Date>> {
    row.get::<_, Option<String>>(column)
        .map(|text| text.parse::<Date>().map_err(|_| Error::Internal))
        .transpose()
        .map_err(Into::into)
}

pub(crate) fn read(row: &Row) -> Result<Policy> {
    Ok(Policy {
        id: row.get("id"),
        leave_type_id: row.get("leave_type_id"),
        version: row.get("version"),
        effective_from: date(row, "effective_from")?.ok_or(Error::Internal)?,
        effective_to: date(row, "effective_to")?,
        entitlement: row.get("entitlement"),
        carry_forward_cap: row.get("carry_forward_cap"),
    })
}

/// The company's versions by type, newest version first.
pub(crate) async fn list(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
) -> Result<Vec<Policy>> {
    client
        .query(
            &format!("SELECT {COLUMNS} FROM people_leave_policies WHERE tenant_id = $1 AND company_id = $2 ORDER BY leave_type_id, version DESC"),
            &[&tenant, &company_id],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(read)
        .collect()
}

/// The type's latest version, locked for the transaction (a new version
/// closes it).
pub(crate) async fn lock_latest(
    client: &impl GenericClient,
    tenant: i64,
    type_id: i64,
) -> Result<Option<Policy>> {
    client
        .query_opt(
            &format!("SELECT {COLUMNS} FROM people_leave_policies WHERE tenant_id = $1 AND leave_type_id = $2 ORDER BY version DESC LIMIT 1 FOR UPDATE"),
            &[&tenant, &type_id],
        )
        .await
        .map_err(Error::from)?
        .as_ref()
        .map(read)
        .transpose()
}

/// The version in force on `date`, if any.
pub(crate) async fn effective(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    type_id: i64,
    date: Date,
) -> Result<Option<Policy>> {
    client
        .query_opt(
            &format!("SELECT {COLUMNS} FROM people_leave_policies WHERE tenant_id = $1 AND company_id = $2 AND leave_type_id = $3 \
                      AND effective_from <= $4::text::date AND (effective_to IS NULL OR effective_to >= $4::text::date)"),
            &[&tenant, &company_id, &type_id, &date.to_string()],
        )
        .await
        .map_err(Error::from)?
        .as_ref()
        .map(read)
        .transpose()
}

impl Policy {
    pub(crate) fn audit_values(&self) -> Value {
        json!({
            "leave_type_id": self.leave_type_id,
            "version": self.version,
            "effective_from": self.effective_from.to_string(),
            "effective_to": self.effective_to.map(|date| date.to_string()),
            "entitlement": self.entitlement.to_string(),
            "carry_forward_cap": self.carry_forward_cap.map(|cap| cap.to_string()),
        })
    }
}

/// Closes `latest` on the day before the new version starts and inserts the
/// new version: an updated and a created `people_leave_policy` change.
pub(crate) async fn insert(
    tx: &mut MutationTransaction<'_>,
    tenant: i64,
    company_id: i64,
    type_id: i64,
    latest: Option<Policy>,
    new: NewPolicy,
    actor_user_id: i64,
) -> Result<Policy> {
    let version = latest.as_ref().map_or(1, |latest| latest.version + 1);
    tx.mutate(move |sql| {
        Box::pin(async move {
            let mut changes = Vec::with_capacity(2);
            if let Some(before) = latest {
                let closed_on = new.effective_from.yesterday().map_err(|_| Error::Invalid)?;
                let row = sql
                    .query_one(
                        &format!(
                            "UPDATE people_leave_policies SET effective_to = $2::text::date, updated_at = now() AT TIME ZONE 'UTC' \
                             WHERE id = $1 RETURNING {COLUMNS}"
                        ),
                        &[&before.id, &closed_on.to_string()],
                    )
                    .await?;
                let after = read(&row).map_err(|_| Error::Internal)?;
                changes.push(Change {
                    kind: "people_leave_policy".into(),
                    id: after.id,
                    old: before.audit_values(),
                    new: after.audit_values(),
                });
            }
            let row = sql
                .query_one(
                    &format!(
                        "INSERT INTO people_leave_policies \
                         (tenant_id, company_id, leave_type_id, version, effective_from, entitlement, carry_forward_cap, actor_user_id) \
                         VALUES ($1, $2, $3, $4, $5::text::date, $6, $7, $8) RETURNING {COLUMNS}"
                    ),
                    &[
                        &tenant,
                        &company_id,
                        &type_id,
                        &version,
                        &new.effective_from.to_string(),
                        &new.entitlement,
                        &new.carry_forward_cap,
                        &actor_user_id,
                    ],
                )
                .await?;
            let policy = read(&row).map_err(|_| Error::Internal)?;
            changes.push(Change {
                kind: "people_leave_policy".into(),
                id: policy.id,
                old: json!({}),
                new: policy.audit_values(),
            });
            Ok((policy, changes))
        })
    })
    .await
    .map_err(Into::into)
}
