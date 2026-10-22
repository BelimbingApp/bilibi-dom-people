//! The `people_skill_categories` and `people_skills` tables: their rows, the
//! SQL that reads them and the writes behind the facade's category and skill
//! functions. Private to the facade.
use crate::{
    rows,
    validate::{ValidCategory, ValidSkill, ValidSkillRevision},
    Category, Result, Skill, SkillsError, Write,
};
use bilibi_database::{Error, Result as DbResult};
use tokio_postgres::{GenericClient, Row};

const CATEGORY: &str = "people_skill_category";
const SKILL: &str = "people_skill";
const CATEGORY_COLUMNS: &str = "id, code, name, description, active";
const SKILL_COLUMNS: &str =
    "id, category_id, code, name, definition, evidence_guide, critical, reassessment_months, active";

fn category(row: &Row) -> Category {
    Category {
        id: row.get("id"),
        code: row.get("code"),
        name: row.get("name"),
        description: row.get("description"),
        active: row.get("active"),
    }
}

fn skill(row: &Row) -> Skill {
    Skill {
        id: row.get("id"),
        category_id: row.get("category_id"),
        code: row.get("code"),
        name: row.get("name"),
        definition: row.get("definition"),
        evidence_guide: row.get("evidence_guide"),
        critical: row.get("critical"),
        reassessment_months: row.get("reassessment_months"),
        active: row.get("active"),
    }
}

// ---- Categories ------------------------------------------------------------

/// The company's categories: active first, then by name, then by id.
pub(crate) async fn list_categories(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
) -> Result<Vec<Category>> {
    Ok(client
        .query(
            &format!("SELECT {CATEGORY_COLUMNS} FROM people_skill_categories WHERE tenant_id = $1 AND company_id = $2 ORDER BY active DESC, name, id"),
            &[&tenant, &company_id],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(category)
        .collect())
}

/// One category of the company, or `None`.
pub(crate) async fn get_category(w: &Write<'_>, id: i64) -> Result<Option<Category>> {
    Ok(w.sql()
        .query_opt(
            &format!("SELECT {CATEGORY_COLUMNS} FROM people_skill_categories WHERE tenant_id = $1 AND company_id = $2 AND id = $3"),
            &[&w.tenant, &w.company_id, &id],
        )
        .await
        .map_err(Error::from)?
        .as_ref()
        .map(category))
}

/// One category of the company, locked for the write, or `None`.
pub(crate) async fn lock_category(w: &Write<'_>, id: i64) -> Result<Option<Category>> {
    Ok(w.sql()
        .query_opt(
            &format!("SELECT {CATEGORY_COLUMNS} FROM people_skill_categories WHERE tenant_id = $1 AND company_id = $2 AND id = $3 FOR UPDATE"),
            &[&w.tenant, &w.company_id, &id],
        )
        .await
        .map_err(Error::from)?
        .as_ref()
        .map(category))
}

/// Whether the category still has an active skill.
pub(crate) async fn has_active_skills(w: &Write<'_>, category_id: i64) -> Result<bool> {
    Ok(w.sql()
        .query_opt(
            "SELECT 1 FROM people_skills WHERE tenant_id = $1 AND category_id = $2 AND active LIMIT 1",
            &[&w.tenant, &category_id],
        )
        .await
        .map_err(Error::from)?
        .is_some())
}

/// Bilimbi `require_category/4`: a category of the skill's company, and an
/// active one when the skill is (or becomes) active; otherwise
/// `CategoryUnavailable`.
pub(crate) async fn require_category(
    w: &Write<'_>,
    category_id: i64,
    must_be_active: bool,
) -> Result<Category> {
    match get_category(w, category_id).await? {
        Some(category) if category.active || !must_be_active => Ok(category),
        _ => Err(SkillsError::CategoryUnavailable),
    }
}

async fn taken(
    client: &impl GenericClient,
    table: &str,
    company_id: i64,
    code: &str,
) -> DbResult<bool> {
    Ok(client
        .query_opt(
            &format!("SELECT 1 FROM {table} WHERE company_id = $1 AND code = $2"),
            &[&company_id, &code],
        )
        .await?
        .is_some())
}

fn code_taken() -> SkillsError {
    SkillsError::Invalid(vec![crate::FieldError::new(
        "code",
        "has already been taken",
    )])
}

pub(crate) async fn insert_category(w: &mut Write<'_>, valid: &ValidCategory) -> Result<Category> {
    if taken(
        w.sql(),
        "people_skill_categories",
        w.company_id,
        &valid.code,
    )
    .await?
    {
        return Err(code_taken());
    }
    let (tenant, company_id, valid) = (w.tenant, w.company_id, valid.clone());
    Ok(w.tx
        .mutate(move |sql| {
            Box::pin(async move {
                let row = sql
                    .query_one(
                        &format!("INSERT INTO people_skill_categories (tenant_id, company_id, code, name, description, active) VALUES ($1, $2, $3, $4, $5, true) RETURNING {CATEGORY_COLUMNS}"),
                        &[&tenant, &company_id, &valid.code, &valid.name, &valid.description],
                    )
                    .await?;
                let created = category(&row);
                let change = rows::created(CATEGORY, created.id, &created);
                Ok((created, vec![change]))
            })
        })
        .await?)
}

pub(crate) async fn update_category(
    w: &mut Write<'_>,
    current: Category,
    valid: ValidCategory,
) -> Result<Category> {
    Ok(w.tx
        .mutate(move |sql| {
            Box::pin(async move {
                let row = sql
                    .query_one(
                        &format!("UPDATE people_skill_categories SET name = $2, description = $3, updated_at = (now() AT TIME ZONE 'UTC') WHERE id = $1 RETURNING {CATEGORY_COLUMNS}"),
                        &[&current.id, &valid.name, &valid.description],
                    )
                    .await?;
                let updated = category(&row);
                let change = rows::changed(CATEGORY, current.id, &current, &updated);
                Ok((updated, vec![change]))
            })
        })
        .await?)
}

pub(crate) async fn set_category_active(
    w: &mut Write<'_>,
    current: Category,
    active: bool,
) -> Result<Category> {
    Ok(w.tx
        .mutate(move |sql| {
            Box::pin(async move {
                let row = sql
                    .query_one(
                        &format!("UPDATE people_skill_categories SET active = $2, updated_at = (now() AT TIME ZONE 'UTC') WHERE id = $1 RETURNING {CATEGORY_COLUMNS}"),
                        &[&current.id, &active],
                    )
                    .await?;
                let updated = category(&row);
                let change = rows::changed(CATEGORY, current.id, &current, &updated);
                Ok((updated, vec![change]))
            })
        })
        .await?)
}

// ---- Skills ----------------------------------------------------------------

/// The company's skills: active first, then by name, then by id.
pub(crate) async fn list_skills(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
) -> Result<Vec<Skill>> {
    Ok(client
        .query(
            &format!("SELECT {SKILL_COLUMNS} FROM people_skills WHERE tenant_id = $1 AND company_id = $2 ORDER BY active DESC, name, id"),
            &[&tenant, &company_id],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(skill)
        .collect())
}

/// One skill of the company, or `None`.
pub(crate) async fn get_skill(w: &Write<'_>, id: i64) -> Result<Option<Skill>> {
    Ok(w.sql()
        .query_opt(
            &format!("SELECT {SKILL_COLUMNS} FROM people_skills WHERE tenant_id = $1 AND company_id = $2 AND id = $3"),
            &[&w.tenant, &w.company_id, &id],
        )
        .await
        .map_err(Error::from)?
        .as_ref()
        .map(skill))
}

/// The company's skills among `ids`, by id.
pub(crate) async fn skills_by_ids(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    ids: &[i64],
) -> Result<Vec<Skill>> {
    Ok(client
        .query(
            &format!("SELECT {SKILL_COLUMNS} FROM people_skills WHERE tenant_id = $1 AND company_id = $2 AND id = ANY($3::bigint[]) ORDER BY id"),
            &[&tenant, &company_id, &ids],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(skill)
        .collect())
}

/// One skill of the company, locked for the write, or `None`.
pub(crate) async fn lock_skill(w: &Write<'_>, id: i64) -> Result<Option<Skill>> {
    Ok(w.sql()
        .query_opt(
            &format!("SELECT {SKILL_COLUMNS} FROM people_skills WHERE tenant_id = $1 AND company_id = $2 AND id = $3 FOR UPDATE"),
            &[&w.tenant, &w.company_id, &id],
        )
        .await
        .map_err(Error::from)?
        .as_ref()
        .map(skill))
}

pub(crate) async fn insert_skill(w: &mut Write<'_>, valid: &ValidSkill) -> Result<Skill> {
    if taken(w.sql(), "people_skills", w.company_id, &valid.code).await? {
        return Err(code_taken());
    }
    let (tenant, company_id, valid) = (w.tenant, w.company_id, valid.clone());
    Ok(w.tx
        .mutate(move |sql| {
            Box::pin(async move {
                let row = sql
                    .query_one(
                        &format!("INSERT INTO people_skills (tenant_id, company_id, category_id, code, name, definition, evidence_guide, critical, reassessment_months, active) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, true) RETURNING {SKILL_COLUMNS}"),
                        &[
                            &tenant,
                            &company_id,
                            &valid.category_id,
                            &valid.code,
                            &valid.name,
                            &valid.definition,
                            &valid.evidence_guide,
                            &valid.critical,
                            &valid.reassessment_months,
                        ],
                    )
                    .await?;
                let created = skill(&row);
                let change = rows::created(SKILL, created.id, &created);
                Ok((created, vec![change]))
            })
        })
        .await?)
}

pub(crate) async fn update_skill(
    w: &mut Write<'_>,
    current: Skill,
    valid: ValidSkillRevision,
) -> Result<Skill> {
    Ok(w.tx
        .mutate(move |sql| {
            Box::pin(async move {
                let row = sql
                    .query_one(
                        &format!("UPDATE people_skills SET category_id = $2, name = $3, definition = $4, evidence_guide = $5, critical = $6, reassessment_months = $7, updated_at = (now() AT TIME ZONE 'UTC') WHERE id = $1 RETURNING {SKILL_COLUMNS}"),
                        &[
                            &current.id,
                            &valid.category_id,
                            &valid.name,
                            &valid.definition,
                            &valid.evidence_guide,
                            &valid.critical,
                            &valid.reassessment_months,
                        ],
                    )
                    .await?;
                let updated = skill(&row);
                let change = rows::changed(SKILL, current.id, &current, &updated);
                Ok((updated, vec![change]))
            })
        })
        .await?)
}

pub(crate) async fn set_skill_active(
    w: &mut Write<'_>,
    current: Skill,
    active: bool,
) -> Result<Skill> {
    Ok(w.tx
        .mutate(move |sql| {
            Box::pin(async move {
                let row = sql
                    .query_one(
                        &format!("UPDATE people_skills SET active = $2, updated_at = (now() AT TIME ZONE 'UTC') WHERE id = $1 RETURNING {SKILL_COLUMNS}"),
                        &[&current.id, &active],
                    )
                    .await?;
                let updated = skill(&row);
                let change = rows::changed(SKILL, current.id, &current, &updated);
                Ok((updated, vec![change]))
            })
        })
        .await?)
}
