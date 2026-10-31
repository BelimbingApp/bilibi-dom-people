//! The `people_skill_profiles`, `people_skill_profile_items` and
//! `people_skill_profile_selectors` tables: their rows, the SQL that reads
//! them, the writes behind the facade's profile functions and the publish
//! rules. Private to the facade. The database guards (migration 001) refuse
//! what these writes never attempt: an edit to a published or retired
//! version or to its items and selectors.
use crate::{
    catalog, rows, scales, Item, ItemInput, PositionOption, Profile, ProfileDetail, Result,
    Selector, SkillsError, Write,
};
use bilibi_database::{Change, Error, Result as DbResult};
use jiff::civil::Date;
use rust_decimal::Decimal;
use serde_json::{json, Value};
use tokio_postgres::{GenericClient, Row, Transaction};

const PROFILE: &str = "people_skill_profile";
const ITEM: &str = "people_skill_profile_item";
const SELECTOR: &str = "people_skill_profile_selector";
const ITEM_COLUMNS: &str = "id, profile_id, skill_id, sequence, required_level, criticality, weight_percent, mandatory, evidence_standard";
const SELECTOR_COLUMNS: &str = "id, profile_id, selector_type, position_id";

fn columns() -> String {
    format!(
        "id, code, name, version, status, scale_id, {}, {}, {}, {}",
        rows::date_text("effective_from"),
        rows::date_text("effective_to"),
        rows::datetime_text("published_at"),
        rows::datetime_text("retired_at")
    )
}

fn profile(row: &Row) -> DbResult<Profile> {
    Ok(Profile {
        id: row.get("id"),
        code: row.get("code"),
        name: row.get("name"),
        version: row.get("version"),
        status: row.get("status"),
        scale_id: row.get("scale_id"),
        effective_from: rows::date(row, "effective_from")?,
        effective_to: rows::date(row, "effective_to")?,
        published_at: rows::datetime(row, "published_at")?,
        retired_at: rows::datetime(row, "retired_at")?,
    })
}

fn item(row: &Row) -> Item {
    Item {
        id: row.get("id"),
        skill_id: row.get("skill_id"),
        sequence: row.get("sequence"),
        required_level: row.get("required_level"),
        criticality: row.get("criticality"),
        weight_percent: row.get("weight_percent"),
        mandatory: row.get("mandatory"),
        evidence_standard: row.get("evidence_standard"),
    }
}

fn selector(row: &Row) -> Selector {
    Selector {
        id: row.get("id"),
        selector_type: row.get("selector_type"),
        position_id: row.get("position_id"),
    }
}

fn item_audit(profile_id: i64, item: &Item) -> Value {
    let mut value = serde_json::to_value(item).unwrap_or_default();
    if let Some(object) = value.as_object_mut() {
        object.insert("profile_id".into(), json!(profile_id));
        object.remove("id");
    }
    value
}

fn selector_audit(profile_id: i64, selector: &Selector) -> Value {
    json!({
        "profile_id": profile_id,
        "selector_type": selector.selector_type,
        "position_id": selector.position_id,
    })
}

// ---- Reads -----------------------------------------------------------------

/// Profile versions: by code, newest version first.
pub(crate) async fn list(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
) -> Result<Vec<Profile>> {
    client
        .query(
            &format!("SELECT {} FROM people_skill_profiles WHERE tenant_id = $1 AND company_id = $2 ORDER BY code, version DESC", columns()),
            &[&tenant, &company_id],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(profile)
        .collect::<DbResult<Vec<_>>>()
        .map_err(SkillsError::from)
}

/// One profile version of the company, or `None`.
pub(crate) async fn get(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    id: i64,
) -> Result<Option<Profile>> {
    client
        .query_opt(
            &format!("SELECT {} FROM people_skill_profiles WHERE tenant_id = $1 AND company_id = $2 AND id = $3", columns()),
            &[&tenant, &company_id, &id],
        )
        .await
        .map_err(Error::from)?
        .as_ref()
        .map(profile)
        .transpose()
        .map_err(SkillsError::from)
}

/// One profile version of the company, locked for the write, or `None`.
pub(crate) async fn lock(w: &Write<'_>, id: i64) -> Result<Option<Profile>> {
    w.sql()
        .query_opt(
            &format!("SELECT {} FROM people_skill_profiles WHERE tenant_id = $1 AND company_id = $2 AND id = $3 FOR UPDATE", columns()),
            &[&w.tenant, &w.company_id, &id],
        )
        .await
        .map_err(Error::from)?
        .as_ref()
        .map(profile)
        .transpose()
        .map_err(SkillsError::from)
}

/// Bilimbi `lock_draft/4`: the locked profile, which must be a draft.
pub(crate) async fn lock_draft(w: &Write<'_>, id: i64) -> Result<Profile> {
    match lock(w, id).await? {
        None => Err(SkillsError::NotFound),
        Some(profile) if profile.status == "draft" => Ok(profile),
        Some(_) => Err(SkillsError::NotDraft),
    }
}

pub(crate) async fn code_exists(w: &Write<'_>, code: &str) -> Result<bool> {
    Ok(w.sql()
        .query_opt(
            "SELECT 1 FROM people_skill_profiles WHERE tenant_id = $1 AND company_id = $2 AND code = $3 LIMIT 1",
            &[&w.tenant, &w.company_id, &code],
        )
        .await
        .map_err(Error::from)?
        .is_some())
}

pub(crate) async fn open_draft(w: &Write<'_>, code: &str) -> Result<bool> {
    Ok(w.sql()
        .query_opt(
            "SELECT 1 FROM people_skill_profiles WHERE tenant_id = $1 AND company_id = $2 AND code = $3 AND status = 'draft'",
            &[&w.tenant, &w.company_id, &code],
        )
        .await
        .map_err(Error::from)?
        .is_some())
}

pub(crate) async fn next_version(w: &Write<'_>, code: &str) -> Result<i32> {
    let row = w
        .sql()
        .query_one(
            "SELECT COALESCE(MAX(version), 0) + 1 FROM people_skill_profiles WHERE tenant_id = $1 AND company_id = $2 AND code = $3",
            &[&w.tenant, &w.company_id, &code],
        )
        .await
        .map_err(Error::from)?;
    Ok(row.get(0))
}

/// The code's published version(s), locked for the write.
pub(crate) async fn lock_published(w: &Write<'_>, code: &str) -> Result<Vec<Profile>> {
    w.sql()
        .query(
            &format!("SELECT {} FROM people_skill_profiles WHERE tenant_id = $1 AND company_id = $2 AND code = $3 AND status = 'published' ORDER BY id FOR UPDATE", columns()),
            &[&w.tenant, &w.company_id, &code],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(profile)
        .collect::<DbResult<Vec<_>>>()
        .map_err(SkillsError::from)
}

/// Whether any profile version uses the scale.
pub(crate) async fn uses_scale(w: &Write<'_>, scale_id: i64) -> Result<bool> {
    Ok(w.sql()
        .query_opt(
            "SELECT 1 FROM people_skill_profiles WHERE tenant_id = $1 AND scale_id = $2 LIMIT 1",
            &[&w.tenant, &scale_id],
        )
        .await
        .map_err(Error::from)?
        .is_some())
}

/// The version's requirements in sequence order.
pub(crate) async fn items(
    client: &(impl GenericClient + Sync),
    profile_id: i64,
) -> Result<Vec<Item>> {
    Ok(client
        .query(
            &format!("SELECT {ITEM_COLUMNS} FROM people_skill_profile_items WHERE profile_id = $1 ORDER BY sequence"),
            &[&profile_id],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(item)
        .collect())
}

/// The version's targets by id.
pub(crate) async fn selectors(
    client: &(impl GenericClient + Sync),
    profile_id: i64,
) -> Result<Vec<Selector>> {
    Ok(client
        .query(
            &format!("SELECT {SELECTOR_COLUMNS} FROM people_skill_profile_selectors WHERE profile_id = $1 ORDER BY id"),
            &[&profile_id],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(selector)
        .collect())
}

/// Bilimbi `full_profile_view/2`.
pub(crate) async fn detail(
    client: &(impl GenericClient + Sync),
    profile: Profile,
) -> Result<ProfileDetail> {
    let items = items(client, profile.id).await?;
    let selectors = selectors(client, profile.id).await?;
    Ok(ProfileDetail {
        profile,
        items,
        selectors,
    })
}

/// Bilimbi `requirements/4`'s query: the published or retired versions in
/// force on the day whose targets cover the company, or the position.
pub(crate) async fn in_force(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    position_id: Option<i64>,
    as_of: Date,
) -> Result<Vec<Profile>> {
    let as_of = as_of.to_string();
    client
        .query(
            &format!(
                "SELECT {} FROM people_skill_profiles \
                 WHERE tenant_id = $1 AND company_id = $2 AND status IN ('published', 'retired') \
                   AND effective_from <= $3::text::date \
                   AND (effective_to IS NULL OR effective_to >= $3::text::date) \
                   AND id IN (SELECT profile_id FROM people_skill_profile_selectors \
                              WHERE selector_type = 'company' \
                                 OR (selector_type = 'position' AND position_id = $4)) \
                 ORDER BY id",
                columns()
            ),
            &[&tenant, &company_id, &as_of, &position_id],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(profile)
        .collect::<DbResult<Vec<_>>>()
        .map_err(SkillsError::from)
}

/// Bilimbi `after_code_history?/4`: a version starts after the published
/// version's start, which it closes, and after every retired version's last
/// day, so versions of one code never overlap.
pub(crate) async fn after_code_history(w: &Write<'_>, code: &str, from: Date) -> Result<bool> {
    let from = from.to_string();
    Ok(w.sql()
        .query_opt(
            "SELECT 1 FROM people_skill_profiles WHERE tenant_id = $1 AND company_id = $2 AND code = $3 \
             AND ((status = 'published' AND effective_from >= $4::text::date) \
               OR (status = 'retired' AND effective_to >= $4::text::date)) LIMIT 1",
            &[&w.tenant, &w.company_id, &code, &from],
        )
        .await
        .map_err(Error::from)?
        .is_none())
}

/// Bilimbi `overlapping?/5`: another code overlaps when one of its versions
/// is published or retired on or after `from` and either side targets the
/// company or both share a position.
pub(crate) async fn overlapping(
    w: &Write<'_>,
    draft: &Profile,
    selectors: &[Selector],
    from: Date,
) -> Result<bool> {
    let from = from.to_string();
    let others = w
        .sql()
        .query(
            "SELECT s.selector_type, s.position_id FROM people_skill_profiles p \
             JOIN people_skill_profile_selectors s ON s.profile_id = p.id \
             WHERE p.tenant_id = $1 AND p.company_id = $2 AND p.code <> $3 \
               AND p.status IN ('published', 'retired') \
               AND (p.effective_to IS NULL OR p.effective_to >= $4::text::date)",
            &[&w.tenant, &w.company_id, &draft.code, &from],
        )
        .await
        .map_err(Error::from)?;
    if others.is_empty() {
        return Ok(false);
    }
    let company_wide = selectors
        .iter()
        .any(|selector| selector.selector_type == "company");
    let mine: Vec<Option<i64>> = selectors
        .iter()
        .map(|selector| selector.position_id)
        .collect();
    Ok(company_wide
        || others.iter().any(|row| {
            let selector_type: String = row.get("selector_type");
            let position_id: Option<i64> = row.get("position_id");
            selector_type == "company" || mine.contains(&position_id)
        }))
}

/// Bilimbi `validate_publishable/5`, in its order: items, selectors, a
/// published scale, levels on it, active skills, weights totalling 100,
/// and positions still exposed (`known_positions` is what the Workforce
/// position read exposed; `None` while it is unavailable).
pub(crate) async fn validate_publishable(
    w: &Write<'_>,
    draft: &Profile,
    items: &[Item],
    selectors: &[Selector],
    known_positions: Option<&[PositionOption]>,
) -> Result<()> {
    if items.is_empty() {
        return Err(SkillsError::NoItems);
    }
    if selectors.is_empty() {
        return Err(SkillsError::NoSelectors);
    }
    match scales::get(w, draft.scale_id).await? {
        Some(scale) if scale.status == "published" => {}
        _ => return Err(SkillsError::ScaleUnavailable),
    }
    let levels = scales::level_numbers(w.sql(), draft.scale_id).await?;
    if items
        .iter()
        .any(|item| !levels.contains(&item.required_level))
    {
        return Err(SkillsError::LevelNotOnScale);
    }
    let skill_ids: Vec<i64> = items.iter().map(|item| item.skill_id).collect();
    let skills = catalog::skills_by_ids(w.sql(), w.tenant, w.company_id, &skill_ids).await?;
    if items.iter().any(|item| {
        !skills
            .iter()
            .any(|skill| skill.id == item.skill_id && skill.active)
    }) {
        return Err(SkillsError::SkillUnavailable);
    }
    let total: Decimal = items.iter().map(|item| item.weight_percent).sum();
    if total != Decimal::ONE_HUNDRED {
        return Err(SkillsError::WeightsNot100);
    }
    let wanted: Vec<i64> = selectors
        .iter()
        .filter_map(|selector| selector.position_id)
        .collect();
    if wanted.is_empty() {
        return Ok(());
    }
    match known_positions {
        Some(known)
            if wanted
                .iter()
                .all(|id| known.iter().any(|option| option.id == *id)) =>
        {
            Ok(())
        }
        _ => Err(SkillsError::PositionUnavailable),
    }
}

// ---- Writes ----------------------------------------------------------------

/// A new draft version of the code against the scale, without items or
/// selectors.
pub(crate) async fn insert(
    w: &mut Write<'_>,
    code: &str,
    name: &str,
    version: i32,
    scale_id: i64,
) -> Result<Profile> {
    let (tenant, company_id) = (w.tenant, w.company_id);
    let (code, name) = (code.to_owned(), name.to_owned());
    Ok(w.tx
        .mutate(move |sql| {
            Box::pin(async move {
                let row = sql
                    .query_one(
                        &format!("INSERT INTO people_skill_profiles (tenant_id, company_id, code, name, version, status, scale_id) VALUES ($1, $2, $3, $4, $5, 'draft', $6) RETURNING {}", columns()),
                        &[&tenant, &company_id, &code, &name, &version, &scale_id],
                    )
                    .await?;
                let created = profile(&row)?;
                let change = rows::created(PROFILE, created.id, &created);
                Ok((created, vec![change]))
            })
        })
        .await?)
}

/// Renames the draft and/or moves it to another scale.
pub(crate) async fn update(
    w: &mut Write<'_>,
    draft: Profile,
    name: Option<String>,
    scale_id: Option<i64>,
) -> Result<Profile> {
    let name = name.unwrap_or_else(|| draft.name.clone());
    let scale_id = scale_id.unwrap_or(draft.scale_id);
    Ok(w.tx
        .mutate(move |sql| {
            Box::pin(async move {
                let row = sql
                    .query_one(
                        &format!("UPDATE people_skill_profiles SET name = $2, scale_id = $3, updated_at = (now() AT TIME ZONE 'UTC') WHERE id = $1 RETURNING {}", columns()),
                        &[&draft.id, &name, &scale_id],
                    )
                    .await?;
                let updated = profile(&row)?;
                let change = rows::changed(PROFILE, draft.id, &draft, &updated);
                Ok((updated, vec![change]))
            })
        })
        .await?)
}

async fn insert_item(
    sql: &Transaction<'_>,
    tenant: i64,
    company_id: i64,
    profile_id: i64,
    sequence: i32,
    input: &ItemInput,
) -> DbResult<(Item, Change)> {
    let row = sql
        .query_one(
            &format!("INSERT INTO people_skill_profile_items (tenant_id, company_id, profile_id, skill_id, sequence, required_level, criticality, weight_percent, mandatory, evidence_standard) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING {ITEM_COLUMNS}"),
            &[
                &tenant,
                &company_id,
                &profile_id,
                &input.skill_id,
                &sequence,
                &input.required_level,
                &input.criticality,
                &input.weight_percent,
                &input.mandatory,
                &input.evidence_standard,
            ],
        )
        .await?;
    let created = item(&row);
    let change = rows::created(ITEM, created.id, &item_audit(profile_id, &created));
    Ok((created, change))
}

async fn insert_selector_row(
    sql: &Transaction<'_>,
    tenant: i64,
    company_id: i64,
    profile_id: i64,
    selector_type: &str,
    position_id: Option<i64>,
) -> DbResult<(Selector, Change)> {
    let row = sql
        .query_one(
            &format!("INSERT INTO people_skill_profile_selectors (tenant_id, company_id, profile_id, selector_type, position_id) VALUES ($1, $2, $3, $4, $5) RETURNING {SELECTOR_COLUMNS}"),
            &[&tenant, &company_id, &profile_id, &selector_type, &position_id],
        )
        .await?;
    let created = selector(&row);
    let change = rows::created(SELECTOR, created.id, &selector_audit(profile_id, &created));
    Ok((created, change))
}

/// Copies the source version's items and selectors onto the new draft.
pub(crate) async fn copy_children(
    w: &mut Write<'_>,
    draft: Profile,
    items: &[Item],
    selectors: &[Selector],
) -> Result<ProfileDetail> {
    if items.is_empty() && selectors.is_empty() {
        return Ok(ProfileDetail {
            profile: draft,
            items: Vec::new(),
            selectors: Vec::new(),
        });
    }
    let (tenant, company_id, profile_id) = (w.tenant, w.company_id, draft.id);
    let (items, selectors) = (items.to_vec(), selectors.to_vec());
    let (items, selectors) =
        w.tx.mutate(move |sql| {
            Box::pin(async move {
                let mut changes = Vec::with_capacity(items.len() + selectors.len());
                let mut copied_items = Vec::with_capacity(items.len());
                for source in &items {
                    let input = ItemInput {
                        skill_id: source.skill_id,
                        required_level: source.required_level,
                        criticality: source.criticality.clone(),
                        weight_percent: source.weight_percent,
                        mandatory: source.mandatory,
                        evidence_standard: source.evidence_standard.clone(),
                    };
                    let (copied, change) =
                        insert_item(sql, tenant, company_id, profile_id, source.sequence, &input)
                            .await?;
                    copied_items.push(copied);
                    changes.push(change);
                }
                let mut copied_selectors = Vec::with_capacity(selectors.len());
                for source in &selectors {
                    let (copied, change) = insert_selector_row(
                        sql,
                        tenant,
                        company_id,
                        profile_id,
                        &source.selector_type,
                        source.position_id,
                    )
                    .await?;
                    copied_selectors.push(copied);
                    changes.push(change);
                }
                Ok(((copied_items, copied_selectors), changes))
            })
        })
        .await?;
    Ok(ProfileDetail {
        profile: draft,
        items,
        selectors,
    })
}

/// Adds the requirement, or replaces the one for the same skill.
pub(crate) async fn put_item(
    w: &mut Write<'_>,
    draft: &Profile,
    items: &[Item],
    input: ItemInput,
) -> Result<Item> {
    let existing = items
        .iter()
        .find(|item| item.skill_id == input.skill_id)
        .cloned();
    let next_sequence = i32::try_from(items.len()).map_err(|_| Error::Invalid)? + 1;
    let (tenant, company_id, profile_id) = (w.tenant, w.company_id, draft.id);
    Ok(w.tx
        .mutate(move |sql| {
            Box::pin(async move {
                match existing {
                    Some(current) => {
                        let row = sql
                            .query_one(
                                &format!("UPDATE people_skill_profile_items SET required_level = $2, criticality = $3, weight_percent = $4, mandatory = $5, evidence_standard = $6, updated_at = (now() AT TIME ZONE 'UTC') WHERE id = $1 RETURNING {ITEM_COLUMNS}"),
                                &[
                                    &current.id,
                                    &input.required_level,
                                    &input.criticality,
                                    &input.weight_percent,
                                    &input.mandatory,
                                    &input.evidence_standard,
                                ],
                            )
                            .await?;
                        let updated = item(&row);
                        let change = rows::changed(
                            ITEM,
                            current.id,
                            &item_audit(profile_id, &current),
                            &item_audit(profile_id, &updated),
                        );
                        Ok((updated, vec![change]))
                    }
                    None => {
                        let (created, change) =
                            insert_item(sql, tenant, company_id, profile_id, next_sequence, &input).await?;
                        Ok((created, vec![change]))
                    }
                }
            })
        })
        .await?)
}

/// Bilimbi `resequence/2`: two passes keep the unique sequence index
/// satisfied while items swap places. Reports a change for every item whose
/// sequence moved.
async fn resequence_sql(
    sql: &Transaction<'_>,
    profile_id: i64,
    items: &[Item],
) -> DbResult<Vec<Change>> {
    for (index, item) in items.iter().enumerate() {
        let sequence = -(i32::try_from(index).map_err(|_| Error::Invalid)? + 1);
        sql.execute(
            "UPDATE people_skill_profile_items SET sequence = $2 WHERE id = $1",
            &[&item.id, &sequence],
        )
        .await?;
    }
    let mut changes = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let sequence = i32::try_from(index).map_err(|_| Error::Invalid)? + 1;
        sql.execute(
            "UPDATE people_skill_profile_items SET sequence = $2, updated_at = (now() AT TIME ZONE 'UTC') WHERE id = $1",
            &[&item.id, &sequence],
        )
        .await?;
        if item.sequence != sequence {
            let moved = Item {
                sequence,
                ..item.clone()
            };
            changes.push(rows::changed(
                ITEM,
                item.id,
                &item_audit(profile_id, item),
                &item_audit(profile_id, &moved),
            ));
        }
    }
    Ok(changes)
}

/// Removes the requirement and closes the gap in the sequence.
pub(crate) async fn remove_item(
    w: &mut Write<'_>,
    draft: &Profile,
    items: Vec<Item>,
    item_id: i64,
) -> Result<()> {
    let profile_id = draft.id;
    w.tx.mutate(move |sql| {
        Box::pin(async move {
            let (removed, kept): (Vec<Item>, Vec<Item>) =
                items.into_iter().partition(|item| item.id == item_id);
            let Some(removed) = removed.into_iter().next() else {
                return Err(Error::NotFound);
            };
            sql.execute(
                "DELETE FROM people_skill_profile_items WHERE id = $1",
                &[&removed.id],
            )
            .await?;
            let mut changes = vec![rows::deleted(
                ITEM,
                removed.id,
                &item_audit(profile_id, &removed),
            )];
            changes.extend(resequence_sql(sql, profile_id, &kept).await?);
            Ok(((), changes))
        })
    })
    .await?;
    Ok(())
}

/// Stores the items in this order.
pub(crate) async fn resequence(w: &mut Write<'_>, draft: &Profile, items: Vec<Item>) -> Result<()> {
    let profile_id = draft.id;
    w.tx.mutate(move |sql| {
        Box::pin(async move {
            let changes = resequence_sql(sql, profile_id, &items).await?;
            Ok(((), changes))
        })
    })
    .await?;
    Ok(())
}

pub(crate) async fn insert_selector(
    w: &mut Write<'_>,
    draft: &Profile,
    selector_type: &str,
    position_id: Option<i64>,
) -> Result<Selector> {
    let (tenant, company_id, profile_id) = (w.tenant, w.company_id, draft.id);
    let selector_type = selector_type.to_owned();
    Ok(w.tx
        .mutate(move |sql| {
            Box::pin(async move {
                let (created, change) = insert_selector_row(
                    sql,
                    tenant,
                    company_id,
                    profile_id,
                    &selector_type,
                    position_id,
                )
                .await?;
                Ok((created, vec![change]))
            })
        })
        .await?)
}

pub(crate) async fn remove_selector(
    w: &mut Write<'_>,
    draft: &Profile,
    selector: Selector,
) -> Result<()> {
    let profile_id = draft.id;
    w.tx.mutate(move |sql| {
        Box::pin(async move {
            sql.execute(
                "DELETE FROM people_skill_profile_selectors WHERE id = $1",
                &[&selector.id],
            )
            .await?;
            let change = rows::deleted(
                SELECTOR,
                selector.id,
                &selector_audit(profile_id, &selector),
            );
            Ok(((), vec![change]))
        })
    })
    .await?;
    Ok(())
}

/// Deletes a draft with its items and selectors.
pub(crate) async fn discard(
    w: &mut Write<'_>,
    draft: Profile,
    items: Vec<Item>,
    selectors: Vec<Selector>,
) -> Result<()> {
    w.tx.mutate(move |sql| {
        Box::pin(async move {
            let mut changes = Vec::with_capacity(items.len() + selectors.len() + 1);
            for item in items {
                sql.execute(
                    "DELETE FROM people_skill_profile_items WHERE id = $1",
                    &[&item.id],
                )
                .await?;
                changes.push(rows::deleted(ITEM, item.id, &item_audit(draft.id, &item)));
            }
            for selector in selectors {
                sql.execute(
                    "DELETE FROM people_skill_profile_selectors WHERE id = $1",
                    &[&selector.id],
                )
                .await?;
                changes.push(rows::deleted(
                    SELECTOR,
                    selector.id,
                    &selector_audit(draft.id, &selector),
                ));
            }
            sql.execute(
                "DELETE FROM people_skill_profiles WHERE id = $1",
                &[&draft.id],
            )
            .await?;
            changes.push(rows::deleted(PROFILE, draft.id, &draft));
            Ok(((), changes))
        })
    })
    .await?;
    Ok(())
}

/// Publishes the draft from `from` and retires the code's previously
/// published version the day before, in the same write.
pub(crate) async fn publish(
    w: &mut Write<'_>,
    draft: Profile,
    previous: Vec<Profile>,
    from: Date,
) -> Result<Profile> {
    let actor = w.actor;
    let from_text = from.to_string();
    let day_before = from
        .yesterday()
        .map_err(|_| SkillsError::InvalidDate)?
        .to_string();
    Ok(w.tx
        .mutate(move |sql| {
            Box::pin(async move {
                let mut changes = Vec::with_capacity(previous.len() + 1);
                for current in previous {
                    let row = sql
                        .query_one(
                            &format!("UPDATE people_skill_profiles SET status = 'retired', retired_at = (now() AT TIME ZONE 'UTC'), effective_to = $2::text::date, updated_at = (now() AT TIME ZONE 'UTC') WHERE id = $1 RETURNING {}", columns()),
                            &[&current.id, &day_before],
                        )
                        .await?;
                    let retired = profile(&row)?;
                    changes.push(rows::changed(PROFILE, current.id, &current, &retired));
                }
                let row = sql
                    .query_one(
                        &format!("UPDATE people_skill_profiles SET status = 'published', effective_from = $2::text::date, published_at = (now() AT TIME ZONE 'UTC'), actor_user_id = $3, updated_at = (now() AT TIME ZONE 'UTC') WHERE id = $1 RETURNING {}", columns()),
                        &[&draft.id, &from_text, &actor],
                    )
                    .await?;
                let published = profile(&row)?;
                changes.push(rows::changed(PROFILE, draft.id, &draft, &published));
                Ok((published, changes))
            })
        })
        .await?)
}

/// Retires the published version after its last effective day.
pub(crate) async fn retire(w: &mut Write<'_>, current: Profile, to: Date) -> Result<Profile> {
    let actor = w.actor;
    let to = to.to_string();
    Ok(w.tx
        .mutate(move |sql| {
            Box::pin(async move {
                let row = sql
                    .query_one(
                        &format!("UPDATE people_skill_profiles SET status = 'retired', retired_at = (now() AT TIME ZONE 'UTC'), effective_to = $2::text::date, actor_user_id = $3, updated_at = (now() AT TIME ZONE 'UTC') WHERE id = $1 RETURNING {}", columns()),
                        &[&current.id, &to, &actor],
                    )
                    .await?;
                let retired = profile(&row)?;
                let change = rows::changed(PROFILE, current.id, &current, &retired);
                Ok((retired, vec![change]))
            })
        })
        .await?)
}
