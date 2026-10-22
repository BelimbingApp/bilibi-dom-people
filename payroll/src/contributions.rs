//! Bilimbi People Payroll.Contributions; page/menu contributions follow in PY4/PY5.
use crate::{Payroll, MODULE, SERVICE};
use bilibi_authz::{CapabilityDefinition as C, RoleDefinition, VocabularyContribution, VOCABULARY};
use bilibi_database::{Db, MutationBoundary};
use bilibi_module_registry::{Contribution, Layer, Module, Provenance, Service};
use bilibi_settings::{Definition, Editor, ScopeKind, Setting, Validator, ValueType, DEFINITIONS};
use serde_json::{json, Value};
use std::sync::Arc;
pub const VIEW: &str = "people.payroll.view";
pub const MANAGE: &str = "people.payroll.manage";
pub const APPROVE: &str = "people.payroll.approve";
pub const ATTENDANCE_MAPPINGS_MANAGE: &str = "people.payroll.attendance-mappings.manage";
pub const CAPABILITIES: [&str; 4] = [VIEW, MANAGE, APPROVE, ATTENDANCE_MAPPINGS_MANAGE];
pub const COUNTRY_KEY: &str = "people.payroll.country";
pub const CURRENCIES_KEY: &str = "people.payroll.currencies";
macro_rules! provenance {
    () => {
        Provenance {
            module: MODULE,
            source: file!(),
            line: line!(),
        }
    };
}
inventory::submit! { Module {
    id: MODULE, layer: Layer::Domains,
    dependencies: &["base/authz", "base/database", "base/module_registry", "base/party_ids", "base/settings", "base/tenancy", "core/company", bilibi_people_settings::MODULE, bilibi_people_workforce::MODULE, bilibi_people_attendance::MODULE, bilibi_people_leave::MODULE, bilibi_people_claims::MODULE],
    migrations: &[("people-payroll-001-foundation", include_str!("../migrations/001-foundation.sql")), ("people-payroll-002-attendance-allowance-mappings", include_str!("../migrations/002-attendance-allowance-mappings.sql"))],
    services: &[Service { id: SERVICE, factory: |s| Ok(Arc::new(Payroll {
        db: s.get::<Db>("base/database.pool")?.as_ref().clone(),
        mutations: s.get::<MutationBoundary>("base/database.mutations")?,
        settings: s.get("base/settings.store")?,
        workforce: s.get(bilibi_people_workforce::SERVICE)?,
        leave: s.get(bilibi_people_leave::SERVICE)?,
        claims: s.get(bilibi_people_claims::SERVICE)?,
        attendance: s.get(bilibi_people_attendance::SERVICE)?,
    })) }],
} }
inventory::submit! { Contribution::new(VOCABULARY, "people/payroll.vocabulary", provenance!(), &VocabularyContribution {
    domains: &["people"], verbs: &["approve"],
    capabilities: &[C::tenant(VIEW), C::tenant(MANAGE), C::tenant(APPROVE), C::tenant(ATTENDANCE_MAPPINGS_MANAGE)],
    roles: &[RoleDefinition { code: "tenant_owner", name: "Tenant Owner", description: "Contributed system role; reconciled explicitly", grant_all: false, capabilities: &[VIEW, MANAGE, APPROVE] }], system_principals: &[],
}) }
pub(crate) fn currencies_valid(value: &Value) -> bool {
    value.as_array().is_some_and(|codes| {
        codes.len() <= 20
            && codes
                .iter()
                .enumerate()
                .all(|(i, c)| c.as_str().is_some_and(currency_shaped) && !codes[..i].contains(c))
    })
}
pub(crate) fn currency_shaped(code: &str) -> bool {
    code.len() == 3 && code.bytes().all(|b| b.is_ascii_uppercase())
}
inventory::submit! { Contribution::new(DEFINITIONS, COUNTRY_KEY, provenance!(), &Setting::Definition(Definition {
    value_type: ValueType::String, scopes: &[ScopeKind::Company], default: || json!(""), nullable: false, encrypted: false, minimum: None, maximum: None,
    validator: Some(Validator { accepts: |v| v.as_str().is_some_and(|s| s.chars().count() <= 100), message: "Enter a country identifier of up to 100 characters" }),
    editor: Some(Editor { group: "People", label: "Payroll country", help: "Operator-selected jurisdiction identifier; no statutory pack is activated.", capability: Some(MANAGE), reveal_duration_ms: 5_000 }),
})) }
inventory::submit! { Contribution::new(DEFINITIONS, CURRENCIES_KEY, provenance!(), &Setting::Definition(Definition {
    value_type: ValueType::Array, scopes: &[ScopeKind::Company], default: || json!([]), nullable: false, encrypted: false, minimum: None, maximum: None,
    validator: Some(Validator { accepts: currencies_valid, message: "Give up to twenty uppercase three-letter currency codes, each once" }),
    editor: Some(Editor { group: "People", label: "Payroll currencies", help: "Explicit currency codes accepted for new pay items and runs.", capability: Some(MANAGE), reveal_duration_ms: 5_000 }),
})) }
