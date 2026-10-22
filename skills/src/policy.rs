//! The company's skills policy (Bilimbi `Bilimbi.People.Skills.Policy`):
//! seven company-scoped Base Settings that steer assessments, development
//! actions and reminders. Historical facts snapshot the value they used.
//! The definitions are contributed in `contributions.rs` from the table
//! below; the facade reads and writes them here.
use crate::{PolicyChanges as Changes, Result, SkillsError};
use bilibi_database::{Error, MutationTransaction};
use bilibi_party_ids::OrganisationId;
use bilibi_settings::{Settings, Target};
use bilibi_tenancy::Scope;
use serde::Serialize;
use serde_json::{json, Value};
use tokio_postgres::GenericClient;

/// One policy value: its setting key, default and inclusive range.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Key {
    pub key: &'static str,
    pub default: i64,
    pub min: i64,
    pub max: i64,
    pub label: &'static str,
    pub help: &'static str,
}

pub(crate) const REASSESSMENT_DUE_DAYS: Key = Key {
    key: "people.skills.reassessment_due_days",
    default: 30,
    min: 1,
    max: 365,
    label: "Reassessment request window",
    help: "Days after a team lead requests a reassessment by which it falls due.",
};
pub(crate) const DEFAULT_REASSESSMENT_MONTHS: Key = Key {
    key: "people.skills.default_reassessment_months",
    default: 12,
    min: 1,
    max: 120,
    label: "Default reassessment interval",
    help: "Months after an assessment when the skill is next due, for a skill with no interval of its own and an assessment with no validity date.",
};
pub(crate) const REMINDER_WINDOW_DAYS: Key = Key {
    key: "people.skills.reminder_window_days",
    default: 30,
    min: 0,
    max: 365,
    label: "Validity reminder window",
    help: "Days before a skill's validity ends that its reminder becomes due.",
};
pub(crate) const BACKUP_MINIMUM: Key = Key {
    key: "people.skills.backup_minimum",
    default: 2,
    min: 1,
    max: 100,
    label: "Critical-skill backup minimum",
    help: "Employees who must hold a critical skill at their required level before it counts as covered.",
};
pub(crate) const MULTIPLIER_CRITICAL: Key = Key {
    key: "people.skills.priority_multiplier_critical",
    default: 3,
    min: 0,
    max: 100,
    label: "Critical priority multiplier",
    help: "Multiplies a critical requirement's gap into its priority score.",
};
pub(crate) const MULTIPLIER_ESSENTIAL: Key = Key {
    key: "people.skills.priority_multiplier_essential",
    default: 2,
    min: 0,
    max: 100,
    label: "Essential priority multiplier",
    help: "Multiplies an essential requirement's gap into its priority score.",
};
pub(crate) const MULTIPLIER_DEVELOPMENT: Key = Key {
    key: "people.skills.priority_multiplier_development",
    default: 1,
    min: 0,
    max: 100,
    label: "Development priority multiplier",
    help: "Multiplies a development requirement's gap into its priority score.",
};

/// The seven setting keys, in the order the policy page shows them.
pub const POLICY_KEYS: [&str; 7] = [
    REASSESSMENT_DUE_DAYS.key,
    DEFAULT_REASSESSMENT_MONTHS.key,
    REMINDER_WINDOW_DAYS.key,
    BACKUP_MINIMUM.key,
    MULTIPLIER_CRITICAL.key,
    MULTIPLIER_ESSENTIAL.key,
    MULTIPLIER_DEVELOPMENT.key,
];

/// The company's assessment, priority and reminder policy values (Bilimbi
/// `Policy.get/2`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Policy {
    pub reassessment_due_days: i64,
    pub default_reassessment_months: i64,
    pub reminder_window_days: i64,
    pub backup_minimum: i64,
    pub multiplier_critical: i64,
    pub multiplier_essential: i64,
    pub multiplier_development: i64,
}

impl Policy {
    /// The multiplier for a requirement's criticality (Bilimbi
    /// `Policy.multiplier/2`); an unknown criticality multiplies by nothing.
    pub fn multiplier(&self, criticality: &str) -> i64 {
        match criticality {
            "critical" => self.multiplier_critical,
            "essential" => self.multiplier_essential,
            "development" => self.multiplier_development,
            _ => 0,
        }
    }
}

/// The values a policy write changes; `None` keeps the current value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PolicyChanges {
    pub reassessment_due_days: Option<i64>,
    pub default_reassessment_months: Option<i64>,
    pub reminder_window_days: Option<i64>,
    pub backup_minimum: Option<i64>,
    pub multiplier_critical: Option<i64>,
    pub multiplier_essential: Option<i64>,
    pub multiplier_development: Option<i64>,
}

impl PolicyChanges {
    fn entries(&self) -> [(Key, Option<i64>); 7] {
        [
            (REASSESSMENT_DUE_DAYS, self.reassessment_due_days),
            (
                DEFAULT_REASSESSMENT_MONTHS,
                self.default_reassessment_months,
            ),
            (REMINDER_WINDOW_DAYS, self.reminder_window_days),
            (BACKUP_MINIMUM, self.backup_minimum),
            (MULTIPLIER_CRITICAL, self.multiplier_critical),
            (MULTIPLIER_ESSENTIAL, self.multiplier_essential),
            (MULTIPLIER_DEVELOPMENT, self.multiplier_development),
        ]
    }
}

/// Bilimbi `Policy.validate/1`: at least one value, each within its range.
pub(crate) fn validate(changes: &Changes) -> Result<()> {
    let given: Vec<(Key, i64)> = changes
        .entries()
        .into_iter()
        .filter_map(|(key, value)| value.map(|value| (key, value)))
        .collect();
    if given.is_empty()
        || given
            .iter()
            .any(|(key, value)| *value < key.min || *value > key.max)
    {
        return Err(SkillsError::InvalidPolicy);
    }
    Ok(())
}

fn target(company_id: i64) -> Result<Target> {
    Ok(Target::Company(OrganisationId::new(company_id)?))
}

fn value_or_default(resolved: Option<&Value>, key: &Key) -> i64 {
    resolved.and_then(Value::as_i64).unwrap_or(key.default)
}

/// The resolved values; one that escaped its definition reads as the default.
pub(crate) async fn get(
    settings: &Settings,
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company_id: i64,
) -> Result<Policy> {
    let resolved = settings
        .resolve_many(client, scope, target(company_id)?, &POLICY_KEYS)
        .await?;
    let value = |key: &Key| value_or_default(resolved.get(key.key).map(|r| &r.value), key);
    Ok(Policy {
        reassessment_due_days: value(&REASSESSMENT_DUE_DAYS),
        default_reassessment_months: value(&DEFAULT_REASSESSMENT_MONTHS),
        reminder_window_days: value(&REMINDER_WINDOW_DAYS),
        backup_minimum: value(&BACKUP_MINIMUM),
        multiplier_critical: value(&MULTIPLIER_CRITICAL),
        multiplier_essential: value(&MULTIPLIER_ESSENTIAL),
        multiplier_development: value(&MULTIPLIER_DEVELOPMENT),
    })
}

/// Stores each given value through Base Settings on the caller's
/// transaction; the definitions refuse a value outside their range.
pub(crate) async fn put(
    settings: &Settings,
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company_id: i64,
    changes: &Changes,
) -> Result<()> {
    let target = target(company_id)?;
    for (key, value) in changes.entries() {
        if let Some(value) = value {
            settings
                .put(tx, scope, target, key.key, json!(value))
                .await
                .map_err(|error| match error {
                    Error::Invalid => SkillsError::InvalidPolicy,
                    other => other.into(),
                })?;
        }
    }
    Ok(())
}
