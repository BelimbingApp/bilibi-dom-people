//! The `people_claim_policy_versions` table: its rows, the SQL that reads,
//! locks and tests them, the open-claim-types join, and the validation and
//! writes behind the facade. Private.
use crate::{
    currency_shaped, date_column, money, period_field, required_date, ClaimType, FieldError,
    NewPolicy, OpenClaimType, Policy, Result, Tx,
};
use bilibi_database::{Change, Error};
use jiff::civil::Date;
use rust_decimal::Decimal;
use serde_json::{json, Value};
use tokio_postgres::{GenericClient, Row};

/// The policy columns, dates as ISO text.
const COLUMNS: &str = "p.id, p.claim_type_id, to_char(p.effective_from, 'YYYY-MM-DD') AS effective_from, \
    to_char(p.effective_to, 'YYYY-MM-DD') AS effective_to, p.currency, p.per_claim_limit, p.monthly_limit, \
    p.yearly_limit, p.receipt_threshold";

fn read(row: &Row) -> Result<Policy> {
    Ok(Policy {
        id: row.get("id"),
        claim_type_id: row.get("claim_type_id"),
        effective_from: required_date(row, "effective_from")?,
        effective_to: date_column(row, "effective_to")?,
        currency: row.get("currency"),
        per_claim_limit: row.get("per_claim_limit"),
        monthly_limit: row.get("monthly_limit"),
        yearly_limit: row.get("yearly_limit"),
        receipt_threshold: row.get("receipt_threshold"),
    })
}

impl Policy {
    pub(crate) fn audit_values(&self) -> Value {
        json!({
            "claim_type_id": self.claim_type_id,
            "effective_from": self.effective_from.to_string(),
            "effective_to": self.effective_to.map(|date| date.to_string()),
            "currency": self.currency,
            "per_claim_limit": money::text(self.per_claim_limit),
            "monthly_limit": money::text(self.monthly_limit),
            "yearly_limit": money::text(self.yearly_limit),
            "receipt_threshold": money::text(self.receipt_threshold),
        })
    }
}

pub(crate) async fn list(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
) -> Result<Vec<Policy>> {
    client
        .query(
            &format!("SELECT {COLUMNS} FROM people_claim_policy_versions p WHERE p.tenant_id = $1 AND p.company_id = $2 ORDER BY p.claim_type_id, p.effective_from DESC"),
            &[&tenant, &company_id],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(read)
        .collect()
}

pub(crate) async fn find(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    id: i64,
) -> Result<Option<Policy>> {
    if id <= 0 {
        return Ok(None);
    }
    client
        .query_opt(
            &format!("SELECT {COLUMNS} FROM people_claim_policy_versions p WHERE p.tenant_id = $1 AND p.company_id = $2 AND p.id = $3"),
            &[&tenant, &company_id, &id],
        )
        .await
        .map_err(Error::from)?
        .as_ref()
        .map(read)
        .transpose()
}

pub(crate) async fn lock(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    id: i64,
) -> Result<Option<Policy>> {
    client
        .query_opt(
            &format!("SELECT {COLUMNS} FROM people_claim_policy_versions p WHERE p.tenant_id = $1 AND p.company_id = $2 AND p.id = $3 FOR UPDATE"),
            &[&tenant, &company_id, &id],
        )
        .await
        .map_err(Error::from)?
        .as_ref()
        .map(read)
        .transpose()
}

/// Whether another policy of the claim type covers any day of the period
/// (Bilimbi `no_overlap/3`).
pub(crate) async fn overlaps(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    claim_type_id: i64,
    from: Date,
    to: Option<Date>,
) -> Result<bool> {
    let from = from.to_string();
    let to = to.map(|date| date.to_string());
    let row = client
        .query_opt(
            "SELECT 1 FROM people_claim_policy_versions \
             WHERE tenant_id = $1 AND company_id = $2 AND claim_type_id = $3 \
               AND (effective_to IS NULL OR effective_to >= $4::text::date) \
               AND ($5::text IS NULL OR effective_from <= $5::text::date) \
             LIMIT 1",
            &[&tenant, &company_id, &claim_type_id, &from, &to],
        )
        .await
        .map_err(Error::from)?;
    Ok(row.is_some())
}

/// Whether a live claim under the policy was incurred after `effective_to`
/// (Bilimbi `requests_after?/4`).
pub(crate) async fn requests_after(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    policy_id: i64,
    effective_to: Date,
) -> Result<bool> {
    let effective_to = effective_to.to_string();
    let row = client
        .query_opt(
            "SELECT 1 FROM people_claim_submissions \
             WHERE tenant_id = $1 AND company_id = $2 AND claim_policy_id = $3 \
               AND status NOT IN ('withdrawn', 'rejected') AND incurred_on > $4::text::date \
             LIMIT 1",
            &[&tenant, &company_id, &policy_id, &effective_to],
        )
        .await
        .map_err(Error::from)?;
    Ok(row.is_some())
}

/// A policy once validated against the company's currencies.
pub(crate) struct Valid {
    pub effective_from: Date,
    pub effective_to: Option<Date>,
    pub currency: String,
    pub per_claim_limit: Option<Decimal>,
    pub monthly_limit: Option<Decimal>,
    pub yearly_limit: Option<Decimal>,
    pub receipt_threshold: Option<Decimal>,
}

/// Bilimbi `Policy.changeset/3`, in its words.
pub(crate) fn validate(
    new: &NewPolicy,
    allowed_currencies: &[String],
) -> std::result::Result<Valid, Vec<FieldError>> {
    let mut errors = Vec::new();
    if new.claim_type_id <= 0 {
        errors.push(FieldError::new("claim_type_id", "can't be blank"));
    }
    let currency = new.currency.trim().to_ascii_uppercase();
    if currency.is_empty() {
        errors.push(FieldError::new("currency", "can't be blank"));
    } else if !currency_shaped(&currency) || !allowed_currencies.contains(&currency) {
        errors.push(FieldError::new(
            "currency",
            "is not an allowed claim currency for this company",
        ));
    }
    period_field(&mut errors, new.effective_from, new.effective_to);
    money::validate_amount(
        &mut errors,
        "per_claim_limit",
        new.per_claim_limit,
        money::Min::Positive,
    );
    money::validate_amount(
        &mut errors,
        "monthly_limit",
        new.monthly_limit,
        money::Min::Positive,
    );
    money::validate_amount(
        &mut errors,
        "yearly_limit",
        new.yearly_limit,
        money::Min::Positive,
    );
    money::validate_amount(
        &mut errors,
        "receipt_threshold",
        new.receipt_threshold,
        money::Min::Zero,
    );
    if errors.is_empty() {
        Ok(Valid {
            effective_from: new.effective_from,
            effective_to: new.effective_to,
            currency,
            per_claim_limit: new.per_claim_limit,
            monthly_limit: new.monthly_limit,
            yearly_limit: new.yearly_limit,
            receipt_threshold: new.receipt_threshold,
        })
    } else {
        Err(errors)
    }
}

/// A threshold rule needs the threshold; any other rule drops it (Bilimbi
/// `receipt_threshold_rule/2`).
pub(crate) fn apply_receipt_rule(
    valid: &mut Valid,
    claim_type: &ClaimType,
) -> std::result::Result<(), Vec<FieldError>> {
    if claim_type.receipt_requirement == "above_threshold" {
        if valid.receipt_threshold.is_none() {
            return Err(vec![FieldError::new("receipt_threshold", "can't be blank")]);
        }
    } else {
        valid.receipt_threshold = None;
    }
    Ok(())
}

pub(crate) async fn insert(
    tx: &mut Tx<'_>,
    tenant: i64,
    company_id: i64,
    claim_type_id: i64,
    valid: Valid,
) -> Result<Policy> {
    tx.mutate(move |sql| {
        Box::pin(async move {
            let from = valid.effective_from.to_string();
            let to = valid.effective_to.map(|date| date.to_string());
            let row = sql
                .query_one(
                    &format!(
                        "WITH p AS (INSERT INTO people_claim_policy_versions \
                         (tenant_id, company_id, claim_type_id, effective_from, effective_to, currency, \
                          per_claim_limit, monthly_limit, yearly_limit, receipt_threshold) \
                         VALUES ($1, $2, $3, $4::text::date, $5::text::date, $6, $7, $8, $9, $10) RETURNING *) \
                         SELECT {COLUMNS} FROM p"
                    ),
                    &[
                        &tenant,
                        &company_id,
                        &claim_type_id,
                        &from,
                        &to,
                        &valid.currency,
                        &valid.per_claim_limit,
                        &valid.monthly_limit,
                        &valid.yearly_limit,
                        &valid.receipt_threshold,
                    ],
                )
                .await?;
            let policy = read(&row).map_err(|_| Error::Internal)?;
            let change = Change {
                kind: "people_claim_policy_version".into(),
                id: policy.id,
                old: json!({}),
                new: policy.audit_values(),
            };
            Ok((policy, vec![change]))
        })
    })
    .await
    .map_err(Into::into)
}

pub(crate) async fn end(tx: &mut Tx<'_>, policy: Policy, effective_to: Date) -> Result<Policy> {
    tx.mutate(move |sql| {
        Box::pin(async move {
            let to = effective_to.to_string();
            let row = sql
                .query_one(
                    &format!(
                        "WITH p AS (UPDATE people_claim_policy_versions SET effective_to = $2::text::date, \
                         updated_at = now() AT TIME ZONE 'UTC' WHERE id = $1 RETURNING *) \
                         SELECT {COLUMNS} FROM p"
                    ),
                    &[&policy.id, &to],
                )
                .await?;
            let ended = read(&row).map_err(|_| Error::Internal)?;
            let change = Change {
                kind: "people_claim_policy_version".into(),
                id: ended.id,
                old: policy.audit_values(),
                new: ended.audit_values(),
            };
            Ok((ended, vec![change]))
        })
    })
    .await
    .map_err(Into::into)
}

/// Active claim types of active categories with a policy in effect on
/// `on_date`, by category and type name (Bilimbi `open_claim_types/4`).
/// With `assigned`, an `assigned_only` type appears only when its id is in
/// the list.
pub(crate) async fn open_types(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    on_date: Date,
    assigned: Option<&[i64]>,
) -> Result<Vec<OpenClaimType>> {
    let on_date = on_date.to_string();
    let filter = if assigned.is_some() {
        " AND (t.eligibility = 'all_employees' OR t.id = ANY($4::bigint[]))"
    } else {
        ""
    };
    let sql = format!(
        "SELECT t.id AS type_id, t.category_id, t.code, t.name, t.receipt_requirement, t.eligibility, t.active, \
                c.name AS category_name, {COLUMNS} \
         FROM people_claim_catalog_types t \
         JOIN people_claim_catalog_groups c ON c.id = t.category_id AND c.tenant_id = t.tenant_id \
         JOIN people_claim_policy_versions p ON p.claim_type_id = t.id AND p.tenant_id = t.tenant_id \
         WHERE t.tenant_id = $1 AND t.company_id = $2 AND t.active AND c.active \
           AND p.effective_from <= $3::text::date AND (p.effective_to IS NULL OR p.effective_to >= $3::text::date){filter} \
         ORDER BY c.name, t.name, t.id"
    );
    let rows = match assigned {
        Some(ids) => {
            client
                .query(&sql, &[&tenant, &company_id, &on_date, &ids])
                .await
        }
        None => client.query(&sql, &[&tenant, &company_id, &on_date]).await,
    }
    .map_err(Error::from)?;
    rows.iter()
        .map(|row| {
            Ok(OpenClaimType {
                claim_type: ClaimType {
                    id: row.get("type_id"),
                    category_id: row.get("category_id"),
                    code: row.get("code"),
                    name: row.get("name"),
                    receipt_requirement: row.get("receipt_requirement"),
                    eligibility: row.get("eligibility"),
                    active: row.get("active"),
                },
                category_name: row.get("category_name"),
                policy: read(row)?,
            })
        })
        .collect()
}
