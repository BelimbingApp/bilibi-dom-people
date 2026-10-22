//! What Leave adds to the shared catalogs (Bilimbi
//! `Bilimbi.People.Leave.Contributions`): the module descriptor with its
//! three migrations and facade service, the three capabilities under the
//! `people` Authz domain (and the verb `approve`, which Base does not
//! declare), and the three company settings. The menu leaves land with their
//! pages (PL3, PL4).
use crate::{Leave, MODULE, SERVICE};
use bilibi_authz::{CapabilityDefinition as C, VocabularyContribution, VOCABULARY};
use bilibi_database::{Db, MutationBoundary};
use bilibi_module_registry::{Contribution, Layer, Module, Provenance, Service};
use bilibi_settings::{Definition, Editor, ScopeKind, Setting, Validator, ValueType, DEFINITIONS};
use serde_json::{json, Value};
use std::sync::Arc;

/// Sees and requests one's own leave (`/people/leave/my`).
pub const SELF_VIEW: &str = "people.leave.self.view";
/// Sees a company's pending requests and decides them (`/people/leave/requests`).
pub const REQUESTS_APPROVE: &str = "people.leave.requests.approve";
/// Edits a company's leave year, rules, types and policies, grants
/// entitlements, records ledger entries and runs carry-forward
/// (`/people/leave/policies`).
pub const POLICIES_MANAGE: &str = "people.leave.policies.manage";
/// Every Leave capability.
pub const CAPABILITIES: [&str; 3] = [SELF_VIEW, REQUESTS_APPROVE, POLICIES_MANAGE];

/// The month a company's leave year begins (1 to 12; default January).
pub const YEAR_START_MONTH_KEY: &str = "people.leave.year_start_month";
/// ISO weekday numbers counted as leave days (default Monday to Friday).
pub const WORKING_WEEKDAYS_KEY: &str = "people.leave.working_weekdays";
/// How many days before today a request may start (0 to 366; default 30).
pub const REQUEST_BACKDATE_DAYS_KEY: &str = "people.leave.request_backdate_days";

macro_rules! provenance {
    () => {
        Provenance {
            module: MODULE,
            source: file!(),
            line: line!(),
        }
    };
}

inventory::submit! {
    Module {
        id: MODULE,
        layer: Layer::Domains,
        dependencies: &[
            "base/authz", "base/database", "base/module_registry", "base/party_ids",
            "base/settings", "base/tenancy", "core/company",
            "people/settings", "people/workforce",
        ],
        migrations: &[
            ("people-leave-001-core", include_str!("../migrations/001-core.sql")),
            ("people-leave-002-applications", include_str!("../migrations/002-applications.sql")),
            ("people-leave-003-carry-forward-skips", include_str!("../migrations/003-carry-forward-skips.sql")),
        ],
        services: &[Service {
            id: SERVICE,
            factory: |services| {
                Ok(Arc::new(Leave {
                    db: services.get::<Db>("base/database.pool")?.as_ref().clone(),
                    mutations: services.get::<MutationBoundary>("base/database.mutations")?,
                    settings: services.get("base/settings.store")?,
                    workforce: services.get(bilibi_people_workforce::SERVICE)?,
                }))
            },
        }],
    }
}

// The Authz domain is `people`, as Bilimbi declares it ("People domain
// modules"). `view` and `manage` are verbs Base already declares; `approve`
// is not, so Leave declares it (the vocabulary refuses a key whose last
// segment no module declared). Bilimbi grants none of these to a seed role.
inventory::submit! {
    Contribution::new(VOCABULARY, "people/leave.vocabulary", provenance!(), &VocabularyContribution {
        domains: &["people"],
        verbs: &["approve"],
        capabilities: &[C::tenant(SELF_VIEW), C::tenant(REQUESTS_APPROVE), C::tenant(POLICIES_MANAGE)],
        roles: &[],
        system_principals: &[],
    })
}

/// A non-empty list of distinct ISO weekday numbers, 1 (Monday) to 7.
pub(crate) fn working_weekdays_valid(value: &Value) -> bool {
    value.as_array().is_some_and(|days| {
        !days.is_empty()
            && days.iter().enumerate().all(|(index, day)| {
                day.as_i64().is_some_and(|day| (1..=7).contains(&day))
                    && !days[..index].contains(day)
            })
    })
}

inventory::submit! { Contribution::new(DEFINITIONS, YEAR_START_MONTH_KEY, provenance!(), &Setting::Definition(Definition {
    value_type: ValueType::Integer, scopes: &[ScopeKind::Company], default: || json!(1),
    nullable: false, encrypted: false, minimum: Some(1.0), maximum: Some(12.0), validator: None,
    editor: Some(Editor { group: "People", label: "Leave year start month", help: "Month in which this company's leave year begins; entitlements are granted per leave year.", capability: Some(POLICIES_MANAGE), reveal_duration_ms: 5_000 }),
})) }

inventory::submit! { Contribution::new(DEFINITIONS, WORKING_WEEKDAYS_KEY, provenance!(), &Setting::Definition(Definition {
    value_type: ValueType::Array, scopes: &[ScopeKind::Company], default: || json!([1, 2, 3, 4, 5]),
    nullable: false, encrypted: false, minimum: None, maximum: None,
    validator: Some(Validator { accepts: working_weekdays_valid, message: "Give one or more ISO weekday numbers, 1 (Monday) to 7, each once" }),
    editor: Some(Editor { group: "People", label: "Leave working weekdays", help: "ISO weekday numbers (1 is Monday) counted as leave days; other weekdays and calendar exceptions are not.", capability: Some(POLICIES_MANAGE), reveal_duration_ms: 5_000 }),
})) }

inventory::submit! { Contribution::new(DEFINITIONS, REQUEST_BACKDATE_DAYS_KEY, provenance!(), &Setting::Definition(Definition {
    value_type: ValueType::Integer, scopes: &[ScopeKind::Company], default: || json!(30),
    nullable: false, encrypted: false, minimum: Some(0.0), maximum: Some(366.0), validator: None,
    editor: Some(Editor { group: "People", label: "Leave request backdating", help: "How many days before today a leave request may start.", capability: Some(POLICIES_MANAGE), reveal_duration_ms: 5_000 }),
})) }
