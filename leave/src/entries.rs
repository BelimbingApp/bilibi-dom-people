//! Bilimbi Leave.record_entry/4, shared by the facade and reviewed writes.
use crate::{
    authorization, ledger, types, validate_entry, Company, Decimal, Employee, Error, Leave,
    LeaveError, LeaveType, LedgerEntry, MutationTransaction, NewEntry, Result, Rules, Scope,
    POLICIES_MANAGE,
};
use bilibi_tenancy::ReadClient;
use serde_json::json;
use tokio_postgres::GenericClient;

// The operation preview receives an object-safe client, while Core's company
// guard and Settings take GenericClient. Hold this Leave-owned lock on the
// caller's transaction before reading through those facades. Every Leave write
// takes it before the company lock (Leave::begin), so reviewed type/rule/key
// facts cannot change between the plan's final preview and its owner commit.
pub(super) async fn serialize(
    client: &dyn ReadClient,
    scope: &Scope,
    company_id: i64,
) -> Result<()> {
    client.query_opt(
        "SELECT pg_advisory_xact_lock(hashtextextended('people/leave:' || $1::text || ':' || $2::text, 0))",
        &[&scope.tenant().to_string(), &company_id.to_string()],
    ).await?;
    Ok(())
}

pub(super) struct RecordedEntry {
    pub entry: LedgerEntry,
    pub created: bool,
}
struct EntryFacts {
    company: Company,
    employee: Employee,
    leave_type: LeaveType,
    rules: Rules,
    quantity: Decimal,
    note: Option<String>,
    existing: Option<LedgerEntry>,
}
impl Leave {
    async fn entry_facts(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
        new: &NewEntry,
    ) -> Result<EntryFacts> {
        authorization::authorize(client, scope, company_id, POLICIES_MANAGE).await?;
        bilibi_company::require_writable(client, scope, company_id, false).await?;
        let company = self.current_company(client, scope, company_id).await?;
        let employee = self
            .current_employee(client, scope, company_id, employee_id)
            .await?;
        let (quantity, note) = validate_entry(new)?;
        let leave_type = types::list(client, scope.tenant(), company_id)
            .await?
            .into_iter()
            .find(|t| t.id == new.leave_type_id && t.status == "active")
            .ok_or(LeaveError::NotFound)?;
        let rules = self.read_rules(client, scope, &company).await?;
        let existing = ledger::find(
            client,
            scope.tenant(),
            company_id,
            &new.source,
            &new.entry_key,
        )
        .await?;
        if let Some(existing) = &existing {
            if existing.employee_id != employee_id
                || existing.leave_type_id != leave_type.id
                || existing.entry_type != new.entry_type
                || existing.quantity != quantity
                || existing.occurred_on != new.occurred_on
                || existing.note != note
            {
                return Err(LeaveError::EntryKeyConflict);
            }
        } else if ledger::closed(
            client,
            scope.tenant(),
            company_id,
            employee_id,
            leave_type.id,
            rules.leave_year(new.occurred_on),
        )
        .await?
        {
            return Err(LeaveError::YearClosed);
        }
        Ok(EntryFacts {
            company,
            employee,
            leave_type,
            rules,
            quantity,
            note,
            existing,
        })
    }

    pub(super) async fn review_entry(
        &self,
        client: &dyn bilibi_agent_api::OperationClient,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
        new: &NewEntry,
    ) -> Result<serde_json::Value> {
        serialize(client, scope, company_id).await?;
        let connection = self.db.get().await.map_err(Error::from)?;
        let facts = self
            .entry_facts(&**connection, scope, company_id, employee_id, new)
            .await?;
        Ok(json!({
            "effect": if facts.existing.is_some() { "existing" } else { "created" },
            "company_id":company_id, "company_name":facts.company.name,
            "employee_id":employee_id, "employee_number":facts.employee.employee_number,
            "employee_name":facts.employee.display_name, "leave_type":facts.leave_type,
            "year_start_month":facts.rules.year_start_month,
            "leave_year":facts.rules.leave_year(new.occurred_on),
            "entry_type":new.entry_type, "quantity":facts.quantity.to_string(),
            "occurred_on":new.occurred_on.to_string(), "source":new.source,
            "entry_key":new.entry_key, "note":facts.note, "existing":facts.existing,
        }))
    }

    // The only entry write, always on the caller's audit transaction. Identity
    // and authorization are reproved after the company lock, including replays.
    pub(super) async fn record_entry_in(
        &self,
        tx: &mut MutationTransaction<'_>,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
        new: &NewEntry,
    ) -> Result<RecordedEntry> {
        serialize(tx.sql(), scope, company_id).await?;
        bilibi_company::require_writable(tx.sql(), scope, company_id, true).await?;
        let facts = self
            .entry_facts(tx.sql(), scope, company_id, employee_id, new)
            .await?;
        if let Some(existing) = facts.existing {
            return Ok(RecordedEntry {
                entry: existing,
                created: false,
            });
        }
        let leave_type =
            types::lock_active(tx.sql(), scope.tenant(), company_id, new.leave_type_id)
                .await?
                .ok_or(LeaveError::NotFound)?;
        let mut inserted = ledger::insert(
            tx,
            scope.tenant(),
            company_id,
            vec![ledger::NewRow {
                employee_id,
                leave_type_id: leave_type.id,
                leave_year: facts.rules.leave_year(new.occurred_on),
                entry_type: new.entry_type.clone(),
                quantity: facts.quantity,
                unit: leave_type.unit,
                policy_id: None,
                policy_version: None,
                occurred_on: new.occurred_on,
                source: new.source.clone(),
                entry_key: new.entry_key.clone(),
                actor_user_id: scope.actor().user(),
                note: facts.note,
            }],
        )
        .await?;
        Ok(RecordedEntry {
            entry: inserted.pop().ok_or(LeaveError::EntryKeyConflict)?,
            created: true,
        })
    }
}
