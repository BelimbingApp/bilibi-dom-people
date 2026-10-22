//! Bilimbi Attendance.Allowances: effective versions, inclusive ends and retirement.
use crate::{
    shifts,
    workflow::{action, decode, lock},
    Attendance, AttendanceError as Refusal, FieldError, Result, ALLOWANCES_MANAGE,
};
use bilibi_database::{Change, Error, MutationTransaction};
use bilibi_tenancy::Scope;
use jiff::civil::Date;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[derive(Clone, Debug, Serialize, Deserialize)]
/// A stored rule version for Payroll; dates are inclusive ISO text.
pub struct AllowanceRule {
    pub id: i64,
    pub code: String,
    pub name: String,
    pub unit: String,
    pub value: Decimal,
    pub currency: String,
    pub effective_from: String,
    pub effective_until: Option<String>,
    pub status: String,
}
#[derive(Clone, Debug)]
/// Company-defined allowance facts and the new version's inclusive dates.
pub struct NewAllowanceRule {
    pub code: String,
    pub name: String,
    pub unit: String,
    pub value: Decimal,
    pub currency: String,
    pub effective_from: Date,
    pub effective_until: Option<Date>,
}
fn validate(new: &NewAllowanceRule) -> Result<(String, String, String, String)> {
    let mut errors = Vec::new();
    let (code, name) = shifts::code_and_name(&new.code, &new.name, &mut errors);
    let unit = new.unit.trim().to_owned();
    let currency = new.currency.trim().to_ascii_uppercase();
    if unit.is_empty() {
        errors.push(FieldError::new("unit", "can't be blank"));
    } else if unit.chars().count() > 32 {
        errors.push(FieldError::new("unit", "should be at most 32 character(s)"));
    } else if !unit.as_bytes()[0].is_ascii_alphabetic() || !shifts::code_format(&unit) {
        errors.push(FieldError::new("unit", "has invalid format"));
    }
    if currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_uppercase()) {
        errors.push(FieldError::new("currency", "has invalid format"));
    }
    if new.value <= Decimal::ZERO {
        errors.push(FieldError::new("value", "must be greater than 0"));
    }
    // PostgreSQL rounds numeric(14,4) half away from zero; validate that stored value before SQL.
    let stored = new
        .value
        .round_dp_with_strategy(4, rust_decimal::RoundingStrategy::MidpointAwayFromZero);
    if stored <= Decimal::ZERO || stored >= Decimal::from(10_000_000_000i64) {
        errors.push(FieldError::new(
            "value",
            "must fit a positive amount with 10 integer digits and 4 decimal places",
        ));
    }
    if new.effective_until.is_some_and(|d| d < new.effective_from) {
        errors.push(FieldError::new(
            "effective_until",
            "must be on or after the start date",
        ));
    }
    if !errors.is_empty() {
        return Err(Refusal::Invalid(errors));
    }
    Ok((code, name, unit, currency))
}
async fn get(tx: &MutationTransaction<'_>, scope: &Scope, company: i64, id: i64) -> Result<Value> {
    tx.sql()
        .query_opt(LOCK_RULE, &[&scope.tenant(), &company, &id])
        .await
        .map_err(Error::from)?
        .map(|r| r.get(0))
        .ok_or(Refusal::NotFound)
}
async fn end(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company: i64,
    old: Value,
    until: Date,
) -> Result<AllowanceRule> {
    let current: AllowanceRule = decode(old.clone())?;
    if until.to_string() < current.effective_from {
        return Err(Refusal::Invalid(vec![FieldError::new(
            "effective_until",
            "must be on or after the start date",
        )]));
    }
    let tenant = scope.tenant();
    let date = until.to_string();
    let id = current.id;
    let after: Value = tx
        .mutate(move |sql| {
            Box::pin(async move {
                let new: Value = sql
                    .query_one(END_RULE, &[&tenant, &company, &id, &date])
                    .await?
                    .get(0);
                Ok((
                    new.clone(),
                    vec![Change {
                        kind: "people_attendance_allowance_policy".into(),
                        id,
                        old,
                        new,
                    }],
                ))
            })
        })
        .await?;
    action(
        tx,
        scope,
        "people.attendance.allowance_rule_ended",
        json!({
        "company_id":company,
        "rule_id":id,
        "code":current.code,
        "previous_effective_until":current.effective_until,
        "effective_until":until.to_string()}
        ),
    )
    .await?;
    decode(after)
}
impl Attendance {
    /// All versions, including retired ones, sorted by code and newest start. No attendance grant.
    pub async fn list_allowance_rules(
        &self,
        scope: &Scope,
        company_id: i64,
    ) -> Result<Vec<AllowanceRule>> {
        let c = self.db.get().await.map_err(Error::from)?;
        crate::access::current_company(&self.workforce, &**c, scope, company_id).await?;
        c.query(LIST_RULES, &[&scope.tenant(), &company_id])
            .await
            .map_err(Error::from)?
            .into_iter()
            .map(|r| decode(r.get(0)))
            .collect()
    }
    /// Read one stored version of the company, with no attendance grant.
    pub async fn get_allowance_rule(
        &self,
        scope: &Scope,
        company_id: i64,
        id: i64,
    ) -> Result<AllowanceRule> {
        self.list_allowance_rules(scope, company_id)
            .await?
            .into_iter()
            .find(|r| r.id == id)
            .ok_or(Refusal::NotFound)
    }
    /// Create an active version, ending an earlier open version and refusing any remaining overlap.
    pub async fn create_allowance_rule(
        &self,
        scope: &Scope,
        company_id: i64,
        new: &NewAllowanceRule,
    ) -> Result<AllowanceRule> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .mutations
            .begin(
                &mut c,
                scope.mutation_context("people.attendance.create_allowance_rule"),
            )
            .await?;
        lock(&tx, scope, company_id, ALLOWANCES_MANAGE).await?;
        let (code, name, unit, currency) = validate(new)?;
        // A retired version still owns its company/code/start key. Check before closing an older version.
        if tx
            .sql()
            .query_opt(
                TAKEN_START,
                &[
                    &scope.tenant(),
                    &company_id,
                    &code,
                    &new.effective_from.to_string(),
                ],
            )
            .await
            .map_err(Error::from)?
            .is_some()
        {
            return Err(Refusal::Invalid(vec![FieldError::new(
                "company_id",
                "has already been taken",
            )]));
        }
        if let Some(row) = tx
            .sql()
            .query_opt(
                OPEN_VERSION,
                &[
                    &scope.tenant(),
                    &company_id,
                    &code,
                    &new.effective_from.to_string(),
                ],
            )
            .await
            .map_err(Error::from)?
        {
            end(
                &mut tx,
                scope,
                company_id,
                row.get(0),
                new.effective_from
                    .checked_sub(jiff::Span::new().days(1))
                    .map_err(|_| Refusal::InvalidEndDate)?,
            )
            .await?;
        }
        let from = new.effective_from.to_string();
        let until = new.effective_until.map(|d| d.to_string());
        if tx
            .sql()
            .query_opt(
                OVERLAPPING_VERSION,
                &[&scope.tenant(), &company_id, &code, &from, &until],
            )
            .await
            .map_err(Error::from)?
            .is_some()
        {
            return Err(Refusal::EffectivePeriodOverlap);
        }
        let tenant = scope.tenant();
        let value = new.value;
        let saved: Value = tx
            .mutate(move |sql| {
                Box::pin(async move {
                    let new: Value = sql
                        .query_one(
                            INSERT_RULE,
                            &[
                                &tenant,
                                &company_id,
                                &code,
                                &name,
                                &unit,
                                &value,
                                &currency,
                                &from,
                                &until,
                            ],
                        )
                        .await?
                        .get(0);
                    Ok((
                        new.clone(),
                        vec![Change {
                            kind: "people_attendance_allowance_policy".into(),
                            id: new["id"].as_i64().ok_or(Error::Internal)?,
                            old: json!({}),
                            new,
                        }],
                    ))
                })
            })
            .await?;
        tx.commit().await?;
        decode(saved)
    }
    /// End an active version inclusively, only earlier, and record the ending action.
    pub async fn end_allowance_rule(
        &self,
        scope: &Scope,
        company_id: i64,
        id: i64,
        until: Date,
    ) -> Result<AllowanceRule> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .mutations
            .begin(
                &mut c,
                scope.mutation_context("people.attendance.end_allowance_rule"),
            )
            .await?;
        lock(&tx, scope, company_id, ALLOWANCES_MANAGE).await?;
        let old = get(&tx, scope, company_id, id).await?;
        let rule: AllowanceRule = decode(old.clone())?;
        if rule.status != "active" {
            return Err(Refusal::NotFound);
        }
        if rule
            .effective_until
            .as_ref()
            .is_some_and(|d| until.to_string() >= *d)
        {
            return Err(Refusal::InvalidEndDate);
        }
        let saved = end(&mut tx, scope, company_id, old, until).await?;
        tx.commit().await?;
        Ok(saved)
    }
    /// Retire a version without deleting its history or freeing its code/start key.
    pub async fn retire_allowance_rule(
        &self,
        scope: &Scope,
        company_id: i64,
        id: i64,
    ) -> Result<AllowanceRule> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .mutations
            .begin(
                &mut c,
                scope.mutation_context("people.attendance.retire_allowance_rule"),
            )
            .await?;
        lock(&tx, scope, company_id, ALLOWANCES_MANAGE).await?;
        let old = get(&tx, scope, company_id, id).await?;
        let tenant = scope.tenant();
        let saved: Value = tx
            .mutate(move |sql| {
                Box::pin(async move {
                    let new: Value = sql
                        .query_one(RETIRE_RULE, &[&tenant, &company_id, &id])
                        .await?
                        .get(0);
                    Ok((
                        new.clone(),
                        vec![Change {
                            kind: "people_attendance_allowance_policy".into(),
                            id,
                            old,
                            new,
                        }],
                    ))
                })
            })
            .await?;
        tx.commit().await?;
        decode(saved)
    }
}

const LOCK_RULE: &str = concat!(
    "SELECT to_jsonb(r) || jsonb_build_object('value',value::text) FROM ",
    "people_attendance_allowance_policies r WHERE tenant_id=$1 AND company_id=$2 AND id=$3 FOR ",
    "UPDATE",
);

const END_RULE: &str = concat!(
    "UPDATE people_attendance_allowance_policies r SET ",
    "effective_until=$4::text::date,updated_at=now() WHERE tenant_id=$1 AND company_id=$2 AND id=$3 ",
    "RETURNING to_jsonb(r) || jsonb_build_object('value',value::text)",
);

const LIST_RULES: &str = concat!(
    "SELECT to_jsonb(r) || jsonb_build_object('value',value::text) FROM ",
    "people_attendance_allowance_policies r WHERE tenant_id=$1 AND company_id=$2 ORDER BY ",
    "code,effective_from DESC,id",
);

const TAKEN_START: &str = concat!(
    "SELECT 1 FROM people_attendance_allowance_policies WHERE tenant_id=$1 AND company_id=$2 AND ",
    "code=$3 AND effective_from=$4::text::date",
);

const OPEN_VERSION: &str = concat!(
    "SELECT to_jsonb(r) || jsonb_build_object('value',value::text) FROM ",
    "people_attendance_allowance_policies r WHERE tenant_id=$1 AND company_id=$2 AND code=$3 AND ",
    "status='active' AND effective_until IS NULL AND effective_from<$4::text::date FOR UPDATE",
);

const OVERLAPPING_VERSION: &str = concat!(
    "SELECT 1 FROM people_attendance_allowance_policies WHERE tenant_id=$1 AND company_id=$2 AND ",
    "code=$3 AND status='active' AND (effective_until IS NULL OR effective_until >= $4::text::date) ",
    "AND ($5::text::date IS NULL OR effective_from <= $5::text::date)",
);

const INSERT_RULE: &str = concat!(
    "INSERT INTO people_attendance_allowance_policies AS r ",
    "(tenant_id,company_id,code,name,unit,value,currency,effective_from,effective_until) ",
    "VALUES($1,$2,$3,$4,$5,$6,$7,$8::text::date,$9::text::date) RETURNING to_jsonb(r) || ",
    "jsonb_build_object('value',value::text)",
);

const RETIRE_RULE: &str = concat!(
    "UPDATE people_attendance_allowance_policies r SET status='retired',updated_at=now() WHERE ",
    "tenant_id=$1 AND company_id=$2 AND id=$3 RETURNING to_jsonb(r) || ",
    "jsonb_build_object('value',value::text)",
);
