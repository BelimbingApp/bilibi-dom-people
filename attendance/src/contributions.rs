//! What Attendance adds to the shared catalogs (Bilimbi
//! `Bilimbi.People.Attendance.Contributions`): the module descriptor with its
//! migration and facade service, the six capabilities under the `people`
//! Authz domain (and the `approve` and `record` verbs Base does not declare),
//! and the five company rule settings, each editable with `people.attendance.rules.manage`.
//! The menu leaves land with their pages (port plan rows PA3 to PA5).
use crate::{Attendance, MODULE, SERVICE};
use bilibi_authz::{CapabilityDefinition as C, VocabularyContribution, VOCABULARY};
use bilibi_database::{Db, MutationBoundary};
use bilibi_module_registry::{Contribution, Layer, Module, Provenance, Service};
use bilibi_settings::{Definition, Editor, ScopeKind, Setting, Validator, ValueType, DEFINITIONS};
use serde_json::{json, Value};
use std::sync::Arc;

/// Sees one's own attendance and clocks oneself (`/people/attendance/my`).
pub const SELF_VIEW: &str = "people.attendance.self.view";
/// Edits a company's rules, shift templates and clocking locations.
pub const RULES_MANAGE: &str = "people.attendance.rules.manage";
/// Plans and publishes rosters.
pub const ROSTER_MANAGE: &str = "people.attendance.roster.manage";
/// Decides adjustment requests.
pub const ADJUSTMENTS_APPROVE: &str = "people.attendance.adjustments.approve";
/// Edits a company's allowance rules.
pub const ALLOWANCES_MANAGE: &str = "people.attendance.allowances.manage";
/// Records a clock event from a device or integration through a reviewed agent operation.
pub const CLOCK_RECORD: &str = "people.attendance.clock.record";
/// Every Attendance capability.
pub const CAPABILITIES: [&str; 6] = [
    SELF_VIEW,
    RULES_MANAGE,
    ROSTER_MANAGE,
    ADJUSTMENTS_APPROVE,
    ALLOWANCES_MANAGE,
    CLOCK_RECORD,
];

/// The company's attendance time zone (IANA), default `Etc/UTC`.
pub const TIMEZONE_KEY: &str = "people.attendance.timezone";
/// Whether linked employee accounts may clock themselves, default off.
pub const SELF_CLOCK_KEY: &str = "people.attendance.self_clock_enabled";
/// Hours after clock-in that a shift stays open, 1 to 24, default 16.
pub const MAX_SHIFT_KEY: &str = "people.attendance.max_shift_hours";
/// Whether a clock event needs coordinates inside an active clocking
/// location, default off.
pub const LOCATION_REQUIRED_KEY: &str = "people.attendance.location_required";
/// Days, counting today, for which an employee may request an adjustment,
/// 1 to 366, default 7.
pub const ADJUSTMENT_WINDOW_KEY: &str = "people.attendance.adjustment_window_days";
/// The five rule settings.
pub const RULE_KEYS: [&str; 5] = [
    TIMEZONE_KEY,
    SELF_CLOCK_KEY,
    MAX_SHIFT_KEY,
    LOCATION_REQUIRED_KEY,
    ADJUSTMENT_WINDOW_KEY,
];

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
            "base/agent_api", "base/audit", "base/authz", "base/database", "base/datetime", "base/module_registry",
            "base/menu", "base/party_ids", "base/settings", "base/tenancy", "base/ui",
            "core/company", "people/settings", "people/workforce",
        ],
        migrations: &[(
            "people-attendance-001-core",
            include_str!("../migrations/001-core.sql"),
        ), ("people-attendance-002-rosters-adjustments", include_str!("../migrations/002-rosters-adjustments.sql")), ("people-attendance-003-allowance-rules", include_str!("../migrations/003-allowance-rules.sql"))],
        services: &[Service {
            id: SERVICE,
            factory: |services| {
                let attendance = Arc::new(Attendance {
                    db: services.get::<Db>("base/database.pool")?.as_ref().clone(),
                    mutations: services.get::<MutationBoundary>("base/database.mutations")?,
                    settings: services.get("base/settings.store")?,
                    workforce: services.get(bilibi_people_workforce::SERVICE)?,
                });
                crate::agent_operations::bind(attendance.clone())?;
                Ok(attendance)
            },
        }, Service {
            id: crate::web::PAGES,
            factory: |services| Ok(Arc::new(bilibi_ui::page::PageService::<bilibi_tenancy::Scope, bilibi_database::Error>(Arc::new(crate::web::Pages::new(
                services.get::<Db>("base/database.pool")?.as_ref().clone(),
                services.get(SERVICE)?,
            ))))),
        }, Service {
            id: crate::settings_web::PAGES,
            factory: |services| Ok(Arc::new(bilibi_ui::page::PageService::<bilibi_tenancy::Scope, bilibi_database::Error>(Arc::new(crate::settings_web::Pages::new(
                services.get::<Db>("base/database.pool")?.as_ref().clone(),
                services.get(SERVICE)?,
            ))))),
        }, Service {
            id: crate::web::my::PAGES,
            factory: |services| Ok(Arc::new(bilibi_ui::page::PageService::<bilibi_tenancy::Scope, bilibi_database::Error>(Arc::new(crate::web::my::Pages::new(
                services.get::<Db>("base/database.pool")?.as_ref().clone(),
                services.get(SERVICE)?,
            ))))),
        }],
    }
}

// Bilimbi grants none of these to a system role; an administrator grants
// them. `view` and `manage` are verbs Base declares; `approve` and `record`
// are not, and the vocabulary refuses an undeclared last segment.
inventory::submit! {
    Contribution::new(VOCABULARY, "people/attendance.vocabulary", provenance!(), &VocabularyContribution {
        domains: &["people"],
        verbs: &["approve", "record"],
        capabilities: &[
            C::tenant(SELF_VIEW), C::tenant(RULES_MANAGE), C::tenant(ROSTER_MANAGE),
            C::tenant(ADJUSTMENTS_APPROVE), C::tenant(ALLOWANCES_MANAGE), C::tenant(CLOCK_RECORD),
        ],
        roles: &[],
        system_principals: &[],
    })
}

/// A convertible IANA identifier in the bundled database.
fn timezone_valid(value: &Value) -> bool {
    value.as_str().is_some_and(bilibi_datetime::valid_timezone)
}

inventory::submit! { Contribution::new(DEFINITIONS, TIMEZONE_KEY, provenance!(), &Setting::Definition(Definition {
    value_type: ValueType::String, scopes: &[ScopeKind::Company], default: || json!("Etc/UTC"),
    nullable: false, encrypted: false, minimum: None, maximum: None,
    validator: Some(Validator { accepts: timezone_valid, message: "Give an IANA time zone name such as Asia/Kuala_Lumpur" }),
    editor: Some(Editor { group: "People", label: "Attendance time zone", help: "Local date used to group clock events for this company.", capability: Some(RULES_MANAGE), reveal_duration_ms: 5_000 }),
})) }

inventory::submit! { Contribution::new(DEFINITIONS, SELF_CLOCK_KEY, provenance!(), &Setting::Definition(Definition {
    value_type: ValueType::Boolean, scopes: &[ScopeKind::Company], default: || json!(false),
    nullable: false, encrypted: false, minimum: None, maximum: None, validator: None,
    editor: Some(Editor { group: "People", label: "Employee clocking", help: "Allow linked employee accounts to record their own clock events.", capability: Some(RULES_MANAGE), reveal_duration_ms: 5_000 }),
})) }

inventory::submit! { Contribution::new(DEFINITIONS, MAX_SHIFT_KEY, provenance!(), &Setting::Definition(Definition {
    value_type: ValueType::Integer, scopes: &[ScopeKind::Company], default: || json!(16),
    nullable: false, encrypted: false, minimum: Some(1.0), maximum: Some(24.0), validator: None,
    editor: Some(Editor { group: "People", label: "Maximum shift length", help: "Hours after clock-in that a shift stays open; a clock-out within it closes the shift on the clock-in day.", capability: Some(RULES_MANAGE), reveal_duration_ms: 5_000 }),
})) }

inventory::submit! { Contribution::new(DEFINITIONS, LOCATION_REQUIRED_KEY, provenance!(), &Setting::Definition(Definition {
    value_type: ValueType::Boolean, scopes: &[ScopeKind::Company], default: || json!(false),
    nullable: false, encrypted: false, minimum: None, maximum: None, validator: None,
    editor: Some(Editor { group: "People", label: "Require a clocking location", help: "Refuse clock events that do not carry coordinates inside an active clocking location. Approved adjustments are exempt.", capability: Some(RULES_MANAGE), reveal_duration_ms: 5_000 }),
})) }

inventory::submit! { Contribution::new(DEFINITIONS, ADJUSTMENT_WINDOW_KEY, provenance!(), &Setting::Definition(Definition {
    value_type: ValueType::Integer, scopes: &[ScopeKind::Company], default: || json!(7),
    nullable: false, encrypted: false, minimum: Some(1.0), maximum: Some(366.0), validator: None,
    editor: Some(Editor { group: "People", label: "Adjustment request window", help: "Days, counting today, for which an employee may request a missing clock event.", capability: Some(RULES_MANAGE), reveal_duration_ms: 5_000 }),
})) }

// PA5: Bilimbi Attendance's Team menu pages and their facade-owned writes.
inventory::submit! { Contribution::new(bilibi_module_registry::contributions::PAGE_WRITES, crate::web::PAGES, provenance!(), &bilibi_module_registry::contributions::PageWrites {
    service: crate::web::PAGES, contracts: &[],
    pending: bilibi_module_registry::contributions::Pending::Every { reason: "Roster planning, publication and adjustment decisions write through Attendance; their operation contracts have not been reviewed." },
}) }
inventory::submit! { Contribution::new(bilibi_module_registry::contributions::ROUTES, "people/attendance.rosters", provenance!(), &bilibi_module_registry::contributions::Route {
    method: bilibi_module_registry::contributions::Method::Get, path: "/people/attendance/rosters", service: crate::web::PAGES, operation: "rosters",
    access: bilibi_module_registry::contributions::Access::Capability(ROSTER_MANAGE), framed: true,
}) }
inventory::submit! { Contribution::new(bilibi_module_registry::contributions::ROUTES, "people/attendance.approvals", provenance!(), &bilibi_module_registry::contributions::Route {
    method: bilibi_module_registry::contributions::Method::Get, path: "/people/attendance/approvals", service: crate::web::PAGES, operation: "approvals",
    access: bilibi_module_registry::contributions::Access::Capability(ADJUSTMENTS_APPROVE), framed: true,
}) }
bilibi_menu::contribute!(
    "people/attendance",
    bilibi_menu::MenuItem::page(
        "people.attendance.rosters",
        "Rosters",
        "/people/attendance/rosters"
    )
    .icon("clipboard-document-list")
    .under("people.team")
    .requires(bilibi_menu::Requirement::Capability(ROSTER_MANAGE))
    .order(30)
);
bilibi_menu::contribute!(
    "people/attendance",
    bilibi_menu::MenuItem::page(
        "people.attendance.approvals",
        "Attendance approvals",
        "/people/attendance/approvals"
    )
    .icon("shield-check")
    .under("people.team")
    .requires(bilibi_menu::Requirement::Capability(ADJUSTMENTS_APPROVE))
    .order(40)
);

// PA4: Bilimbi Attendance.Web.MyLive and its My work menu leaf.
inventory::submit! {
    Contribution::new(bilibi_module_registry::contributions::ROUTES, "people/attendance.my", provenance!(), &bilibi_module_registry::contributions::Route {
        method: bilibi_module_registry::contributions::Method::Get,
        path: "/people/attendance/my", service: crate::web::my::PAGES, operation: "my",
        access: bilibi_module_registry::contributions::Access::Capability(SELF_VIEW), framed: true,
    })
}
inventory::submit! {
    Contribution::new(bilibi_module_registry::contributions::PAGE_WRITES, crate::web::my::PAGES, provenance!(), &bilibi_module_registry::contributions::PageWrites {
        service: crate::web::my::PAGES, contracts: &[],
        pending: bilibi_module_registry::contributions::Pending::Every { reason: "Self clocking and adjustment requests use the Attendance facade; no reviewed operation contracts exist for these self-service writes." },
    })
}
bilibi_menu::contribute!(
    "people/attendance",
    bilibi_menu::MenuItem::page(
        "people.attendance.my",
        "My attendance",
        "/people/attendance/my"
    )
    .icon("clipboard-document-list")
    .under("people.my_work")
    .requires(bilibi_menu::Requirement::Capability(SELF_VIEW))
    .order(10)
);

// PA3: Bilimbi Attendance.Web settings routes and Settings menu leaf.
macro_rules! settings_route {
    ($id:literal, $path:literal, $op:literal, $cap:expr) => {
        inventory::submit! {
            Contribution::new(bilibi_module_registry::contributions::ROUTES, $id, provenance!(), &bilibi_module_registry::contributions::Route {
                method: bilibi_module_registry::contributions::Method::Get,
                path: $path, service: crate::settings_web::PAGES, operation: $op,
                access: bilibi_module_registry::contributions::Access::Capability($cap), framed: true,
            })
        }
    };
}
settings_route!(
    "people/attendance.rules",
    "/people/attendance/rules",
    "rules",
    RULES_MANAGE
);
settings_route!(
    "people/attendance.shifts",
    "/people/attendance/rules/shifts",
    "shifts",
    RULES_MANAGE
);
settings_route!(
    "people/attendance.locations",
    "/people/attendance/rules/locations",
    "locations",
    RULES_MANAGE
);
settings_route!(
    "people/attendance.allowances",
    "/people/attendance/rules/allowances",
    "allowances",
    ALLOWANCES_MANAGE
);
inventory::submit! {
    Contribution::new(bilibi_module_registry::contributions::PAGE_WRITES, crate::settings_web::PAGES, provenance!(), &bilibi_module_registry::contributions::PageWrites {
        service: crate::settings_web::PAGES, contracts: &[],
        pending: bilibi_module_registry::contributions::Pending::Every { reason: "Rules, shifts, locations and allowances use Attendance's authorized mutation facade; reviewed contracts for these settings writes are not yet defined." },
    })
}
bilibi_menu::contribute!(
    "people/attendance",
    bilibi_menu::MenuItem::page(
        "people.attendance.rules",
        "Attendance rules",
        "/people/attendance/rules"
    )
    .icon("clipboard-document-list")
    .under("people.settings")
    .requires(bilibi_menu::Requirement::Capability(RULES_MANAGE))
    .order(10)
);
