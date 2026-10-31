//! The claim catalog tables, `people_claim_catalog_groups` (categories) and
//! `people_claim_catalog_types`: their rows, the SQL that reads and locks
//! them, and the validation and writes behind the facade. Private.
use crate::{
    text_field, Category, ClaimType, ClaimsError, FieldError, NewCategory, NewClaimType, Result,
    Tx, ELIGIBILITIES, RECEIPT_REQUIREMENTS,
};
use bilibi_database::{Change, Error};
use serde_json::{json, Value};
use tokio_postgres::{GenericClient, Row};

const CATEGORY_COLUMNS: &str = "id, code, name, active";
const TYPE_COLUMNS: &str = "id, category_id, code, name, receipt_requirement, eligibility, active";

fn read_category(row: &Row) -> Category {
    Category {
        id: row.get("id"),
        code: row.get("code"),
        name: row.get("name"),
        active: row.get("active"),
    }
}

fn read_claim_type(row: &Row) -> ClaimType {
    ClaimType {
        id: row.get("id"),
        category_id: row.get("category_id"),
        code: row.get("code"),
        name: row.get("name"),
        receipt_requirement: row.get("receipt_requirement"),
        eligibility: row.get("eligibility"),
        active: row.get("active"),
    }
}

impl Category {
    pub(crate) fn audit_values(&self) -> Value {
        json!({ "code": self.code, "name": self.name, "active": self.active })
    }
}

impl ClaimType {
    pub(crate) fn audit_values(&self) -> Value {
        json!({
            "category_id": self.category_id,
            "code": self.code,
            "name": self.name,
            "receipt_requirement": self.receipt_requirement,
            "eligibility": self.eligibility,
            "active": self.active,
        })
    }
}

// Categories

pub(crate) async fn categories(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
) -> Result<Vec<Category>> {
    Ok(client
        .query(
            &format!("SELECT {CATEGORY_COLUMNS} FROM people_claim_catalog_groups WHERE tenant_id = $1 AND company_id = $2 ORDER BY name, id"),
            &[&tenant, &company_id],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(read_category)
        .collect())
}

pub(crate) async fn find_category(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    id: i64,
) -> Result<Option<Category>> {
    if id <= 0 {
        return Ok(None);
    }
    Ok(client
        .query_opt(
            &format!("SELECT {CATEGORY_COLUMNS} FROM people_claim_catalog_groups WHERE tenant_id = $1 AND company_id = $2 AND id = $3"),
            &[&tenant, &company_id, &id],
        )
        .await
        .map_err(Error::from)?
        .as_ref()
        .map(read_category))
}

pub(crate) async fn lock_category(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    id: i64,
) -> Result<Option<Category>> {
    if id <= 0 {
        return Ok(None);
    }
    Ok(client
        .query_opt(
            &format!("SELECT {CATEGORY_COLUMNS} FROM people_claim_catalog_groups WHERE tenant_id = $1 AND company_id = $2 AND id = $3 FOR UPDATE"),
            &[&tenant, &company_id, &id],
        )
        .await
        .map_err(Error::from)?
        .as_ref()
        .map(read_category))
}

/// A category once validated: code and name trimmed, present and bounded.
pub(crate) struct ValidCategory {
    pub code: String,
    pub name: String,
}

/// Bilimbi `Category.changeset/2`, in its words.
pub(crate) fn validate_category(
    new: &NewCategory,
) -> std::result::Result<ValidCategory, Vec<FieldError>> {
    let mut errors = Vec::new();
    let code = text_field(&mut errors, "code", &new.code, 60);
    let name = text_field(&mut errors, "name", &new.name, 120);
    if errors.is_empty() {
        Ok(ValidCategory { code, name })
    } else {
        Err(errors)
    }
}

/// `has already been taken` for a code another category of the company holds.
pub(crate) async fn check_category_code(
    client: &(impl GenericClient + Sync),
    company_id: i64,
    code: &str,
) -> Result<()> {
    let taken = client
        .query_opt(
            "SELECT 1 FROM people_claim_catalog_groups WHERE company_id = $1 AND code = $2",
            &[&company_id, &code],
        )
        .await
        .map_err(Error::from)?;
    if taken.is_some() {
        return Err(ClaimsError::Invalid(vec![FieldError::new(
            "code",
            "has already been taken",
        )]));
    }
    Ok(())
}

pub(crate) async fn insert_category(
    tx: &mut Tx<'_>,
    tenant: i64,
    company_id: i64,
    valid: ValidCategory,
) -> Result<Category> {
    Ok(tx
        .mutate(move |sql| {
            Box::pin(async move {
                let row = sql
                    .query_one(
                        &format!("INSERT INTO people_claim_catalog_groups (tenant_id, company_id, code, name) VALUES ($1, $2, $3, $4) RETURNING {CATEGORY_COLUMNS}"),
                        &[&tenant, &company_id, &valid.code, &valid.name],
                    )
                    .await?;
                let category = read_category(&row);
                let change = Change {
                    kind: "people_claim_catalog_group".into(),
                    id: category.id,
                    old: json!({}),
                    new: category.audit_values(),
                };
                Ok((category, vec![change]))
            })
        })
        .await?)
}

pub(crate) async fn set_category_active(
    tx: &mut Tx<'_>,
    category: Category,
    active: bool,
) -> Result<Category> {
    Ok(tx
        .mutate(move |sql| {
            Box::pin(async move {
                let row = sql
                    .query_one(
                        &format!("UPDATE people_claim_catalog_groups SET active = $2, updated_at = now() AT TIME ZONE 'UTC' WHERE id = $1 RETURNING {CATEGORY_COLUMNS}"),
                        &[&category.id, &active],
                    )
                    .await?;
                let updated = read_category(&row);
                let change = Change {
                    kind: "people_claim_catalog_group".into(),
                    id: updated.id,
                    old: category.audit_values(),
                    new: updated.audit_values(),
                };
                Ok((updated, vec![change]))
            })
        })
        .await?)
}

// Claim types

pub(crate) async fn claim_types(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
) -> Result<Vec<ClaimType>> {
    Ok(client
        .query(
            &format!("SELECT {TYPE_COLUMNS} FROM people_claim_catalog_types WHERE tenant_id = $1 AND company_id = $2 ORDER BY name, id"),
            &[&tenant, &company_id],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(read_claim_type)
        .collect())
}

/// One claim type of the company, locked `FOR UPDATE` for the rest of the
/// transaction: the lock every policy write of the type takes.
pub(crate) async fn lock_claim_type(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    id: i64,
) -> Result<Option<ClaimType>> {
    if id <= 0 {
        return Ok(None);
    }
    Ok(client
        .query_opt(
            &format!("SELECT {TYPE_COLUMNS} FROM people_claim_catalog_types WHERE tenant_id = $1 AND company_id = $2 AND id = $3 FOR UPDATE"),
            &[&tenant, &company_id, &id],
        )
        .await
        .map_err(Error::from)?
        .as_ref()
        .map(read_claim_type))
}

/// Whether every id is a claim type of the company (Bilimbi `all_in_company/5`).
pub(crate) async fn all_claim_types_in_company(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    ids: &[i64],
) -> Result<bool> {
    if ids.is_empty() {
        return Ok(true);
    }
    let row = client
        .query_one(
            "SELECT count(*) FROM people_claim_catalog_types WHERE tenant_id = $1 AND company_id = $2 AND id = ANY($3::bigint[])",
            &[&tenant, &company_id, &ids],
        )
        .await
        .map_err(Error::from)?;
    let found: i64 = row.get(0);
    Ok(found == ids.len() as i64)
}

/// A claim type once validated.
pub(crate) struct ValidClaimType {
    pub code: String,
    pub name: String,
    pub receipt_requirement: String,
    pub eligibility: String,
}

/// Bilimbi `ClaimType.changeset/2`, in its words.
pub(crate) fn validate_claim_type(
    new: &NewClaimType,
) -> std::result::Result<ValidClaimType, Vec<FieldError>> {
    let mut errors = Vec::new();
    if new.category_id <= 0 {
        errors.push(FieldError::new("category_id", "can't be blank"));
    }
    let code = text_field(&mut errors, "code", &new.code, 60);
    let name = text_field(&mut errors, "name", &new.name, 120);
    let receipt_requirement = new.receipt_requirement.trim().to_owned();
    if receipt_requirement.is_empty() {
        errors.push(FieldError::new("receipt_requirement", "can't be blank"));
    } else if !RECEIPT_REQUIREMENTS.contains(&receipt_requirement.as_str()) {
        errors.push(FieldError::new("receipt_requirement", "is invalid"));
    }
    let eligibility = new
        .eligibility
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("all_employees")
        .to_owned();
    if !ELIGIBILITIES.contains(&eligibility.as_str()) {
        errors.push(FieldError::new("eligibility", "is invalid"));
    }
    if errors.is_empty() {
        Ok(ValidClaimType {
            code,
            name,
            receipt_requirement,
            eligibility,
        })
    } else {
        Err(errors)
    }
}

pub(crate) async fn check_claim_type_code(
    client: &(impl GenericClient + Sync),
    company_id: i64,
    code: &str,
) -> Result<()> {
    let taken = client
        .query_opt(
            "SELECT 1 FROM people_claim_catalog_types WHERE company_id = $1 AND code = $2",
            &[&company_id, &code],
        )
        .await
        .map_err(Error::from)?;
    if taken.is_some() {
        return Err(ClaimsError::Invalid(vec![FieldError::new(
            "code",
            "has already been taken",
        )]));
    }
    Ok(())
}

pub(crate) async fn insert_claim_type(
    tx: &mut Tx<'_>,
    tenant: i64,
    company_id: i64,
    category_id: i64,
    valid: ValidClaimType,
) -> Result<ClaimType> {
    Ok(tx
        .mutate(move |sql| {
            Box::pin(async move {
                let row = sql
                    .query_one(
                        &format!("INSERT INTO people_claim_catalog_types (tenant_id, company_id, category_id, code, name, receipt_requirement, eligibility) VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING {TYPE_COLUMNS}"),
                        &[
                            &tenant,
                            &company_id,
                            &category_id,
                            &valid.code,
                            &valid.name,
                            &valid.receipt_requirement,
                            &valid.eligibility,
                        ],
                    )
                    .await?;
                let claim_type = read_claim_type(&row);
                let change = Change {
                    kind: "people_claim_catalog_type".into(),
                    id: claim_type.id,
                    old: json!({}),
                    new: claim_type.audit_values(),
                };
                Ok((claim_type, vec![change]))
            })
        })
        .await?)
}

pub(crate) async fn set_claim_type_active(
    tx: &mut Tx<'_>,
    claim_type: ClaimType,
    active: bool,
) -> Result<ClaimType> {
    Ok(tx
        .mutate(move |sql| {
            Box::pin(async move {
                let row = sql
                    .query_one(
                        &format!("UPDATE people_claim_catalog_types SET active = $2, updated_at = now() AT TIME ZONE 'UTC' WHERE id = $1 RETURNING {TYPE_COLUMNS}"),
                        &[&claim_type.id, &active],
                    )
                    .await?;
                let updated = read_claim_type(&row);
                let change = Change {
                    kind: "people_claim_catalog_type".into(),
                    id: updated.id,
                    old: claim_type.audit_values(),
                    new: updated.audit_values(),
                };
                Ok((updated, vec![change]))
            })
        })
        .await?)
}
