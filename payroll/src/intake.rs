//! Bilimbi Payroll.intake and frozen mapping resolution. No live source formulas.
use crate::{
    input, replay,
    store::{self, Kind},
    *,
};
use rust_decimal::Decimal;
use serde_json::json;

impl Payroll {
    pub async fn intake(
        &self,
        scope: &Scope,
        company: i64,
        run: i64,
        input: &NewContribution,
    ) -> Result<Contribution> {
        self.accept(scope, company, run, input, None).await
    }
    /// `source_key` selects the frozen Leave/Claims mapping. The contribution's
    /// own `source_key` remains the company-unique operator replay key.
    pub async fn intake_mapped(
        &self,
        scope: &Scope,
        company: i64,
        run: i64,
        source_kind: &str,
        source_key: &str,
        input: &NewContribution,
    ) -> Result<Contribution> {
        self.accept(
            scope,
            company,
            run,
            input,
            Some(Mapped::Source(source_kind, source_key)),
        )
        .await
    }
    /// Attendance always earns the frozen item's rate. The allowance rule's
    /// configured value is not added or used to infer eligibility.
    pub async fn intake_attendance_allowance(
        &self,
        scope: &Scope,
        company: i64,
        run: i64,
        rule_code: &str,
        input: &NewContribution,
    ) -> Result<Contribution> {
        self.accept(
            scope,
            company,
            run,
            input,
            Some(Mapped::Attendance(rule_code)),
        )
        .await
    }
    async fn accept(
        &self,
        scope: &Scope,
        company: i64,
        run_id: i64,
        original: &NewContribution,
        mapping: Option<Mapped<'_>>,
    ) -> Result<Contribution> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .begin(&mut c, scope, company, MANAGE, "people.payroll.intake")
            .await?;
        let run = store::run_for_update(tx.sql(), scope, company, run_id).await?;
        if store::one_for_run::<Calculation>(tx.sql(), scope, company, Kind::Calculation, run_id)
            .await?
            .is_some()
        {
            return Err(PayrollError::IntakeUnavailable);
        }
        let mut input = original.clone();
        let date = input.on_date.to_string();
        if let Some(mapping) = mapping {
            input.item_id = match mapping {
                Mapped::Source(kind, key) => {
                    let mappings: Vec<Mapping> = store::decode(run.snapshot["mappings"].clone())?;
                    let ids: Vec<_> = mappings
                        .iter()
                        .filter(|m| {
                            m.source_kind == kind
                                && m.source_key == key
                                && input::covers(
                                    &m.effective_from,
                                    m.effective_to.as_deref(),
                                    &date,
                                    Some(&date),
                                )
                        })
                        .map(|m| m.item_id)
                        .collect();
                    match ids.as_slice() {
                        [id] => *id,
                        _ => return Err(PayrollError::SourceNotMapped),
                    }
                }
                Mapped::Attendance(code) => {
                    if !input.direction.is_empty() && input.direction != "earning" {
                        return Err(PayrollError::DirectionConflict);
                    }
                    input.direction = "earning".into();
                    let mappings: Vec<AttendanceAllowanceMapping> =
                        store::decode(run.snapshot["attendance_mappings"].clone())?;
                    let ids: Vec<_> = mappings
                        .iter()
                        .filter(|m| {
                            m.attendance_rule_code == code
                                && input::covers(
                                    &m.effective_from,
                                    m.effective_to.as_deref(),
                                    &date,
                                    Some(&date),
                                )
                        })
                        .map(|m| m.item_id)
                        .collect();
                    match ids.as_slice() {
                        [id] => *id,
                        _ => return Err(PayrollError::AllowanceNotMapped),
                    }
                }
            };
        }
        input::text(&input.source_key, "source_key", 120)?;
        input::text(&input.evidence, "evidence", 500)?;
        if !matches!(
            input.direction.as_str(),
            "earning" | "deduction" | "employer"
        ) {
            return Err(PayrollError::Invalid(vec![FieldError::new(
                "direction",
                "is invalid",
            )]));
        }
        if input.units <= Decimal::ZERO
            || input.units >= Decimal::from(100_000_000_000_000_i64)
            || input.units.normalize().scale() > 6
        {
            return Err(PayrollError::Invalid(vec![FieldError::new("units","must be an exact decimal with at most six fractional digits and greater than zero and less than 100000000000000")]));
        }
        let ids = self
            .workforce
            .working_employee_ids_on(tx.sql(), scope, company, &[input.employee_id])
            .await?
            .require_current()
            .map_err(|f| PayrollError::NotCurrent(f.0))?;
        if !ids.contains(&input.employee_id) {
            return Err(PayrollError::IntakeUnavailable);
        }
        let items: Vec<Item> = store::decode(run.snapshot["items"].clone())?;
        let period: Period = store::decode(run.snapshot["period"].clone())?;
        let item = items
            .iter()
            .find(|i| i.id == input.item_id)
            .ok_or(PayrollError::IntakeUnavailable)?;
        if date < period.starts_on
            || date > period.ends_on
            || !input::covers(
                &item.effective_from,
                item.effective_to.as_deref(),
                &date,
                Some(&date),
            )
        {
            return Err(PayrollError::IntakeUnavailable);
        }
        replay::multiply(&item.amount, input.units)?;
        // Keys are unique across every run of the company, not only this run.
        let previous:Option<i64>=tx.sql().query_opt("SELECT id FROM people_payroll_contributions WHERE tenant_id=$1 AND company_id=$2 AND source_key=$3",&[&scope.tenant(),&company,&input.source_key]).await?.map(|r|r.get(0));
        if let Some(id) = previous {
            let previous: Contribution =
                store::get(tx.sql(), scope, company, Kind::Contribution, id)
                    .await?
                    .ok_or(PayrollError::NotFound)?;
            if previous.run_id == run_id
                && previous.employee_id == input.employee_id
                && previous.item_id == input.item_id
                && previous.evidence == input.evidence
                && previous.on_date == date
                && previous.direction == input.direction
                && Decimal::from_str_exact(&previous.units).map_err(|_| Error::Internal)?
                    == input.units
            {
                return Ok(previous); // Read-only replay: drop the transaction, no audit.
            }
            return Err(PayrollError::ReplayConflict);
        }
        let contribution=store::insert(&mut tx,scope,company,Kind::Contribution,json!({"run_id":run_id,"employee_id":input.employee_id,"item_id":input.item_id,"source_key":input.source_key,"evidence":input.evidence,"on_date":date,"units":input.units.to_string(),"direction":input.direction})).await?;
        tx.commit().await?;
        Ok(contribution)
    }
}
enum Mapped<'a> {
    Source(&'a str, &'a str),
    Attendance(&'a str),
}
