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
    dependencies: &["base/artifacts", "base/authz", "base/database", "base/events", "base/menu", "base/ui", "base/datetime", "base/module_registry", "base/party_ids", "base/settings", "base/tenancy", "core/company", bilibi_people_settings::MODULE, bilibi_people_workforce::MODULE, bilibi_people_attendance::MODULE, bilibi_people_leave::MODULE, bilibi_people_claims::MODULE],
    migrations: &[("people-payroll-001-foundation", include_str!("../migrations/001-foundation.sql")), ("people-payroll-002-attendance-allowance-mappings", include_str!("../migrations/002-attendance-allowance-mappings.sql")), ("people-payroll-003-outputs", include_str!("../migrations/003-outputs.sql"))],
    services: &[Service { id: SERVICE, factory: |s| Ok(Arc::new(Payroll {
        db: s.get::<Db>("base/database.pool")?.as_ref().clone(),
        mutations: s.get::<MutationBoundary>("base/database.mutations")?,
        settings: s.get("base/settings.store")?,
        workforce: s.get(bilibi_people_workforce::SERVICE)?,
        leave: s.get(bilibi_people_leave::SERVICE)?,
        claims: s.get(bilibi_people_claims::SERVICE)?,
        attendance: s.get(bilibi_people_attendance::SERVICE)?,
        events: s.get(bilibi_events::SERVICE)?,
        artifacts: s.get("base/artifacts.store")?,
    })) }, Service { id: crate::web::RUN_PAGES, factory: |s| Ok(Arc::new(bilibi_ui::page::PageService::<bilibi_tenancy::Scope, bilibi_database::Error>(Arc::new(crate::web::Runs::new(
        s.get::<Db>("base/database.pool")?.as_ref().clone(), s.get(SERVICE)?
    ))))) }, Service { id: crate::web::PAGES, factory: |s| Ok(Arc::new(bilibi_ui::page::PageService::<bilibi_tenancy::Scope, bilibi_database::Error>(Arc::new(crate::web::Pages::new(
        s.get::<Db>("base/database.pool")?.as_ref().clone(), s.get(SERVICE)?
    ))))) }, Service { id: crate::download::SERVICE, factory: |s| Ok(Arc::new(
        bilibi_ui::download::DownloadService::<bilibi_tenancy::Scope, bilibi_database::Error>(
            Arc::new(crate::download::DocumentDownload { payroll: s.get(SERVICE)? })
        )
    )) }],
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

/// Facts released to other Domains only after independent approval.
pub const RUN_APPROVED: &str = "people/payroll.run_approved";
inventory::submit! { Contribution::new(bilibi_events::EVENTS, RUN_APPROVED, provenance!(), &bilibi_events::EventType { version: 1 }) }

// PY4: Bilimbi Payroll.Web.SetupLive and AttendanceMappingsLive.
inventory::submit! { Contribution::new(bilibi_module_registry::contributions::PAGE_WRITES, crate::web::PAGES, provenance!(), &bilibi_module_registry::contributions::PageWrites {
    service: crate::web::PAGES, contracts: &[],
    pending: bilibi_module_registry::contributions::Pending::Every { reason: "Payroll setup writes use the authorized facade; their operation contracts have not been reviewed." },
}) }
macro_rules! page {
    ($id:literal, $path:literal, $operation:literal, $cap:expr) => {
        inventory::submit! { Contribution::new(bilibi_module_registry::contributions::ROUTES, $id, provenance!(), &bilibi_module_registry::contributions::Route {
            method: bilibi_module_registry::contributions::Method::Get, path: $path, service: crate::web::PAGES, operation: $operation,
            access: bilibi_module_registry::contributions::Access::Capability($cap), framed: true,
        }) }
    };
}
page!(
    "people/payroll.setup",
    "/people/payroll/setup",
    "setup",
    VIEW
);
page!(
    "people/payroll.mappings",
    "/people/payroll/setup/mappings",
    "mappings",
    VIEW
);
page!(
    "people/payroll.attendance_mappings",
    "/people/payroll/attendance-mappings",
    "attendance_mappings",
    ATTENDANCE_MAPPINGS_MANAGE
);
bilibi_menu::contribute!(
    "people/payroll",
    bilibi_menu::MenuItem::page(
        "people.payroll.mappings",
        "Pay-item mappings",
        "/people/payroll/setup/mappings"
    )
    .icon("squares-2x2")
    .under("people.payroll")
    .requires(bilibi_menu::Requirement::Capability(VIEW))
    .order(20)
);
bilibi_menu::contribute!(
    "people/payroll",
    bilibi_menu::MenuItem::page(
        "people.settings.payroll_setup",
        "Payroll setup",
        "/people/payroll/setup"
    )
    .icon("cog-6-tooth")
    .under("people.settings")
    .requires(bilibi_menu::Requirement::Capability(VIEW))
    .order(70)
);

// Bilimbi Payroll.Web.DocumentController: access is checked on every request.
inventory::submit! { Contribution::new(
    bilibi_module_registry::contributions::DOWNLOADS,
    "people/payroll.documents.download", provenance!(),
    &bilibi_module_registry::contributions::Download {
        path: "/people/payroll/documents/{id}",
        service: crate::download::SERVICE, operation: "document",
        access: bilibi_module_registry::contributions::Access::Capability(VIEW),
    }
) }

// PY5: Bilimbi Payroll.Web.RunsLive.
inventory::submit! { Contribution::new(bilibi_module_registry::contributions::PAGE_WRITES, crate::web::RUN_PAGES, provenance!(), &bilibi_module_registry::contributions::PageWrites {
    service: crate::web::RUN_PAGES, contracts: &[],
    pending: bilibi_module_registry::contributions::Pending::Every { reason: "Payroll run writes use the authorized facade; their operation contracts have not been reviewed." },
}) }
inventory::submit! { Contribution::new(bilibi_module_registry::contributions::ROUTES, "people/payroll.runs", provenance!(), &bilibi_module_registry::contributions::Route {
    method: bilibi_module_registry::contributions::Method::Get, path: "/people/payroll/runs", service: crate::web::RUN_PAGES, operation: "runs",
    access: bilibi_module_registry::contributions::Access::Capability(VIEW), framed: true,
}) }
bilibi_menu::contribute!(
    "people/payroll",
    bilibi_menu::MenuItem::page("people.payroll.runs", "Runs", "/people/payroll/runs")
        .icon("circle-stack")
        .under("people.payroll")
        .requires(bilibi_menu::Requirement::Capability(VIEW))
        .order(10)
);
