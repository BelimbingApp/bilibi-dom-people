//! The `people_leave_ledger_entries` table (and the pending quantity read
//! from `people_leave_applications`): rows, the SQL that reads and sums
//! them, and the inserts behind `Leave::grant_entitlements` and
//! `Leave::record_entry`. The table is append-only (migration 001's
//! trigger), so there is no update here. Private to the facade.
use crate::{LedgerEntry, Result};
use bilibi_database::{Change, Error, MutationTransaction};
use jiff::civil::Date;
use rust_decimal::Decimal;
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio_postgres::{GenericClient, Row};

/// The source of grants (`policy`); with `request` and `carry_forward`, one
/// of the module's own sources an external entry may not use.
pub(crate) const GRANT_SOURCE: &str = "policy";
pub(crate) const RESERVED_SOURCES: [&str; 3] = ["policy", "request", "carry_forward"];
/// The source carry-forward writes; a `carried_forward` entry from it in a
/// later year closes the earlier years of its employee and type.
const CARRY_FORWARD_SOURCE: &str = "carry_forward";

const COLUMNS: &str =
    "id, employee_id, leave_type_id, leave_year, entry_type, quantity, unit, policy_version, \
    to_char(occurred_on, 'YYYY-MM-DD') AS occurred_on, source, entry_key, note";

pub(crate) fn read(row: &Row) -> Result<LedgerEntry> {
    Ok(LedgerEntry {
        id: row.get("id"),
        employee_id: row.get("employee_id"),
        leave_type_id: row.get("leave_type_id"),
        leave_year: row.get("leave_year"),
        entry_type: row.get("entry_type"),
        quantity: row.get("quantity"),
        unit: row.get("unit"),
        policy_version: row.get("policy_version"),
        occurred_on: crate::policies::date(row, "occurred_on")?.ok_or(Error::Internal)?,
        source: row.get("source"),
        entry_key: row.get("entry_key"),
        note: row.get("note"),
    })
}

/// The entry under a company, source and key, if any.
pub(crate) async fn find(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    source: &str,
    entry_key: &str,
) -> Result<Option<LedgerEntry>> {
    client
        .query_opt(
            &format!("SELECT {COLUMNS} FROM people_leave_ledger_entries WHERE tenant_id = $1 AND company_id = $2 AND source = $3 AND entry_key = $4"),
            &[&tenant, &company_id, &source, &entry_key],
        )
        .await
        .map_err(Error::from)?
        .as_ref()
        .map(read)
        .transpose()
}

/// Whether the company has any ledger entry (each carries its leave year,
/// which is why the leave year cannot change afterwards).
pub(crate) async fn in_use(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
) -> Result<bool> {
    Ok(client
        .query_opt(
            "SELECT 1 FROM people_leave_ledger_entries WHERE tenant_id = $1 AND company_id = $2 LIMIT 1",
            &[&tenant, &company_id],
        )
        .await
        .map_err(Error::from)?
        .is_some())
}

/// Whether an entitlement of the type was granted on or after `date`.
pub(crate) async fn granted_on_or_after(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    type_id: i64,
    date: Date,
) -> Result<bool> {
    Ok(client
        .query_opt(
            "SELECT 1 FROM people_leave_ledger_entries WHERE tenant_id = $1 AND company_id = $2 AND leave_type_id = $3 \
             AND entry_type = 'entitlement' AND occurred_on >= $4::text::date LIMIT 1",
            &[&tenant, &company_id, &type_id, &date.to_string()],
        )
        .await
        .map_err(Error::from)?
        .is_some())
}

/// Whether `year` is closed for the employee and type: carry-forward wrote
/// a `carried_forward` entry into a later year (Bilimbi
/// `CarryForward.closed?/5`; a year's closing entry lives in the next year).
pub(crate) async fn closed(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    employee_id: i64,
    type_id: i64,
    year: i32,
) -> Result<bool> {
    Ok(client
        .query_opt(
            "SELECT 1 FROM people_leave_ledger_entries WHERE tenant_id = $1 AND company_id = $2 AND employee_id = $3 \
             AND leave_type_id = $4 AND source = $5 AND entry_type = 'carried_forward' AND leave_year > $6 LIMIT 1",
            &[&tenant, &company_id, &employee_id, &type_id, &CARRY_FORWARD_SOURCE, &year],
        )
        .await
        .map_err(Error::from)?
        .is_some())
}

/// One employee's entries of a leave year, latest date first.
pub(crate) async fn list(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    employee_id: i64,
    year: i32,
) -> Result<Vec<LedgerEntry>> {
    client
        .query(
            &format!("SELECT {COLUMNS} FROM people_leave_ledger_entries WHERE tenant_id = $1 AND company_id = $2 \
                      AND employee_id = $3 AND leave_year = $4 ORDER BY occurred_on DESC, id DESC"),
            &[&tenant, &company_id, &employee_id, &year],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(read)
        .collect()
}

/// The sum of one employee's entries of a leave year, per type and entry type.
pub(crate) async fn totals(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    employee_id: i64,
    year: i32,
) -> Result<HashMap<i64, HashMap<String, Decimal>>> {
    let rows = client
        .query(
            "SELECT leave_type_id, entry_type, sum(quantity) AS total FROM people_leave_ledger_entries \
             WHERE tenant_id = $1 AND company_id = $2 AND employee_id = $3 AND leave_year = $4 \
             GROUP BY leave_type_id, entry_type",
            &[&tenant, &company_id, &employee_id, &year],
        )
        .await
        .map_err(Error::from)?;
    let mut totals: HashMap<i64, HashMap<String, Decimal>> = HashMap::new();
    for row in rows {
        totals
            .entry(row.get("leave_type_id"))
            .or_default()
            .insert(row.get("entry_type"), row.get("total"));
    }
    Ok(totals)
}

/// The quantity one employee's pending requests of a leave year hold, per
/// type (Bilimbi `Requests.pending_totals/4`).
pub(crate) async fn pending_totals(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    employee_id: i64,
    year: i32,
) -> Result<HashMap<i64, Decimal>> {
    Ok(client
        .query(
            "SELECT leave_type_id, sum(quantity) AS total FROM people_leave_applications \
             WHERE tenant_id = $1 AND company_id = $2 AND employee_id = $3 AND leave_year = $4 AND status = 'pending' \
             GROUP BY leave_type_id",
            &[&tenant, &company_id, &employee_id, &year],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(|row| (row.get("leave_type_id"), row.get("total")))
        .collect())
}

/// An entry about to be written.
pub(crate) struct NewRow {
    pub employee_id: i64,
    pub leave_type_id: i64,
    pub leave_year: i32,
    pub entry_type: String,
    pub quantity: Decimal,
    pub unit: String,
    pub policy_id: Option<i64>,
    pub policy_version: Option<i32>,
    pub occurred_on: Date,
    pub source: String,
    pub entry_key: String,
    pub actor_user_id: i64,
    pub note: Option<String>,
}

impl LedgerEntry {
    pub(crate) fn audit_values(&self) -> Value {
        json!({
            "employee_id": self.employee_id,
            "leave_type_id": self.leave_type_id,
            "leave_year": self.leave_year,
            "entry_type": self.entry_type,
            "quantity": self.quantity.to_string(),
            "unit": self.unit,
            "policy_version": self.policy_version,
            "occurred_on": self.occurred_on.to_string(),
            "source": self.source,
            "entry_key": self.entry_key,
            "note": self.note,
        })
    }
}

/// Inserts the rows, each recorded as a created `people_leave_ledger_entry`.
/// A row whose company, source and key already exist is left out of the
/// result (a concurrent run that won the unique key; the company lock makes
/// this rare), so the caller counts it as existing. At least one row must
/// land, or the boundary refuses the write.
pub(crate) async fn insert(
    tx: &mut MutationTransaction<'_>,
    tenant: i64,
    company_id: i64,
    rows: Vec<NewRow>,
) -> Result<Vec<LedgerEntry>> {
    tx.mutate(move |sql| {
        Box::pin(async move {
            let mut entries = Vec::with_capacity(rows.len());
            let mut changes = Vec::with_capacity(rows.len());
            for new in rows {
                let Some(row) = sql
                    .query_opt(
                        &format!(
                            "INSERT INTO people_leave_ledger_entries \
                             (tenant_id, company_id, employee_id, leave_type_id, leave_year, entry_type, quantity, unit, \
                              policy_id, policy_version, occurred_on, source, entry_key, actor_user_id, note) \
                             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11::text::date, $12, $13, $14, $15) \
                             ON CONFLICT (company_id, source, entry_key) DO NOTHING RETURNING {COLUMNS}"
                        ),
                        &[
                            &tenant,
                            &company_id,
                            &new.employee_id,
                            &new.leave_type_id,
                            &new.leave_year,
                            &new.entry_type,
                            &new.quantity,
                            &new.unit,
                            &new.policy_id,
                            &new.policy_version,
                            &new.occurred_on.to_string(),
                            &new.source,
                            &new.entry_key,
                            &new.actor_user_id,
                            &new.note,
                        ],
                    )
                    .await?
                else {
                    continue;
                };
                let entry = read(&row).map_err(|_| Error::Internal)?;
                changes.push(Change {
                    kind: "people_leave_ledger_entry".into(),
                    id: entry.id,
                    old: json!({}),
                    new: entry.audit_values(),
                });
                entries.push(entry);
            }
            Ok((entries, changes))
        })
    })
    .await
    .map_err(Into::into)
}
