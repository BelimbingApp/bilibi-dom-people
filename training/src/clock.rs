//! Local wall clocks, IANA zones and UTC instants (Bilimbi `Training.local_instant/2`
//! and `day_start/2`): the one place a session's entered time becomes the
//! instant the catalog stores, and the one place a calendar day gets its
//! bounds. Storage stays UTC; the delivery zone is kept beside it.
use crate::{Result, TrainingError};
use jiff::{
    civil::{Date, DateTime},
    tz::{AmbiguousOffset, TimeZone},
    Timestamp,
};

/// The longest zone name a session stores (`time_zone varchar(100)`).
pub(crate) const ZONE_LENGTH: usize = 100;

/// Converts a local wall clock (`YYYY-MM-DDTHH:MM` or with seconds, as a
/// `datetime-local` control posts it) in a named IANA zone to the UTC instant,
/// refusing what the zone cannot place: a wall clock inside a DST gap is
/// `NonexistentTime`, one inside an overlap `AmbiguousTime`, and an unknown
/// zone or unparsable clock `InvalidTimeZoneOrTime`. Seconds below one are
/// dropped, as Bilimbi truncates to the second.
pub fn local_instant(local: &str, zone: &str) -> Result<Timestamp> {
    let tz = time_zone(zone)?;
    let local = if local.len() == 16 {
        format!("{local}:00")
    } else {
        local.to_owned()
    };
    let wall: DateTime = local
        .parse()
        .map_err(|_| TrainingError::InvalidTimeZoneOrTime)?;
    let ambiguous = tz.to_ambiguous_timestamp(wall);
    let instant = match ambiguous.offset() {
        AmbiguousOffset::Unambiguous { .. } => ambiguous
            .unambiguous()
            .map_err(|_| TrainingError::InvalidTimeZoneOrTime)?,
        AmbiguousOffset::Gap { .. } => return Err(TrainingError::NonexistentTime),
        AmbiguousOffset::Fold { .. } => return Err(TrainingError::AmbiguousTime),
    };
    Ok(whole_second(instant))
}

/// The first UTC instant of a local calendar day in a named zone, resolving
/// a midnight DST gap to the first instant after it and a midnight overlap to
/// the earlier of the two, so a calendar bound never fails on a clock change.
pub fn day_start(date: Date, zone: &str) -> Result<Timestamp> {
    let tz = time_zone(zone)?;
    let ambiguous = tz.to_ambiguous_timestamp(date.at(0, 0, 0, 0));
    let instant = match ambiguous.offset() {
        AmbiguousOffset::Unambiguous { .. } => ambiguous.unambiguous(),
        // In a gap the offset before the change places midnight at the
        // transition instant itself: the first valid instant of the day.
        AmbiguousOffset::Gap { .. } => ambiguous.later(),
        AmbiguousOffset::Fold { .. } => ambiguous.earlier(),
    }
    .map_err(|_| TrainingError::InvalidTimeZoneOrTime)?;
    Ok(whole_second(instant))
}

/// A convertible IANA zone of at most [`ZONE_LENGTH`] bytes, by the same
/// rules as Base DateTime's `localization.timezone`.
fn time_zone(zone: &str) -> Result<TimeZone> {
    if zone.is_empty() || zone.len() > ZONE_LENGTH || !bilibi_datetime::valid_timezone(zone) {
        return Err(TrainingError::InvalidTimeZoneOrTime);
    }
    TimeZone::get(zone).map_err(|_| TrainingError::InvalidTimeZoneOrTime)
}

fn whole_second(instant: Timestamp) -> Timestamp {
    Timestamp::from_second(instant.as_second()).unwrap_or(instant)
}

/// A UTC instant as the catalog binds it (`timestamp(0)`, bound as text).
pub(crate) fn sql_text(instant: Timestamp) -> String {
    instant.strftime("%Y-%m-%d %H:%M:%S").to_string()
}

/// A stored instant read back as `to_char(col, 'YYYY-MM-DD"T"HH24:MI:SS"Z"')`.
pub(crate) fn from_sql_text(text: &str) -> Option<Timestamp> {
    text.parse().ok()
}
