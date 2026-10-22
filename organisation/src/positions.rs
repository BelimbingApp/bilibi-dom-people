//! The three tables (`people_positions`, `people_position_revisions`,
//! `people_position_placements`): their rows, the SQL that reads them, the
//! validation in Bilimbi's changeset words and the writes behind the facade.
//! Private to the facade.
use crate::{
    Assignment, FieldError, NewAssignment, NewPosition, NewVersion, Position, PositionVersion,
    Result, ASSIGNMENT_KINDS, ASSIGNMENT_LIMIT,
};
use bilibi_audit::{record_action, Attribution};
use bilibi_database::{Change, Error, MutationTransaction};
use bilibi_tenancy::Scope;
use jiff::civil::Date;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use tokio_postgres::{GenericClient, Row};

/// Dates are bound and read as ISO text: the workspace's `tokio-postgres`
/// has no date feature, and a Domain must not turn one on.
const POSITION: &str = "id, company_id, code, parent_id";
const VERSION: &str = "id, position_id, version, title, \
    to_char(effective_from, 'YYYY-MM-DD') AS effective_from, to_char(effective_to, 'YYYY-MM-DD') AS effective_to";
const PLACEMENT: &str = "id, position_id, employee_id, kind, \
    to_char(effective_from, 'YYYY-MM-DD') AS effective_from, to_char(effective_to, 'YYYY-MM-DD') AS effective_to";

fn date(row: &Row, column: &str) -> Result<Date> {
    row.get::<_, String>(column)
        .parse::<Date>()
        .map_err(|_| Error::Internal.into())
}
fn optional_date(row: &Row, column: &str) -> Result<Option<Date>> {
    row.get::<_, Option<String>>(column)
        .map(|text| text.parse::<Date>().map_err(|_| Error::Internal.into()))
        .transpose()
}
fn text(date: Option<Date>) -> Option<String> {
    date.map(|date| date.to_string())
}

fn position(row: &Row) -> Position {
    Position {
        id: row.get("id"),
        platform_company_id: row.get("company_id"),
        code: row.get("code"),
        parent_id: row.get("parent_id"),
    }
}
fn version(row: &Row) -> Result<PositionVersion> {
    Ok(PositionVersion {
        id: row.get("id"),
        position_id: row.get("position_id"),
        version: row.get("version"),
        title: row.get("title"),
        effective_from: date(row, "effective_from")?,
        effective_to: optional_date(row, "effective_to")?,
    })
}
fn placement(row: &Row) -> Result<Assignment> {
    Ok(Assignment {
        id: row.get("id"),
        position_id: row.get("position_id"),
        employee_id: row.get("employee_id"),
        kind: row.get("kind"),
        effective_from: date(row, "effective_from")?,
        effective_to: optional_date(row, "effective_to")?,
    })
}

impl Position {
    fn audit_values(&self) -> Value {
        json!({ "company_id": self.platform_company_id, "code": self.code, "parent_id": self.parent_id })
    }
}
impl PositionVersion {
    fn audit_values(&self) -> Value {
        json!({
            "position_id": self.position_id, "version": self.version, "title": self.title,
            "effective_from": self.effective_from.to_string(), "effective_to": text(self.effective_to),
        })
    }
}
impl Assignment {
    fn audit_values(&self) -> Value {
        json!({
            "position_id": self.position_id, "employee_id": self.employee_id, "kind": self.kind,
            "effective_from": self.effective_from.to_string(), "effective_to": text(self.effective_to),
        })
    }
}

// ---- positions ----------------------------------------------------------

/// A position's facts once validated: the code trimmed, 1 to 255 characters.
pub(crate) struct ValidPosition {
    pub code: String,
    pub parent_id: Option<i64>,
}

/// Bilimbi `Position.changeset/2`, in its words.
pub(crate) fn validate_position(
    new: &NewPosition,
) -> std::result::Result<ValidPosition, Vec<FieldError>> {
    let mut errors = Vec::new();
    let code = new.code.trim().to_owned();
    if code.is_empty() {
        errors.push(FieldError::new("code", "can't be blank"));
    } else if code.chars().count() > 255 {
        errors.push(FieldError::new(
            "code",
            "should be at most 255 character(s)",
        ));
    }
    if errors.is_empty() {
        Ok(ValidPosition {
            code,
            parent_id: new.parent_id,
        })
    } else {
        Err(errors)
    }
}

/// One position of the company, or `None` (a non-positive id included).
pub(crate) async fn find(
    client: &impl GenericClient,
    company_id: i64,
    id: i64,
) -> Result<Option<Position>> {
    if id <= 0 {
        return Ok(None);
    }
    Ok(client
        .query_opt(
            &format!("SELECT {POSITION} FROM people_positions WHERE company_id = $1 AND id = $2"),
            &[&company_id, &id],
        )
        .await
        .map_err(Error::from)?
        .as_ref()
        .map(position))
}

/// A code already used in the company is "has already been taken".
pub(crate) async fn check_code(
    client: &impl GenericClient,
    company_id: i64,
    code: &str,
) -> Result<()> {
    let taken = client
        .query_opt(
            "SELECT 1 FROM people_positions WHERE company_id = $1 AND code = $2",
            &[&company_id, &code],
        )
        .await
        .map_err(Error::from)?;
    if taken.is_some() {
        return Err(crate::OrganisationError::Invalid(vec![FieldError::new(
            "code",
            "has already been taken",
        )]));
    }
    Ok(())
}

/// Inserts the position, recorded as a created `people_position` change.
pub(crate) async fn insert_position(
    tx: &mut MutationTransaction<'_>,
    company_id: i64,
    valid: ValidPosition,
) -> Result<Position> {
    let position = tx
        .mutate(move |sql| {
            Box::pin(async move {
                let row = sql
                    .query_one(
                        &format!(
                            "INSERT INTO people_positions (company_id, code, parent_id) VALUES ($1, $2, $3) RETURNING {POSITION}"
                        ),
                        &[&company_id, &valid.code, &valid.parent_id],
                    )
                    .await?;
                let position = position(&row);
                // An empty object on the absent side is what names the audit
                // event `created`.
                let change = Change {
                    kind: "people_position".into(),
                    id: position.id,
                    old: json!({}),
                    new: position.audit_values(),
                };
                Ok((position, vec![change]))
            })
        })
        .await?;
    Ok(position)
}

/// Walks up from `parent`: reaching `id` is a `Cycle`, an ancestor that is
/// not a position of the company is `NotFound`, the root is fine.
pub(crate) async fn reject_cycle(
    client: &impl GenericClient,
    company_id: i64,
    id: i64,
    parent: Option<i64>,
) -> Result<()> {
    let mut seen = HashSet::new();
    let mut next = parent;
    while let Some(ancestor) = next {
        if ancestor == id || !seen.insert(ancestor) {
            return Err(crate::OrganisationError::Cycle);
        }
        let position = find(client, company_id, ancestor)
            .await?
            .ok_or(crate::OrganisationError::NotFound)?;
        next = position.parent_id;
    }
    Ok(())
}

/// Moves the position under `parent`, recorded as a `people_position` change.
pub(crate) async fn update_parent(
    tx: &mut MutationTransaction<'_>,
    position: Position,
    parent: Option<i64>,
) -> Result<Position> {
    let moved = tx
        .mutate(move |sql| {
            Box::pin(async move {
                let row = sql
                    .query_one(
                        &format!(
                            "UPDATE people_positions SET parent_id = $2, updated_at = now() AT TIME ZONE 'UTC' WHERE id = $1 RETURNING {POSITION}"
                        ),
                        &[&position.id, &parent],
                    )
                    .await?;
                let moved = self::position(&row);
                let change = Change {
                    kind: "people_position".into(),
                    id: moved.id,
                    old: position.audit_values(),
                    new: moved.audit_values(),
                };
                Ok((moved, vec![change]))
            })
        })
        .await?;
    Ok(moved)
}

/// The company's positions by id: one offset page.
pub(crate) async fn page(
    client: &impl GenericClient,
    company_id: i64,
    page: i64,
    size: i64,
) -> Result<Vec<Position>> {
    let offset = (page - 1).saturating_mul(size);
    Ok(client
        .query(
            &format!("SELECT {POSITION} FROM people_positions WHERE company_id = $1 ORDER BY id LIMIT $2 OFFSET $3"),
            &[&company_id, &size, &offset],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(position)
        .collect())
}

/// The company's positions with ids above `last_id` and at or below
/// `high_water_id`, by id, at most `limit`.
pub(crate) async fn keyset(
    client: &impl GenericClient,
    company_id: i64,
    last_id: i64,
    high_water_id: i64,
    limit: i64,
) -> Result<Vec<Position>> {
    Ok(client
        .query(
            &format!("SELECT {POSITION} FROM people_positions WHERE company_id = $1 AND id > $2 AND id <= $3 ORDER BY id LIMIT $4"),
            &[&company_id, &last_id, &high_water_id, &limit],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(position)
        .collect())
}

pub(crate) async fn count(client: &impl GenericClient, company_id: i64) -> Result<i64> {
    let row = client
        .query_one(
            "SELECT count(*) FROM people_positions WHERE company_id = $1",
            &[&company_id],
        )
        .await
        .map_err(Error::from)?;
    Ok(row.get(0))
}

// ---- versions -----------------------------------------------------------

pub(crate) struct ValidVersion {
    pub version: i32,
    pub title: String,
    pub effective_from: Date,
    pub effective_to: Option<Date>,
}

/// Bilimbi `PositionVersion.changeset/2`, in its words.
pub(crate) fn validate_version(
    new: &NewVersion,
) -> std::result::Result<ValidVersion, Vec<FieldError>> {
    let mut errors = Vec::new();
    if new.version <= 0 {
        errors.push(FieldError::new("version", "must be greater than 0"));
    }
    let title = new.title.trim().to_owned();
    if title.is_empty() {
        errors.push(FieldError::new("title", "can't be blank"));
    } else if title.chars().count() > 255 {
        errors.push(FieldError::new(
            "title",
            "should be at most 255 character(s)",
        ));
    }
    if new.effective_to.is_some_and(|to| to < new.effective_from) {
        errors.push(FieldError::new(
            "effective_to",
            "must be on or after the start",
        ));
    }
    if errors.is_empty() {
        Ok(ValidVersion {
            version: new.version,
            title,
            effective_from: new.effective_from,
            effective_to: new.effective_to,
        })
    } else {
        Err(errors)
    }
}

/// Decided before the insert, as Bilimbi's constraints report them: a
/// version number already used ("has already been taken") and an interval
/// overlapping an existing version ("violates an exclusion constraint").
pub(crate) async fn check_version(
    client: &impl GenericClient,
    position_id: i64,
    valid: &ValidVersion,
) -> Result<()> {
    let mut errors = Vec::new();
    let taken = client
        .query_opt(
            "SELECT 1 FROM people_position_revisions WHERE position_id = $1 AND version = $2",
            &[&position_id, &valid.version],
        )
        .await
        .map_err(Error::from)?;
    if taken.is_some() {
        errors.push(FieldError::new("version", "has already been taken"));
    }
    let overlap = client
        .query_opt(
            "SELECT 1 FROM people_position_revisions WHERE position_id = $1 \
             AND daterange(effective_from, effective_to, '[]') && daterange($2::text::date, $3::text::date, '[]')",
            &[
                &position_id,
                &valid.effective_from.to_string(),
                &text(valid.effective_to),
            ],
        )
        .await
        .map_err(Error::from)?;
    if overlap.is_some() {
        errors.push(FieldError::new(
            "effective_from",
            "violates an exclusion constraint",
        ));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(crate::OrganisationError::Invalid(errors))
    }
}

/// Inserts the version, recorded as a created `people_position_revision`.
pub(crate) async fn insert_version(
    tx: &mut MutationTransaction<'_>,
    position_id: i64,
    valid: ValidVersion,
) -> Result<PositionVersion> {
    let version = tx
        .mutate(move |sql| {
            Box::pin(async move {
                let row = sql
                    .query_one(
                        &format!(
                            "INSERT INTO people_position_revisions (position_id, version, title, effective_from, effective_to) \
                             VALUES ($1, $2, $3, $4::text::date, $5::text::date) RETURNING {VERSION}"
                        ),
                        &[
                            &position_id,
                            &valid.version,
                            &valid.title,
                            &valid.effective_from.to_string(),
                            &text(valid.effective_to),
                        ],
                    )
                    .await?;
                let version = version(&row).map_err(|_| Error::Internal)?;
                let change = Change {
                    kind: "people_position_revision".into(),
                    id: version.id,
                    old: json!({}),
                    new: version.audit_values(),
                };
                Ok((version, vec![change]))
            })
        })
        .await?;
    Ok(version)
}

/// The version of each position effective on `day`: `(title, version)`.
pub(crate) async fn effective_versions(
    client: &impl GenericClient,
    ids: &[i64],
    day: Date,
) -> Result<HashMap<i64, (String, i32)>> {
    Ok(client
        .query(
            "SELECT position_id, title, version FROM people_position_revisions \
             WHERE position_id = ANY($1::bigint[]) AND effective_from <= $2::text::date \
             AND (effective_to IS NULL OR effective_to >= $2::text::date)",
            &[&ids, &day.to_string()],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(|row| {
            (
                row.get::<_, i64>("position_id"),
                (row.get::<_, String>("title"), row.get::<_, i32>("version")),
            )
        })
        .collect())
}

// ---- assignments --------------------------------------------------------

pub(crate) struct ValidAssignment {
    pub employee_id: i64,
    pub kind: String,
    pub effective_from: Date,
    pub effective_to: Option<Date>,
}

/// Bilimbi `PositionAssignment.changeset/2`, in its words. A non-positive
/// employee id is what the Workforce read answers `NotFound` to.
pub(crate) fn validate_assignment(
    new: &NewAssignment,
) -> std::result::Result<ValidAssignment, Vec<FieldError>> {
    let mut errors = Vec::new();
    let kind = new.kind.trim().to_owned();
    if kind.is_empty() {
        errors.push(FieldError::new("kind", "can't be blank"));
    } else if !ASSIGNMENT_KINDS.contains(&kind.as_str()) {
        errors.push(FieldError::new("kind", "is invalid"));
    }
    if new.effective_to.is_some_and(|to| to < new.effective_from) {
        errors.push(FieldError::new(
            "effective_to",
            "must be on or after the start",
        ));
    }
    if errors.is_empty() {
        Ok(ValidAssignment {
            employee_id: new.employee_id,
            kind,
            effective_from: new.effective_from,
            effective_to: new.effective_to,
        })
    } else {
        Err(errors)
    }
}

/// Bilimbi `PositionAssignment.end_changeset/2`: the new end is on or after
/// the start and before the current end.
pub(crate) fn validate_end(
    assignment: &Assignment,
    effective_to: Date,
) -> std::result::Result<(), Vec<FieldError>> {
    let mut errors = Vec::new();
    if effective_to < assignment.effective_from {
        errors.push(FieldError::new(
            "effective_to",
            "must be on or after the start",
        ));
    }
    if assignment
        .effective_to
        .is_some_and(|current| effective_to >= current)
    {
        errors.push(FieldError::new(
            "effective_to",
            "must be before the current end",
        ));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// The substantive assignments of the position that began before `start`
/// and still run on it, locked for the transaction: the holders a new
/// substantive assignment may have to release.
pub(crate) async fn releasable(
    client: &impl GenericClient,
    position_id: i64,
    start: Date,
) -> Result<Vec<Assignment>> {
    client
        .query(
            &format!(
                "SELECT {PLACEMENT} FROM people_position_placements WHERE position_id = $1 AND kind = 'substantive' \
                 AND effective_from < $2::text::date AND (effective_to IS NULL OR effective_to >= $2::text::date) \
                 ORDER BY id FOR UPDATE"
            ),
            &[&position_id, &start.to_string()],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(placement)
        .collect()
}

/// A substantive assignment still overlapping the interval refuses the new
/// one, as the exclusion constraint would ("violates an exclusion
/// constraint" on `effective_from`); the constraint stays the guarantee for
/// a writer that bypasses the company lock.
pub(crate) async fn check_substantive(
    client: &impl GenericClient,
    position_id: i64,
    from: Date,
    to: Option<Date>,
) -> Result<()> {
    let overlap = client
        .query_opt(
            "SELECT 1 FROM people_position_placements WHERE position_id = $1 AND kind = 'substantive' \
             AND daterange(effective_from, effective_to, '[]') && daterange($2::text::date, $3::text::date, '[]')",
            &[&position_id, &from.to_string(), &text(to)],
        )
        .await
        .map_err(Error::from)?;
    if overlap.is_some() {
        return Err(crate::OrganisationError::Invalid(vec![FieldError::new(
            "effective_from",
            "violates an exclusion constraint",
        )]));
    }
    Ok(())
}

/// Inserts the assignment, recorded as a created `people_position_placement`.
pub(crate) async fn insert_assignment(
    tx: &mut MutationTransaction<'_>,
    position_id: i64,
    valid: ValidAssignment,
) -> Result<Assignment> {
    let assignment = tx
        .mutate(move |sql| {
            Box::pin(async move {
                let row = sql
                    .query_one(
                        &format!(
                            "INSERT INTO people_position_placements (position_id, employee_id, kind, effective_from, effective_to) \
                             VALUES ($1, $2, $3, $4::text::date, $5::text::date) RETURNING {PLACEMENT}"
                        ),
                        &[
                            &position_id,
                            &valid.employee_id,
                            &valid.kind,
                            &valid.effective_from.to_string(),
                            &text(valid.effective_to),
                        ],
                    )
                    .await?;
                let assignment = placement(&row).map_err(|_| Error::Internal)?;
                let change = Change {
                    kind: "people_position_placement".into(),
                    id: assignment.id,
                    old: json!({}),
                    new: assignment.audit_values(),
                };
                Ok((assignment, vec![change]))
            })
        })
        .await?;
    Ok(assignment)
}

/// One assignment of a position of the company, locked for the transaction,
/// or `None`.
pub(crate) async fn lock_assignment(
    client: &impl GenericClient,
    company_id: i64,
    id: i64,
) -> Result<Option<Assignment>> {
    if id <= 0 {
        return Ok(None);
    }
    client
        .query_opt(
            "SELECT a.id, a.position_id, a.employee_id, a.kind, \
             to_char(a.effective_from, 'YYYY-MM-DD') AS effective_from, to_char(a.effective_to, 'YYYY-MM-DD') AS effective_to \
             FROM people_position_placements a JOIN people_positions p ON p.id = a.position_id \
             WHERE a.id = $1 AND p.company_id = $2 FOR UPDATE OF a",
            &[&id, &company_id],
        )
        .await
        .map_err(Error::from)?
        .as_ref()
        .map(placement)
        .transpose()
}

/// Ends each assignment on its day and records the audit action (`event`)
/// as performed by the scope's actor, in the same transaction; each row is a
/// `people_position_placement` change.
pub(crate) async fn end_placements(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company_id: i64,
    ending: Vec<(Assignment, Date)>,
    event: &'static str,
) -> Result<Vec<Assignment>> {
    let who = Attribution::from_scope(scope);
    let ended = tx
        .mutate(move |sql| {
            Box::pin(async move {
                let mut ended = Vec::with_capacity(ending.len());
                let mut changes = Vec::with_capacity(ending.len());
                for (assignment, effective_to) in ending {
                    let row = sql
                        .query_one(
                            &format!(
                                "UPDATE people_position_placements SET effective_to = $2::text::date WHERE id = $1 RETURNING {PLACEMENT}"
                            ),
                            &[&assignment.id, &effective_to.to_string()],
                        )
                        .await?;
                    let after = placement(&row).map_err(|_| Error::Internal)?;
                    record_action(
                        sql,
                        &who,
                        None,
                        event,
                        json!({
                            "company_id": company_id,
                            "assignment_id": after.id,
                            "position_id": after.position_id,
                            "employee_id": after.employee_id,
                            "previous_effective_to": text(assignment.effective_to),
                            "effective_to": effective_to.to_string(),
                        }),
                        true,
                    )
                    .await?;
                    changes.push(Change {
                        kind: "people_position_placement".into(),
                        id: after.id,
                        old: assignment.audit_values(),
                        new: after.audit_values(),
                    });
                    ended.push(after);
                }
                Ok((ended, changes))
            })
        })
        .await?;
    Ok(ended)
}

/// The assignments of the positions effective on `day`, by position then id,
/// at most [`ASSIGNMENT_LIMIT`]; past it, the position id of the first
/// omitted row, from which holders are incomplete.
pub(crate) async fn effective_assignments(
    client: &impl GenericClient,
    ids: &[i64],
    day: Date,
) -> Result<(Vec<Assignment>, Option<i64>)> {
    let limit = ASSIGNMENT_LIMIT + 1;
    let mut rows: Vec<Assignment> = client
        .query(
            &format!(
                "SELECT {PLACEMENT} FROM people_position_placements WHERE position_id = ANY($1::bigint[]) \
                 AND effective_from <= $2::text::date AND (effective_to IS NULL OR effective_to >= $2::text::date) \
                 ORDER BY position_id, id LIMIT $3"
            ),
            &[&ids, &day.to_string(), &limit],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(placement)
        .collect::<Result<_>>()?;
    let truncated_at = if rows.len() as i64 > ASSIGNMENT_LIMIT {
        rows.pop().map(|assignment| assignment.position_id)
    } else {
        None
    };
    Ok((rows, truncated_at))
}

/// The substantive assignments of the positions effective on `day`.
pub(crate) async fn substantive_assignments(
    client: &impl GenericClient,
    ids: &[i64],
    day: Date,
) -> Result<Vec<Assignment>> {
    client
        .query(
            &format!(
                "SELECT {PLACEMENT} FROM people_position_placements WHERE position_id = ANY($1::bigint[]) \
                 AND kind = 'substantive' AND effective_from <= $2::text::date \
                 AND (effective_to IS NULL OR effective_to >= $2::text::date) ORDER BY id"
            ),
            &[&ids, &day.to_string()],
        )
        .await
        .map_err(Error::from)?
        .iter()
        .map(placement)
        .collect()
}
