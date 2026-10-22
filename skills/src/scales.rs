//! The `people_skill_scales` and `people_skill_scale_levels` tables: their
//! rows, the SQL that reads them and the writes behind the facade's scale
//! functions. Private to the facade. The database guards (migration 001)
//! refuse what these writes never attempt: an edit to a published or retired
//! scale or to its levels.
use crate::{rows, Item, Level, Result, Scale, SkillsError, Write};
use bilibi_database::{Change, Error, Result as DbResult};
use serde_json::{json, Value};
use tokio_postgres::{GenericClient, Row, Transaction};

const SCALE: &str = "people_skill_scale";
const LEVEL: &str = "people_skill_scale_level";
const LEVEL_COLUMNS: &str = "id, scale_id, level, name, anchor, authority";

fn columns() -> String {
    format!(
        "id, code, name, version, status, {}, {}",
        rows::datetime_text("published_at"),
        rows::datetime_text("retired_at")
    )
}

fn scale(row: &Row) -> DbResult<Scale> {
    Ok(Scale {
        id: row.get("id"),
        code: row.get("code"),
        name: row.get("name"),
        version: row.get("version"),
        status: row.get("status"),
        published_at: rows::datetime(row, "published_at")?,
        retired_at: rows::datetime(row, "retired_at")?,
        levels: Vec::new(),
    })
}

/// The scale row as audit records it (its levels are their own rows).
fn audit(scale: &Scale) -> Value {
    json!({
        "code": scale.code,
        "name": scale.name,
        "version": scale.version,
        "status": scale.status,
        "published_at": scale.published_at.map(|at| at.to_string()),
        "retired_at": scale.retired_at.map(|at| at.to_string()),
    })
}

/// One level with its row id, which the audit change names.
pub(crate) struct LevelRow {
    pub id: i64,
    pub level: Level,
}

fn level_row(row: &Row) -> LevelRow {
    LevelRow {
        id: row.get("id"),
        level: Level {
            level: row.get("level"),
            name: row.get("name"),
            anchor: row.get("anchor"),
            authority: row.get("authority"),
        },
    }
}

fn level_audit(scale_id: i64, level: &Level) -> Value {
    json!({
        "scale_id": scale_id,
        "level": level.level,
        "name": level.name,
        "anchor": level.anchor,
        "authority": level.authority,
    })
}

// ---- Reads -----------------------------------------------------------------

/// Scale versions with their levels: by code, newest version first.
pub(crate) async fn list(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
) -> Result<Vec<Scale>> {
    let mut scales = client
        .query(
            &format!("SELECT {} FROM people_skill_scales WHERE tenant_id = $1 AND company_id = $2 ORDER BY code, version DESC", columns()),
            &[&tenant, &company_id],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(scale)
        .collect::<DbResult<Vec<_>>>()?;
    let ids: Vec<i64> = scales.iter().map(|scale| scale.id).collect();
    let levels = client
        .query(
            &format!("SELECT {LEVEL_COLUMNS} FROM people_skill_scale_levels WHERE scale_id = ANY($1::bigint[]) ORDER BY scale_id, level"),
            &[&ids],
        )
        .await
        .map_err(Error::from)?;
    for row in &levels {
        let scale_id: i64 = row.get("scale_id");
        if let Some(scale) = scales.iter_mut().find(|scale| scale.id == scale_id) {
            scale.levels.push(level_row(row).level);
        }
    }
    Ok(scales)
}

/// One scale of the company without its levels, or `None`.
pub(crate) async fn get(w: &Write<'_>, id: i64) -> Result<Option<Scale>> {
    w.sql()
        .query_opt(
            &format!("SELECT {} FROM people_skill_scales WHERE tenant_id = $1 AND company_id = $2 AND id = $3", columns()),
            &[&w.tenant, &w.company_id, &id],
        )
        .await
        .map_err(Error::from)?
        .as_ref()
        .map(scale)
        .transpose()
        .map_err(SkillsError::from)
}

/// One scale of the company, locked for the write, or `None`.
pub(crate) async fn lock(w: &Write<'_>, id: i64) -> Result<Option<Scale>> {
    w.sql()
        .query_opt(
            &format!("SELECT {} FROM people_skill_scales WHERE tenant_id = $1 AND company_id = $2 AND id = $3 FOR UPDATE", columns()),
            &[&w.tenant, &w.company_id, &id],
        )
        .await
        .map_err(Error::from)?
        .as_ref()
        .map(scale)
        .transpose()
        .map_err(SkillsError::from)
}

/// Bilimbi `lock_draft/4`: the locked scale, which must be a draft.
pub(crate) async fn lock_draft(w: &Write<'_>, id: i64) -> Result<Scale> {
    match lock(w, id).await? {
        None => Err(SkillsError::NotFound),
        Some(scale) if scale.status == "draft" => Ok(scale),
        Some(_) => Err(SkillsError::NotDraft),
    }
}

/// Whether any version of the company uses the code.
pub(crate) async fn code_exists(w: &Write<'_>, code: &str) -> Result<bool> {
    Ok(w.sql()
        .query_opt(
            "SELECT 1 FROM people_skill_scales WHERE tenant_id = $1 AND company_id = $2 AND code = $3 LIMIT 1",
            &[&w.tenant, &w.company_id, &code],
        )
        .await
        .map_err(Error::from)?
        .is_some())
}

/// Whether the code has an open draft.
pub(crate) async fn open_draft(w: &Write<'_>, code: &str) -> Result<bool> {
    Ok(w.sql()
        .query_opt(
            "SELECT 1 FROM people_skill_scales WHERE tenant_id = $1 AND company_id = $2 AND code = $3 AND status = 'draft'",
            &[&w.tenant, &w.company_id, &code],
        )
        .await
        .map_err(Error::from)?
        .is_some())
}

/// The version after the code's highest.
pub(crate) async fn next_version(w: &Write<'_>, code: &str) -> Result<i32> {
    let row = w
        .sql()
        .query_one(
            "SELECT COALESCE(MAX(version), 0) + 1 FROM people_skill_scales WHERE tenant_id = $1 AND company_id = $2 AND code = $3",
            &[&w.tenant, &w.company_id, &code],
        )
        .await
        .map_err(Error::from)?;
    Ok(row.get(0))
}

/// The code's published version(s), locked for the write.
pub(crate) async fn lock_published(w: &Write<'_>, code: &str) -> Result<Vec<Scale>> {
    w.sql()
        .query(
            &format!("SELECT {} FROM people_skill_scales WHERE tenant_id = $1 AND company_id = $2 AND code = $3 AND status = 'published' ORDER BY id FOR UPDATE", columns()),
            &[&w.tenant, &w.company_id, &code],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(scale)
        .collect::<DbResult<Vec<_>>>()
        .map_err(SkillsError::from)
}

/// Bilimbi `require_published_scale/3`: a published scale of the company,
/// else `ScaleUnavailable`.
pub(crate) async fn require_published(w: &Write<'_>, scale_id: i64) -> Result<Scale> {
    match get(w, scale_id).await? {
        Some(scale) if scale.status == "published" => Ok(scale),
        _ => Err(SkillsError::ScaleUnavailable),
    }
}

async fn level_rows(client: &impl GenericClient, scale_id: i64) -> DbResult<Vec<LevelRow>> {
    Ok(client
        .query(
            &format!("SELECT {LEVEL_COLUMNS} FROM people_skill_scale_levels WHERE scale_id = $1 ORDER BY level"),
            &[&scale_id],
        )
        .await?
        .iter()
        .map(level_row)
        .collect())
}

/// The scale's levels, lowest first.
pub(crate) async fn levels_of(client: &impl GenericClient, scale_id: i64) -> Result<Vec<Level>> {
    Ok(level_rows(client, scale_id)
        .await?
        .into_iter()
        .map(|row| row.level)
        .collect())
}

/// The scale's level numbers, lowest first.
pub(crate) async fn level_numbers(client: &impl GenericClient, scale_id: i64) -> Result<Vec<i32>> {
    Ok(level_rows(client, scale_id)
        .await?
        .into_iter()
        .map(|row| row.level.level)
        .collect())
}

/// Bilimbi `current_scale/4`: the scale a new profile draft uses. When the
/// source scale's code has a published version holding every required
/// level, that version; otherwise the source scale, with the required
/// levels the published version lacks (unique, ascending).
pub(crate) async fn current_scale(
    w: &Write<'_>,
    scale_id: i64,
    items: &[Item],
) -> Result<(i64, Vec<i32>)> {
    let Some(source) = get(w, scale_id).await? else {
        return Ok((scale_id, Vec::new()));
    };
    let published = w
        .sql()
        .query(
            &format!("SELECT {} FROM people_skill_scales WHERE tenant_id = $1 AND company_id = $2 AND code = $3 AND status = 'published'", columns()),
            &[&w.tenant, &w.company_id, &source.code],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(scale)
        .collect::<DbResult<Vec<_>>>()?;
    let [current] = published.as_slice() else {
        return Ok((scale_id, Vec::new()));
    };
    let levels = level_numbers(w.sql(), current.id).await?;
    let mut missing: Vec<i32> = items
        .iter()
        .map(|item| item.required_level)
        .filter(|level| !levels.contains(level))
        .collect();
    missing.sort_unstable();
    missing.dedup();
    if missing.is_empty() {
        Ok((current.id, missing))
    } else {
        Ok((scale_id, missing))
    }
}

// ---- Writes ----------------------------------------------------------------

/// A new draft version of the code, without levels.
pub(crate) async fn insert(
    w: &mut Write<'_>,
    code: &str,
    name: &str,
    version: i32,
) -> Result<Scale> {
    let (tenant, company_id) = (w.tenant, w.company_id);
    let (code, name) = (code.to_owned(), name.to_owned());
    Ok(w.tx
        .mutate(move |sql| {
            Box::pin(async move {
                let row = sql
                    .query_one(
                        &format!("INSERT INTO people_skill_scales (tenant_id, company_id, code, name, version, status) VALUES ($1, $2, $3, $4, $5, 'draft') RETURNING {}", columns()),
                        &[&tenant, &company_id, &code, &name, &version],
                    )
                    .await?;
                let created = scale(&row)?;
                let change = rows::created(SCALE, created.id, &audit(&created));
                Ok((created, vec![change]))
            })
        })
        .await?)
}

async fn insert_level(
    sql: &Transaction<'_>,
    tenant: i64,
    company_id: i64,
    scale_id: i64,
    level: &Level,
) -> DbResult<(LevelRow, Change)> {
    let row = sql
        .query_one(
            &format!("INSERT INTO people_skill_scale_levels (tenant_id, company_id, scale_id, level, name, anchor, authority) VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING {LEVEL_COLUMNS}"),
            &[&tenant, &company_id, &scale_id, &level.level, &level.name, &level.anchor, &level.authority],
        )
        .await?;
    let created = level_row(&row);
    let change = rows::created(LEVEL, created.id, &level_audit(scale_id, &created.level));
    Ok((created, change))
}

/// Copies the source version's levels onto the new draft.
pub(crate) async fn copy_levels(
    w: &mut Write<'_>,
    mut draft: Scale,
    levels: &[Level],
) -> Result<Scale> {
    if levels.is_empty() {
        return Ok(draft);
    }
    let (tenant, company_id, scale_id) = (w.tenant, w.company_id, draft.id);
    let levels = levels.to_vec();
    draft.levels =
        w.tx.mutate(move |sql| {
            Box::pin(async move {
                let mut copied = Vec::with_capacity(levels.len());
                let mut changes = Vec::with_capacity(levels.len());
                for level in &levels {
                    let (row, change) =
                        insert_level(sql, tenant, company_id, scale_id, level).await?;
                    copied.push(row.level);
                    changes.push(change);
                }
                Ok((copied, changes))
            })
        })
        .await?;
    Ok(draft)
}

pub(crate) async fn rename(w: &mut Write<'_>, draft: Scale, name: String) -> Result<Scale> {
    Ok(w.tx
        .mutate(move |sql| {
            Box::pin(async move {
                let row = sql
                    .query_one(
                        &format!("UPDATE people_skill_scales SET name = $2, updated_at = (now() AT TIME ZONE 'UTC') WHERE id = $1 RETURNING {}", columns()),
                        &[&draft.id, &name],
                    )
                    .await?;
                let updated = scale(&row)?;
                let change = rows::changed(SCALE, draft.id, &audit(&draft), &audit(&updated));
                Ok((updated, vec![change]))
            })
        })
        .await?)
}

/// Adds or replaces the draft's level with this number.
pub(crate) async fn put_level(w: &mut Write<'_>, draft: &Scale, level: Level) -> Result<Level> {
    let existing = level_rows(w.sql(), draft.id)
        .await?
        .into_iter()
        .find(|row| row.level.level == level.level);
    let (tenant, company_id, scale_id) = (w.tenant, w.company_id, draft.id);
    Ok(w.tx
        .mutate(move |sql| {
            Box::pin(async move {
                match existing {
                    Some(current) => {
                        let row = sql
                            .query_one(
                                &format!("UPDATE people_skill_scale_levels SET name = $2, anchor = $3, authority = $4, updated_at = (now() AT TIME ZONE 'UTC') WHERE id = $1 RETURNING {LEVEL_COLUMNS}"),
                                &[&current.id, &level.name, &level.anchor, &level.authority],
                            )
                            .await?;
                        let updated = level_row(&row);
                        let change = rows::changed(
                            LEVEL,
                            current.id,
                            &level_audit(scale_id, &current.level),
                            &level_audit(scale_id, &updated.level),
                        );
                        Ok((updated.level, vec![change]))
                    }
                    None => {
                        let (created, change) =
                            insert_level(sql, tenant, company_id, scale_id, &level).await?;
                        Ok((created.level, vec![change]))
                    }
                }
            })
        })
        .await?)
}

/// Removes the draft's level with this number; `NotFound` when it has none.
pub(crate) async fn delete_level(w: &mut Write<'_>, draft: &Scale, level: i32) -> Result<()> {
    let existing = level_rows(w.sql(), draft.id)
        .await?
        .into_iter()
        .find(|row| row.level.level == level)
        .ok_or(SkillsError::NotFound)?;
    let scale_id = draft.id;
    w.tx.mutate(move |sql| {
        Box::pin(async move {
            sql.execute(
                "DELETE FROM people_skill_scale_levels WHERE id = $1",
                &[&existing.id],
            )
            .await?;
            let change = rows::deleted(LEVEL, existing.id, &level_audit(scale_id, &existing.level));
            Ok(((), vec![change]))
        })
    })
    .await?;
    Ok(())
}

/// Publishes the draft and retires the code's previously published version
/// in the same write, so one version is current.
pub(crate) async fn publish(
    w: &mut Write<'_>,
    draft: Scale,
    levels: Vec<Level>,
    previous: Vec<Scale>,
) -> Result<Scale> {
    let actor = w.actor;
    Ok(w.tx
        .mutate(move |sql| {
            Box::pin(async move {
                let mut changes = Vec::with_capacity(previous.len() + 1);
                for current in previous {
                    let row = sql
                        .query_one(
                            &format!("UPDATE people_skill_scales SET status = 'retired', retired_at = (now() AT TIME ZONE 'UTC'), updated_at = (now() AT TIME ZONE 'UTC') WHERE id = $1 RETURNING {}", columns()),
                            &[&current.id],
                        )
                        .await?;
                    let retired = scale(&row)?;
                    changes.push(rows::changed(SCALE, current.id, &audit(&current), &audit(&retired)));
                }
                let row = sql
                    .query_one(
                        &format!("UPDATE people_skill_scales SET status = 'published', published_at = (now() AT TIME ZONE 'UTC'), actor_user_id = $2, updated_at = (now() AT TIME ZONE 'UTC') WHERE id = $1 RETURNING {}", columns()),
                        &[&draft.id, &actor],
                    )
                    .await?;
                let mut published = scale(&row)?;
                changes.push(rows::changed(SCALE, draft.id, &audit(&draft), &audit(&published)));
                published.levels = levels;
                Ok((published, changes))
            })
        })
        .await?)
}

pub(crate) async fn retire(w: &mut Write<'_>, current: Scale) -> Result<Scale> {
    Ok(w.tx
        .mutate(move |sql| {
            Box::pin(async move {
                let row = sql
                    .query_one(
                        &format!("UPDATE people_skill_scales SET status = 'retired', retired_at = (now() AT TIME ZONE 'UTC'), updated_at = (now() AT TIME ZONE 'UTC') WHERE id = $1 RETURNING {}", columns()),
                        &[&current.id],
                    )
                    .await?;
                let retired = scale(&row)?;
                let change = rows::changed(SCALE, current.id, &audit(&current), &audit(&retired));
                Ok((retired, vec![change]))
            })
        })
        .await?)
}

/// Deletes a draft with its levels.
pub(crate) async fn discard(w: &mut Write<'_>, draft: Scale, levels: Vec<Level>) -> Result<()> {
    let level_rows = level_rows(w.sql(), draft.id).await?;
    debug_assert_eq!(level_rows.len(), levels.len());
    w.tx.mutate(move |sql| {
        Box::pin(async move {
            let mut changes = Vec::with_capacity(level_rows.len() + 1);
            for row in level_rows {
                sql.execute(
                    "DELETE FROM people_skill_scale_levels WHERE id = $1",
                    &[&row.id],
                )
                .await?;
                changes.push(rows::deleted(
                    LEVEL,
                    row.id,
                    &level_audit(draft.id, &row.level),
                ));
            }
            sql.execute(
                "DELETE FROM people_skill_scales WHERE id = $1",
                &[&draft.id],
            )
            .await?;
            changes.push(rows::deleted(SCALE, draft.id, &audit(&draft)));
            Ok(((), changes))
        })
    })
    .await?;
    Ok(())
}
