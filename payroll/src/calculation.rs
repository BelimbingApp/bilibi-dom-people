//! Bilimbi Payroll.calculate, run_output and independent final decisions.
use crate::{
    replay,
    store::{self, Kind},
    *,
};
use rust_decimal::Decimal;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
impl Payroll {
    /// Calculate once; a retry returns the stored calculation without audit writes.
    pub async fn calculate(&self, scope: &Scope, company: i64, run_id: i64) -> Result<Calculation> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .begin(&mut c, scope, company, MANAGE, "people.payroll.calculate")
            .await?;
        let run = store::run_for_update(tx.sql(), scope, company, run_id).await?;
        if run.locked_at.is_none() {
            return Err(PayrollError::RunNotLocked);
        }
        if let Some(previous) =
            store::one_for_run(tx.sql(), scope, company, Kind::Calculation, run_id).await?
        {
            return Ok(previous);
        }
        let inputs: Vec<Contribution> =
            store::for_run(tx.sql(), scope, company, Kind::Contribution, run_id).await?;
        if inputs.is_empty() {
            return Err(PayrollError::NoContributions);
        }
        let ids: Vec<_> = inputs
            .iter()
            .map(|i| i.employee_id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        for page in ids.chunks(bilibi_people_workforce::LOOKUP_LIMIT) {
            let current = self
                .workforce
                .working_employee_ids_on(tx.sql(), scope, company, page)
                .await?
                .require_current()
                .map_err(|f| PayrollError::NotCurrent(f.0))?;
            if current.len() != page.len() {
                return Err(PayrollError::EmployeeNotFound);
            }
        }
        let result = replay::calculate(&run, &inputs)?;
        let snapshot = json!({"setup":run.snapshot,"contributions":inputs,"result":result});
        let digest = replay::digest(&snapshot);
        // Entries precede Calculation: the database stage trigger closes intake
        // and result lines as soon as the immutable calculation is inserted.
        for line in snapshot["result"]["lines"]
            .as_array()
            .ok_or(Error::Internal)?
        {
            let _:ResultLine=store::insert(&mut tx,scope,company,Kind::Entry,json!({"run_id":run_id,"contribution_id":line["id"],"employee_id":line["employee_id"],"direction":line["direction"],"amount":line["amount"]})).await?;
        }
        let calculation = store::insert(
            &mut tx,
            scope,
            company,
            Kind::Calculation,
            json!({"run_id":run_id,"snapshot":snapshot,"digest":digest}),
        )
        .await?;
        tx.commit().await?;
        Ok(calculation)
    }
    pub async fn run_output(&self, scope: &Scope, company: i64, run_id: i64) -> Result<RunOutput> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        // One MVCC snapshot prevents a mixed pre/post-calculation output.
        let tx = c
            .build_transaction()
            .isolation_level(tokio_postgres::IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .await?;
        self.authorize(&tx, scope, company, VIEW).await?;
        let run = store::get(&tx, scope, company, Kind::Run, run_id)
            .await?
            .ok_or(PayrollError::NotFound)?;
        let output = RunOutput {
            run,
            contributions: store::for_run(&tx, scope, company, Kind::Contribution, run_id).await?,
            calculation: store::one_for_run(&tx, scope, company, Kind::Calculation, run_id).await?,
            decision: store::one_for_run(&tx, scope, company, Kind::Decision, run_id).await?,
            documents: store::for_run(&tx, scope, company, Kind::Document, run_id).await?,
            entries: store::for_run(&tx, scope, company, Kind::Entry, run_id).await?,
        };
        tx.commit().await?;
        Ok(output)
    }
    /// Approve or reject once with a reason. Author, locker, calculator and
    /// every contribution author are excluded, even with APPROVE permission.
    pub async fn decide_run(
        &self,
        scope: &Scope,
        company: i64,
        run_id: i64,
        outcome: &str,
        reason: &str,
    ) -> Result<Decision> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .begin(&mut c, scope, company, APPROVE, "people.payroll.decide_run")
            .await?;
        if !matches!(outcome, "approved" | "rejected")
            || reason.trim().is_empty()
            || reason.trim().chars().count() > 500
        {
            return Err(PayrollError::InvalidDecision);
        }
        let run = store::run_for_update(tx.sql(), scope, company, run_id).await?;
        let calculation: Calculation =
            store::one_for_run(tx.sql(), scope, company, Kind::Calculation, run_id)
                .await?
                .ok_or(PayrollError::NotDecidable)?;
        let actor = scope.actor().user();
        if actor == run.created_by_actor_id
            || Some(actor) == run.locked_by_actor_id
            || actor == calculation.created_by_actor_id
            || store::one_for_run::<Decision>(tx.sql(), scope, company, Kind::Decision, run_id)
                .await?
                .is_some()
        {
            return Err(PayrollError::NotDecidable);
        }
        if tx.sql().query_one("SELECT EXISTS(SELECT 1 FROM people_payroll_contributions WHERE tenant_id=$1 AND company_id=$2 AND run_id=$3 AND created_by_actor_id=$4)",&[&scope.tenant(),&company,&run_id,&actor]).await?.get::<_,bool>(0) {return Err(PayrollError::NotDecidable);}
        let payload = if outcome == "approved" {
            Some(approved_payload(company, &run, &calculation)?)
        } else {
            None
        };
        let decision = store::insert(
            &mut tx,
            scope,
            company,
            Kind::Decision,
            json!({"run_id":run_id,"outcome":outcome,"reason":reason.trim()}),
        )
        .await?;
        if let Some(payload) = payload {
            self.events
                .record(
                    &mut tx,
                    bilibi_events::NewEvent {
                        name: RUN_APPROVED,
                        subject: &format!("run:{run_id}"),
                        key: &format!("run_approved:{run_id}"),
                        payload,
                    },
                )
                .await?;
        }
        tx.commit().await?;
        Ok(decision)
    }
    pub async fn approve_run(
        &self,
        scope: &Scope,
        company: i64,
        run: i64,
        reason: &str,
    ) -> Result<Decision> {
        self.decide_run(scope, company, run, "approved", reason)
            .await
    }
    pub async fn reject_run(
        &self,
        scope: &Scope,
        company: i64,
        run: i64,
        reason: &str,
    ) -> Result<Decision> {
        self.decide_run(scope, company, run, "rejected", reason)
            .await
    }
}
fn approved_payload(company: i64, run: &Run, calculation: &Calculation) -> Result<Value> {
    let mut components: BTreeMap<(String, String), Decimal> = BTreeMap::new();
    for line in calculation.snapshot["result"]["lines"]
        .as_array()
        .ok_or(Error::Internal)?
    {
        let key = (
            line["item_code"]
                .as_str()
                .ok_or(Error::Internal)?
                .to_owned(),
            line["direction"]
                .as_str()
                .ok_or(Error::Internal)?
                .to_owned(),
        );
        let value = replay::amount(line["amount"].as_str().ok_or(Error::Internal)?)?;
        let sum = components.entry(key).or_insert(Decimal::ZERO);
        *sum = replay::add(*sum, value)?;
    }
    let components:Vec<_>=components.into_iter().map(|((code,direction),value)|json!({"code":code,"direction":direction,"amount":value.to_string()})).collect();
    let period = &run.snapshot["period"];
    let payload = json!({"company_id":company,"run_id":run.id,"reference":period["code"],"currency":run.currency,"pay_on":period["pay_on"],"components":components});
    if serde_json::to_vec(&payload)
        .map_err(|_| Error::Internal)?
        .len()
        > 65_536
    {
        return Err(PayrollError::ApprovalTooLarge);
    }
    Ok(payload)
}
