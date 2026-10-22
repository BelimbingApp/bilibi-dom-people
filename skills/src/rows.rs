//! Row and audit helpers the private SQL modules share: dates and UTC wall
//! clocks read as ISO text (the workspace's `tokio-postgres` has no date
//! feature, and a Domain must not turn one on), and the `Change` each write
//! reports to the mutation boundary (an empty object on the absent side is
//! what names the audit event `created` or `deleted`).
use bilibi_database::{Change, Error, Result};
use jiff::civil::{Date, DateTime};
use serde::Serialize;
use serde_json::{json, Value};
use tokio_postgres::Row;

/// `to_char(col, 'YYYY-MM-DD')` back to a date.
pub(crate) fn date(row: &Row, column: &str) -> Result<Option<Date>> {
    row.get::<_, Option<String>>(column)
        .map(|text| text.parse::<Date>().map_err(|_| Error::Internal))
        .transpose()
}

/// `to_char(col, 'YYYY-MM-DD"T"HH24:MI:SS')` back to a UTC wall clock.
pub(crate) fn datetime(row: &Row, column: &str) -> Result<Option<DateTime>> {
    row.get::<_, Option<String>>(column)
        .map(|text| text.parse::<DateTime>().map_err(|_| Error::Internal))
        .transpose()
}

/// The SQL that reads a date column as ISO text.
pub(crate) fn date_text(column: &str) -> String {
    format!(
        "to_char({column}, 'YYYY-MM-DD') AS {}",
        column.rsplit('.').next().unwrap_or(column)
    )
}

/// The SQL that reads a `timestamp(0)` column as ISO text.
pub(crate) fn datetime_text(column: &str) -> String {
    format!(
        "to_char({column}, 'YYYY-MM-DD\"T\"HH24:MI:SS') AS {}",
        column.rsplit('.').next().unwrap_or(column)
    )
}

fn values(record: &impl Serialize) -> Value {
    serde_json::to_value(record).unwrap_or_default()
}

pub(crate) fn created(kind: &str, id: i64, new: &impl Serialize) -> Change {
    Change {
        kind: kind.into(),
        id,
        old: json!({}),
        new: values(new),
    }
}

pub(crate) fn changed(kind: &str, id: i64, old: &impl Serialize, new: &impl Serialize) -> Change {
    Change {
        kind: kind.into(),
        id,
        old: values(old),
        new: values(new),
    }
}

pub(crate) fn deleted(kind: &str, id: i64, old: &impl Serialize) -> Change {
    Change {
        kind: kind.into(),
        id,
        old: values(old),
        new: json!({}),
    }
}
