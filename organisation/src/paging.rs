//! The high-water keyset scan of a company's positions (Bilimbi
//! `Bilimbi.People.Organisation.PositionPaging`): the read a reconciling
//! consumer (Workforce's `positions` scan, and through it the People
//! connector) walks to completion. The first step fixes the high-water mark,
//! the last id the positions sequence issued (zero before any position
//! exists). It is monotonic, so deleting the highest position never lowers
//! it, and every step is bounded above by it: positions created during the
//! scan lie above the mark and are never mixed in, so a consumer may
//! deactivate only the ids at or below it that it did not see.
//!
//! A cursor is an opaque URL-safe transport of (version, tenant, company,
//! day, last id seen, high-water id); it is not an authorization token. The
//! facade reproves the live company before every step, including resumes,
//! and a cursor minted for another tenant, company or day is refused.
use crate::{OrganisationError, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use bilibi_database::Error;
use jiff::civil::Date;
use tokio_postgres::GenericClient;

/// What a cursor is bound to.
pub(crate) struct Boundary {
    pub tenant: i64,
    pub company_id: i64,
    pub day: Date,
}

/// Where a step reads: ids above `last_id`, at or below `high_water_id`.
pub(crate) struct Bounds {
    pub last_id: i64,
    pub high_water_id: i64,
}

const VERSION: &str = "1";
const CURSOR_LIMIT: usize = 256;

/// The bounds of the next step: from the sequence for a fresh scan, from the
/// cursor for a resume.
pub(crate) async fn bounds(
    client: &(impl GenericClient + Sync),
    boundary: &Boundary,
    cursor: Option<&str>,
) -> Result<Bounds> {
    match cursor {
        None => {
            let row = client
                .query_one(
                    "SELECT coalesce(pg_sequence_last_value(pg_get_serial_sequence('people_positions', 'id')::regclass), 0)",
                    &[],
                )
                .await
                .map_err(Error::from)?;
            Ok(Bounds {
                last_id: 0,
                high_water_id: row.get(0),
            })
        }
        Some(cursor) => decode(boundary, cursor).ok_or(OrganisationError::InvalidCursor),
    }
}

/// The cursor that resumes after `last_id`, bound to the boundary.
pub(crate) fn encode(boundary: &Boundary, last_id: i64, high_water_id: i64) -> String {
    URL_SAFE_NO_PAD.encode(format!(
        "{VERSION}:{}:{}:{}:{last_id}:{high_water_id}",
        boundary.tenant, boundary.company_id, boundary.day
    ))
}

fn decode(boundary: &Boundary, cursor: &str) -> Option<Bounds> {
    if cursor.len() > CURSOR_LIMIT {
        return None;
    }
    let payload = URL_SAFE_NO_PAD.decode(cursor).ok()?;
    let payload = String::from_utf8(payload).ok()?;
    let parts: Vec<&str> = payload.split(':').collect();
    let [version, tenant, company, day, last, high] = parts.as_slice() else {
        return None;
    };
    if *version != VERSION
        || tenant.parse::<i64>().ok()? != boundary.tenant
        || company.parse::<i64>().ok()? != boundary.company_id
        || day.parse::<Date>().ok()? != boundary.day
    {
        return None;
    }
    let last_id: i64 = last.parse().ok()?;
    let high_water_id: i64 = high.parse().ok()?;
    if last_id <= 0 || high_water_id < last_id {
        return None;
    }
    Some(Bounds {
        last_id,
        high_water_id,
    })
}
