//! Bilimbi `Leave.CarryForward`: close years in order and retain skip reasons.
use crate::{
    authorization, employee_ids, ledger, policies, types, Decimal, Error, Leave, LeaveError,
    MutationTransaction, Result, Rules, Scope, POLICIES_MANAGE,
};
use bilibi_database::Change;
use bilibi_queue::execution::Execution;
use serde::Serialize;
use serde_json::json;
use tokio_postgres::GenericClient;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CarryForwardCounts {
    pub carried: usize,
    pub existing: usize,
    pub pending: usize,
    pub previous_year_open: usize,
}
#[derive(Clone, Debug, Serialize)]
pub struct CarryForwardSkip {
    pub from_year: i32,
    pub employee_id: i64,
    pub employee_name: String,
    pub leave_type_id: i64,
    pub leave_type_name: String,
    pub reason: String,
    pub blocking_year: i32,
}
#[derive(Clone, Debug, Serialize)]
pub struct CarryForwardSkipped {
    pub skips: Vec<CarryForwardSkip>,
    pub total: i64,
}

impl Leave {
    /// Carries capped positive balances into the next year and expires the
    /// excess. Zero carry entries close years too; negative deficits stay.
    pub async fn carry_forward(
        &self,
        scope: &Scope,
        company_id: i64,
        from_year: i32,
    ) -> Result<CarryForwardCounts> {
        self.carry_forward_run(scope, company_id, from_year, None)
            .await
    }
    pub(super) async fn carry_forward_run(
        &self,
        scope: &Scope,
        company_id: i64,
        year: i32,
        execution: Option<&Execution>,
    ) -> Result<CarryForwardCounts> {
        if !(1900..=9997).contains(&year) {
            return Err(LeaveError::InvalidYear);
        }
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = match execution {
            Some(execution) => {
                let tx = execution
                    .begin(&mut client, "people.leave.carry_forward")
                    .await?;
                crate::entries::serialize(tx.sql(), scope, company_id).await?;
                crate::organisation::require_writable(tx.sql(), scope, company_id, true).await?;
                tx
            }
            None => {
                self.begin(&mut client, scope, company_id, "people.leave.carry_forward")
                    .await?
            }
        };
        authorization::authorize(tx.sql(), scope, company_id, POLICIES_MANAGE).await?;
        let company = self.current_company(scope, company_id).await?;
        let rules = self.read_rules(tx.sql(), scope, &company).await?;
        let (_, last_day) = rules.year_range(year)?;
        if last_day >= self.today_on(tx.sql(), scope, &company).await? {
            return Err(LeaveError::YearNotEnded);
        }
        let (first_next, _) = rules.year_range(year + 1)?;
        let employees = self
            .workforce
            .employees(scope, company_id)
            .await?
            .require_current()
            .map_err(|e| LeaveError::NotCurrent(e.0))?;
        let ids = employee_ids(&employees)?;
        let tenant = scope.tenant();
        let active = types::lock_all(tx.sql(), tenant, company_id).await?;
        let mut counts = CarryForwardCounts::default();
        let mut skips = Vec::new();
        let mut rows = Vec::new();
        let mut closed = Vec::new();
        for leave_type in active.into_iter().filter(|t| t.status == "active") {
            let Some(policy) =
                policies::effective(tx.sql(), tenant, company_id, leave_type.id, last_day).await?
            else {
                continue;
            };
            let Some(cap) = policy.carry_forward_cap else {
                continue;
            };
            for (employee, &employee_id) in employees.iter().zip(&ids) {
                if ledger::closed(
                    tx.sql(),
                    tenant,
                    company_id,
                    employee_id,
                    leave_type.id,
                    year,
                )
                .await?
                {
                    counts.existing += 1;
                    continue;
                }
                let blocking = blocking_year(
                    tx.sql(),
                    scope,
                    company_id,
                    employee_id,
                    leave_type.id,
                    year,
                    rules,
                )
                .await?;
                if let Some((reason, blocking_year)) = blocking {
                    if reason == "pending" {
                        counts.pending += 1;
                    } else {
                        counts.previous_year_open += 1;
                    }
                    skips.push(CarryForwardSkip {
                        from_year: year,
                        employee_id,
                        employee_name: format!(
                            "{} ({})",
                            employee.display_name, employee.employee_number
                        ),
                        leave_type_id: leave_type.id,
                        leave_type_name: leave_type.name.clone(),
                        reason: reason.into(),
                        blocking_year,
                    });
                    continue;
                }
                let sums = ledger::totals(tx.sql(), tenant, company_id, employee_id, year).await?;
                let remaining: Decimal = sums
                    .get(&leave_type.id)
                    .into_iter()
                    .flat_map(|s| s.values())
                    .copied()
                    .sum();
                let carried = remaining.min(cap).max(Decimal::ZERO);
                let expired = (remaining - carried).max(Decimal::ZERO);
                rows.push(ledger::NewRow {
                    employee_id,
                    leave_type_id: leave_type.id,
                    leave_year: year + 1,
                    entry_type: "carried_forward".into(),
                    quantity: carried,
                    unit: leave_type.unit.clone(),
                    policy_id: Some(policy.id),
                    policy_version: Some(policy.version),
                    occurred_on: first_next,
                    source: "carry_forward".into(),
                    entry_key: format!("carry:{}:{employee_id}:{year}", leave_type.id),
                    actor_user_id: scope.actor().user(),
                    note: None,
                });
                if expired > Decimal::ZERO {
                    rows.push(ledger::NewRow {
                        employee_id,
                        leave_type_id: leave_type.id,
                        leave_year: year,
                        entry_type: "expired".into(),
                        quantity: -expired,
                        unit: leave_type.unit.clone(),
                        policy_id: Some(policy.id),
                        policy_version: Some(policy.version),
                        occurred_on: last_day,
                        source: "carry_forward".into(),
                        entry_key: format!("expire:{}:{employee_id}:{year}", leave_type.id),
                        actor_user_id: scope.actor().user(),
                        note: None,
                    });
                }
                closed.push((employee_id, leave_type.id));
                counts.carried += 1;
            }
        }
        let mut changed = false;
        if !rows.is_empty() {
            ledger::insert(&mut tx, tenant, company_id, rows).await?;
            changed = true;
        }
        // Replace this run's report, and remove older/later reports whose
        // blocking year this run resolved. Each removed row is audited.
        let obsolete = tx.sql().query("SELECT id,employee_id,leave_type_id,from_year,blocking_year,to_jsonb(s) AS facts FROM people_leave_carry_forward_skips s WHERE tenant_id=$1 AND company_id=$2", &[&tenant,&company_id]).await.map_err(Error::from)?.into_iter().filter(|r| {
            r.get::<_,i32>("from_year") == year || closed.contains(&(r.get("employee_id"),r.get("leave_type_id"))) && (r.get::<_,i32>("from_year") <= year || r.get::<_,i32>("blocking_year") <= year)
        }).collect::<Vec<_>>();
        if !obsolete.is_empty() || !skips.is_empty() {
            replace_skips(&mut tx, tenant, company_id, year, obsolete, skips).await?;
            changed = true;
        }
        if changed {
            tx.commit().await?;
        } else {
            tx.rollback().await?;
        }
        Ok(counts)
    }
    /// Number of balances carried from a year (a closing row lives next year).
    pub async fn carried_forward_count(
        &self,
        scope: &Scope,
        company_id: i64,
        from_year: i32,
    ) -> Result<i64> {
        self.current_company(scope, company_id).await?;
        let next_year = from_year.checked_add(1).ok_or(LeaveError::InvalidYear)?;
        let client = self.db.get().await.map_err(Error::from)?;
        Ok(client.query_one("SELECT count(*) FROM people_leave_ledger_entries WHERE tenant_id=$1 AND company_id=$2 AND source='carry_forward' AND entry_type='carried_forward' AND leave_year=$3",&[&scope.tenant(),&company_id,&next_year]).await.map_err(Error::from)?.get(0))
    }
    /// Latest skipped years, oldest first, bounded to 200 plus the full count.
    pub async fn carry_forward_skipped(
        &self,
        scope: &Scope,
        company_id: i64,
    ) -> Result<CarryForwardSkipped> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, POLICIES_MANAGE).await?;
        self.current_company(scope, company_id).await?;
        // One snapshot keeps total and rows consistent during a worker run.
        let rows=client.query("SELECT s.from_year,s.employee_id,s.employee_label,s.leave_type_id,t.name,s.reason,s.blocking_year,count(*) OVER() AS total FROM people_leave_carry_forward_skips s JOIN people_leave_catalog_types t ON t.id=s.leave_type_id AND t.tenant_id=s.tenant_id AND t.company_id=s.company_id WHERE s.tenant_id=$1 AND s.company_id=$2 ORDER BY s.from_year,s.employee_label,t.name,s.id LIMIT 200",&[&scope.tenant(),&company_id]).await.map_err(Error::from)?;
        let total = rows.first().map_or(0, |r| r.get("total"));
        Ok(CarryForwardSkipped {
            total,
            skips: rows
                .iter()
                .map(|r| CarryForwardSkip {
                    from_year: r.get("from_year"),
                    employee_id: r.get("employee_id"),
                    employee_name: r.get("employee_label"),
                    leave_type_id: r.get("leave_type_id"),
                    leave_type_name: r.get("name"),
                    reason: r.get("reason"),
                    blocking_year: r.get("blocking_year"),
                })
                .collect(),
        })
    }
    /// Queues carry-forward as this operator; every execution rechecks access.
    pub async fn enqueue_carry_forward(
        &self,
        scope: &Scope,
        company_id: i64,
        from_year: i32,
    ) -> Result<bilibi_queue::JobRef> {
        if company_id <= 0 || !(1900..=9997).contains(&from_year) {
            return Err(LeaveError::InvalidCarryForward);
        }
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .begin(
                &mut client,
                scope,
                company_id,
                "people.leave.enqueue_carry_forward",
            )
            .await?;
        authorization::authorize(tx.sql(), scope, company_id, POLICIES_MANAGE).await?;
        let job = self
            .queue
            .enqueue_for(
                &mut tx,
                scope,
                crate::worker::WORKER,
                json!({"company_id":company_id,"from_year":from_year}),
            )
            .await?;
        tx.commit().await?;
        Ok(job)
    }
}
#[allow(clippy::too_many_arguments)]
async fn blocking_year(
    client: &impl GenericClient,
    scope: &Scope,
    company: i64,
    employee: i64,
    leave_type: i64,
    year: i32,
    rules: Rules,
) -> Result<Option<(&'static str, i32)>> {
    let row=client.query_one("SELECT coalesce(max(leave_year)-1,0) FROM people_leave_ledger_entries WHERE tenant_id=$1 AND company_id=$2 AND employee_id=$3 AND leave_type_id=$4 AND source='carry_forward' AND entry_type='carried_forward'",&[&scope.tenant(),&company,&employee,&leave_type]).await.map_err(Error::from)?;
    let after: i32 = row.get(0);
    let pending:Vec<i32>=client.query("SELECT DISTINCT leave_year FROM people_leave_applications WHERE tenant_id=$1 AND company_id=$2 AND employee_id=$3 AND leave_type_id=$4 AND status='pending' AND leave_year>$5 AND leave_year<=$6 ORDER BY leave_year",&[&scope.tenant(),&company,&employee,&leave_type,&after,&year]).await.map_err(Error::from)?.iter().map(|r|r.get(0)).collect();
    if pending.contains(&year) {
        return Ok(Some(("pending", year)));
    }
    let years=client.query("SELECT DISTINCT leave_year FROM people_leave_ledger_entries WHERE tenant_id=$1 AND company_id=$2 AND employee_id=$3 AND leave_type_id=$4 AND leave_year>$5 AND leave_year<$6 ORDER BY leave_year",&[&scope.tenant(),&company,&employee,&leave_type,&after,&year]).await.map_err(Error::from)?;
    let mut earliest = pending.into_iter().min();
    for row in years {
        let earlier: i32 = row.get(0);
        let (_, last) = rules.year_range(earlier)?;
        if policies::effective(client, scope.tenant(), company, leave_type, last)
            .await?
            .is_some_and(|p| p.carry_forward_cap.is_some())
        {
            earliest = Some(earliest.map_or(earlier, |e| e.min(earlier)));
        }
    }
    Ok(earliest.map(|year| ("previous_year_open", year)))
}
async fn replace_skips(
    tx: &mut MutationTransaction<'_>,
    tenant: i64,
    company: i64,
    year: i32,
    obsolete: Vec<tokio_postgres::Row>,
    skips: Vec<CarryForwardSkip>,
) -> Result<()> {
    tx.mutate(move|sql|Box::pin(async move {
        let mut changes=Vec::new();
        for row in obsolete {let id:i64=row.get("id");sql.execute("DELETE FROM people_leave_carry_forward_skips WHERE tenant_id=$1 AND company_id=$2 AND id=$3",&[&tenant,&company,&id]).await?;changes.push(Change {kind:"people_leave_carry_forward_skip".into(),id,old:row.get("facts"),new:json!({})});}
        for skip in skips {
            let row=sql.query_one("INSERT INTO people_leave_carry_forward_skips (tenant_id,company_id,from_year,employee_id,employee_label,leave_type_id,reason,blocking_year) VALUES ($1,$2,$3,$4,$5,$6,$7,$8) RETURNING id,to_jsonb(people_leave_carry_forward_skips) AS facts",&[&tenant,&company,&year,&skip.employee_id,&skip.employee_name,&skip.leave_type_id,&skip.reason,&skip.blocking_year]).await?;
            changes.push(Change {kind:"people_leave_carry_forward_skip".into(),id:row.get("id"),old:json!({}),new:row.get("facts")});
        }
        Ok(((),changes))
    })).await?;
    Ok(())
}
