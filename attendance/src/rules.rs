//! The company's five attendance rules, read and written as Base Settings
//! at the company target (Bilimbi `rules/2`, `put_rules/3`). Private to the
//! facade.
use crate::{
    AttendanceError, Result, ADJUSTMENT_WINDOW_KEY, LOCATION_REQUIRED_KEY, MAX_SHIFT_KEY,
    RULE_KEYS, SELF_CLOCK_KEY, TIMEZONE_KEY,
};
use bilibi_database::MutationTransaction;
use bilibi_party_ids::OrganisationId;
use bilibi_people_workforce::Company;
use bilibi_settings::{Settings, Target};
use bilibi_tenancy::{ReadClient, Scope};
use serde::Serialize;
use serde_json::json;

/// The company's rules as they resolve now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Rules {
    /// The IANA time zone that gives a clock event its local date.
    pub timezone: String,
    /// Whether linked employee accounts may clock themselves.
    pub self_clock_enabled: bool,
    /// Hours after clock-in that a shift stays open (1 to 24).
    pub max_shift_hours: i64,
    /// Whether a clock event needs coordinates inside an active location.
    pub location_required: bool,
    /// Days, counting today, open to an adjustment request (1 to 366).
    pub adjustment_window_days: i64,
}

/// What `put_rules` changes; a `None` keeps the current value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RuleChanges {
    pub timezone: Option<String>,
    pub self_clock_enabled: Option<bool>,
    pub max_shift_hours: Option<i64>,
    pub location_required: Option<bool>,
    pub adjustment_window_days: Option<i64>,
}
impl RuleChanges {
    pub fn is_empty(&self) -> bool {
        self.timezone.is_none()
            && self.self_clock_enabled.is_none()
            && self.max_shift_hours.is_none()
            && self.location_required.is_none()
            && self.adjustment_window_days.is_none()
    }
}

fn target(company: &Company) -> Result<Target> {
    Ok(Target::Company(OrganisationId::new(
        company.platform_company_id,
    )?))
}

/// The five rules at the company target, defaults where nothing is stored.
pub(crate) async fn read(
    client: &dyn ReadClient,
    settings: &Settings,
    scope: &Scope,
    company: &Company,
) -> Result<Rules> {
    let resolved = settings
        .resolve_many(client, scope, target(company)?, &RULE_KEYS)
        .await?;
    let value = |key: &str| resolved.get(key).map(|resolved| &resolved.value);
    Ok(Rules {
        timezone: value(TIMEZONE_KEY)
            .and_then(|value| value.as_str())
            .unwrap_or("Etc/UTC")
            .to_owned(),
        self_clock_enabled: value(SELF_CLOCK_KEY)
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        max_shift_hours: value(MAX_SHIFT_KEY)
            .and_then(|value| value.as_i64())
            .unwrap_or(16),
        location_required: value(LOCATION_REQUIRED_KEY)
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        adjustment_window_days: value(ADJUSTMENT_WINDOW_KEY)
            .and_then(|value| value.as_i64())
            .unwrap_or(7),
    })
}

/// Bilimbi `validate_rules/1`: a known time zone, a shift length of 1 to
/// 24 hours, a window of 1 to 366 days.
pub(crate) fn validate(changes: &RuleChanges) -> Result<()> {
    let valid = changes
        .timezone
        .as_deref()
        .is_none_or(bilibi_datetime::valid_timezone)
        && changes
            .max_shift_hours
            .is_none_or(|hours| (1..=24).contains(&hours))
        && changes
            .adjustment_window_days
            .is_none_or(|days| (1..=366).contains(&days));
    if valid {
        Ok(())
    } else {
        Err(AttendanceError::InvalidRules)
    }
}

/// Stores each given rule at the company target in the caller's
/// transaction; Settings records every value as a change.
pub(crate) async fn write(
    tx: &mut MutationTransaction<'_>,
    settings: &Settings,
    scope: &Scope,
    company: &Company,
    changes: &RuleChanges,
) -> Result<()> {
    let target = target(company)?;
    let values = [
        (
            TIMEZONE_KEY,
            changes.timezone.as_ref().map(|zone| json!(zone)),
        ),
        (
            SELF_CLOCK_KEY,
            changes.self_clock_enabled.map(|on| json!(on)),
        ),
        (
            MAX_SHIFT_KEY,
            changes.max_shift_hours.map(|hours| json!(hours)),
        ),
        (
            LOCATION_REQUIRED_KEY,
            changes.location_required.map(|on| json!(on)),
        ),
        (
            ADJUSTMENT_WINDOW_KEY,
            changes.adjustment_window_days.map(|days| json!(days)),
        ),
    ];
    for (key, value) in values {
        if let Some(value) = value {
            settings.put(tx, scope, target, key, value).await?;
        }
    }
    Ok(())
}
