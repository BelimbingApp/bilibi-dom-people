//! The `people/organisation.positions` port: Bilimbi's
//! `register_position_reader` seam, through which Workforce's `positions`
//! scan (and, through it, the People connector's native adapter) reads
//! bounded pages of a company's positions now, on the caller's client and
//! sealed scope, with no edge from Workforce to Organisation. Without
//! Organisation mounted the caller gets `Unavailable`, Bilimbi's unregistered
//! reader. The request and the answer are documented in `README.md`; that
//! shape is the whole contract.
//!
//! The read is the facade's own: it proves the live company and filters the
//! holders through Workforce, and it needs no capability, as Bilimbi's
//! position read does not. The position rows are read on the caller's
//! client, so a caller inside a transaction sees its own pending positions.
use crate::{
    Organisation, OrganisationError, PositionProjection, PAGE_SIZE,
};
use bilibi_database::Error;
use bilibi_ports::{answer, on_client, request, PortClient, PortFuture, PortProvider, Value};
use bilibi_tenancy::Scope;
use jiff::civil::Date;
use serde::Serialize;
use std::sync::{Arc, RwLock};

/// The facade the port answers through. A port provider is a static, so the
/// service factory hands the facade over here when the registry builds
/// `people/organisation.organisation`, before any port is called.
static FACADE: RwLock<Option<Arc<Organisation>>> = RwLock::new(None);

pub(crate) fn install(facade: Arc<Organisation>) {
    *FACADE
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(facade);
}

fn facade() -> Result<Arc<Organisation>, Error> {
    FACADE
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
        .ok_or(Error::Internal)
}

/// The documented answer: the page, and the scan bounds for a cursor step
/// (`null` for an offset page).
#[derive(Serialize)]
struct Answer {
    positions: Vec<PositionProjection>,
    next_cursor: Option<String>,
    high_water_id: Option<i64>,
}

const KEYS: [&str; 5] = ["company_id", "as_of", "page_size", "page", "cursor"];

pub(crate) struct PositionsPort;
impl PortProvider for PositionsPort {
    fn call<'a>(&'a self, client: PortClient<'a>, scope: &'a Scope, body: Value) -> PortFuture<'a> {
        Box::pin(async move {
            // `company_id` and `as_of`, an optional `page_size`, and exactly one
            // of `page` (an offset page) or `cursor` (a scan step; `null`
            // starts the scan); nothing else.
            let body: serde_json::Map<String, Value> = request(body)?;
            if body.keys().any(|key| !KEYS.contains(&key.as_str())) {
                return Err(Error::Invalid);
            }
            let company_id = body
                .get("company_id")
                .and_then(Value::as_i64)
                .ok_or(Error::Invalid)?;
            let as_of: Date = body
                .get("as_of")
                .and_then(Value::as_str)
                .ok_or(Error::Invalid)?
                .parse()
                .map_err(|_| Error::Invalid)?;
            let page_size = match body.get("page_size") {
                None => PAGE_SIZE,
                Some(size) => size.as_i64().ok_or(Error::Invalid)?,
            };
            let facade = facade()?;
            let answered = match (body.get("page"), body.get("cursor")) {
                (Some(page), None) => {
                    let page = page.as_i64().ok_or(Error::Invalid)?;
                    let positions = on_client!(client, |c| facade
                        .positions_on(c, scope, company_id, as_of, page, page_size)
                        .await)
                    .map_err(refused)?;
                    Answer {
                        positions,
                        next_cursor: None,
                        high_water_id: None,
                    }
                }
                (None, Some(cursor)) => {
                    let cursor = match cursor {
                        Value::Null => None,
                        Value::String(cursor) => Some(cursor.as_str()),
                        _ => return Err(Error::Invalid),
                    };
                    let scan = on_client!(client, |c| facade
                        .scan_positions_on(c, scope, company_id, as_of, page_size, cursor)
                        .await)
                    .map_err(refused)?;
                    Answer {
                        positions: scan.positions,
                        next_cursor: scan.next_cursor,
                        high_water_id: Some(scan.high_water_id),
                    }
                }
                _ => return Err(Error::Invalid),
            };
            answer(&answered)
        })
    }
}

/// The facade's refusals as the port carries them: `Forbidden`, `NotFound`,
/// `Invalid` for options and cursors, or the failure itself.
fn refused(error: OrganisationError) -> Error {
    match error {
        OrganisationError::Unauthorized => Error::Forbidden,
        OrganisationError::NotFound | OrganisationError::CompanyArchived => Error::NotFound,
        OrganisationError::Failed(error) => error,
        OrganisationError::Cycle
        | OrganisationError::InvalidOptions
        | OrganisationError::InvalidCursor
        | OrganisationError::NotCurrent(_)
        | OrganisationError::Invalid(_) => Error::Invalid,
    }
}
