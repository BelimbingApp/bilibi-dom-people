//! Bilimbi Payroll.create_run: stable frozen definitions and unmapped sources.
use crate::{
    input,
    setup::rule_overlaps,
    store::{self, Kind},
    *,
};
use serde_json::json;
use std::collections::BTreeSet;
impl Payroll {
    /// Freeze one period/currency, exactly once. No intake or calculation is implied.
    pub async fn freeze_setup(
        &self,
        scope: &Scope,
        company: i64,
        period_id: i64,
        currency: &str,
    ) -> Result<Run> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .begin(
                &mut c,
                scope,
                company,
                MANAGE,
                "people.payroll.freeze_setup",
            )
            .await?;
        self.workforce
            .company_on(tx.sql(), scope, company)
            .await?
            .require_current()
            .map_err(|e| PayrollError::NotCurrent(e.0))?;
        let (country, currencies) = self.configuration(tx.sql(), scope, company).await?;
        if country.trim().is_empty() || !currencies.iter().any(|v| v == currency) {
            return Err(PayrollError::RunUnavailable);
        }
        let period: Period = store::get(tx.sql(), scope, company, Kind::Period, period_id)
            .await?
            .ok_or(PayrollError::RunUnavailable)?;
        let runs: Vec<Run> = store::list(tx.sql(), scope, company, Kind::Run).await?;
        if runs
            .iter()
            .any(|r| r.period_id == period_id && r.currency == currency)
        {
            return Err(PayrollError::RunUnavailable);
        }
        let mut sources = self.sources(scope, company).await?;
        let from = period.starts_on.parse().map_err(|_| Error::Internal)?;
        let to = period.ends_on.parse().map_err(|_| Error::Internal)?;
        let mut requested = BTreeSet::new();
        for id in self
            .leave
            .requested_type_ids(scope, company, from, to)
            .await?
        {
            requested.insert(("leave".to_owned(), id.to_string()));
        }
        for id in self
            .claims
            .requested_claim_type_ids(scope, company, from, to)
            .await?
        {
            requested.insert(("claims".to_owned(), id.to_string()));
        }
        let rules = self.attendance.list_allowance_rules(scope, company).await?;
        let rules: Vec<_> = rules
            .iter()
            .filter(|r| rule_overlaps(r, &period.starts_on, Some(&period.ends_on)))
            .collect();
        let mut attendance_sources = Vec::new();
        for rule in rules.iter().filter(|r| r.currency == currency) {
            if !attendance_sources
                .iter()
                .any(|v: &Source| v.key == rule.code)
            {
                attendance_sources.push(Source {
                    key: rule.code.clone(),
                    name: rule.name.clone(),
                    active: true,
                });
            }
        }
        sources.insert("attendance".into(), attendance_sources);
        let items: Vec<Item> = store::list(tx.sql(), scope, company, Kind::Item).await?;
        let items: Vec<_> = items
            .into_iter()
            .filter(|r| {
                r.currency == currency
                    && effective(&period, &r.effective_from, r.effective_to.as_deref())
            })
            .collect();
        let ids: BTreeSet<_> = items.iter().map(|i| i.id).collect();
        let classifications: Vec<Classification> =
            store::list(tx.sql(), scope, company, Kind::Classification).await?;
        let classifications: Vec<_> = classifications
            .into_iter()
            .filter(|r| effective(&period, &r.effective_from, r.effective_to.as_deref()))
            .collect();
        let mappings: Vec<Mapping> = store::list(tx.sql(), scope, company, Kind::Mapping).await?;
        let mappings: Vec<_> = mappings
            .into_iter()
            .filter(|r| {
                ids.contains(&r.item_id)
                    && effective(&period, &r.effective_from, r.effective_to.as_deref())
            })
            .collect();
        let attendance: Vec<AttendanceAllowanceMapping> =
            store::list(tx.sql(), scope, company, Kind::AttendanceMapping).await?;
        let mut attendance_mappings = Vec::new();
        let mut mismatches = std::collections::BTreeMap::new();
        for mapping in attendance.into_iter().filter(|r| {
            ids.contains(&r.item_id)
                && effective(&period, &r.effective_from, r.effective_to.as_deref())
        }) {
            if let Some(rule) = rules.iter().find(|r| {
                r.code == mapping.attendance_rule_code
                    && rule_overlaps(r, &mapping.effective_from, mapping.effective_to.as_deref())
                    && r.currency != currency
            }) {
                mismatches.insert(mapping.attendance_rule_code, rule.name.clone());
            } else {
                attendance_mappings.push(mapping);
            }
        }
        let mut mapped: BTreeSet<_> = mappings
            .iter()
            .map(|r| (r.source_kind.clone(), r.source_key.clone()))
            .collect();
        for mapping in &attendance_mappings {
            mapped.insert(("attendance".into(), mapping.attendance_rule_code.clone()));
        }
        for code in mismatches.keys() {
            mapped.insert(("attendance".into(), code.clone()));
        }
        let mut unmapped_sources = Vec::new();
        for (kind, choices) in &mut sources {
            choices.sort_by(|a, b| a.key.cmp(&b.key));
            for choice in choices {
                let key = (kind.clone(), choice.key.clone());
                if (choice.active || requested.contains(&key)) && !mapped.contains(&key) {
                    unmapped_sources.push(
                        json!({"source_kind":kind,"source_key":choice.key,"name":choice.name}),
                    );
                }
            }
        }
        for (code, name) in mismatches {
            unmapped_sources.push(json!({"source_kind":"attendance","source_key":code,"name":name,"reason":"currency mismatch"}));
        }
        let snapshot = json!({"period":period,"items":items,"classifications":classifications,"mappings":mappings,"attendance_mappings":attendance_mappings,"unmapped_sources":unmapped_sources});
        let run=store::insert(&mut tx,scope,company,Kind::Run,json!({"period_id":period_id,"country":country,"currency":currency,"snapshot":snapshot})).await?;
        tx.commit().await?;
        Ok(run)
    }
    /// Bilimbi's name for the same freeze; retained for page and follow-on callers.
    pub async fn create_run(
        &self,
        scope: &Scope,
        company: i64,
        period: i64,
        currency: &str,
    ) -> Result<Run> {
        self.freeze_setup(scope, company, period, currency).await
    }
}
fn effective(period: &Period, from: &str, to: Option<&str>) -> bool {
    input::overlaps(&period.starts_on, Some(&period.ends_on), from, to)
}
