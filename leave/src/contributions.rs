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
            "base/agent_api", "base/authz", "base/database", "base/module_registry", "base/party_ids",
            "base/settings", "base/tenancy", "base/datetime", "base/queue", "base/menu", "base/ui", "core/company",
            "people/settings", "people/workforce", "people/reference_data",
        ],
        migrations: &[
            ("people-leave-001-core", include_str!("../migrations/001-core.sql")),
            ("people-leave-002-applications", include_str!("../migrations/002-applications.sql")),
            ("people-leave-003-carry-forward-skips", include_str!("../migrations/003-carry-forward-skips.sql")),
        ],
        services: &[Service {
            id: SERVICE,
            factory: |services| {
                let facade = Arc::new(Leave {
                    db: services.get::<Db>("base/database.pool")?.as_ref().clone(),
                    mutations: services.get::<MutationBoundary>("base/database.mutations")?,
                    settings: services.get("base/settings.store")?,
                    workforce: services.get(bilibi_people_workforce::SERVICE)?,
                    reference_data: services.get(bilibi_people_reference_data::SERVICE)?,
                    queue: services.get("base/queue.store")?,
                });
                crate::agent::bind(facade.clone())?;
                Ok(facade)
            },
        }, Service {
            id: crate::web::PAGES,
            factory: |services| Ok(Arc::new(bilibi_ui::page::PageService::<bilibi_tenancy::Scope, bilibi_database::Error>(Arc::new(crate::web::Pages::new(
                services.get::<Db>("base/database.pool")?.as_ref().clone(),
                services.get(SERVICE)?,
            ))))),
        }, Service {
            id: crate::policy_web::SERVICE,
            factory: |services| Ok(Arc::new(bilibi_ui::page::PageService::<bilibi_tenancy::Scope, bilibi_database::Error>(
                Arc::new(crate::policy_web::PoliciesPage::new(services.get("base/database.pool")?, services.get(SERVICE)?)),
            ))),
        }, Service {
            id: crate::worker::SERVICE,
            factory: |services| Ok(Arc::new(bilibi_queue::HandlerService(Arc::new(crate::worker::CarryForwardWorker(services.get(SERVICE)?))))),
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

// Bilimbi CarryForwardWorker: no schedule, five attempts as the operator.
inventory::submit! { Contribution::new(bilibi_queue::WORKERS, crate::worker::WORKER, provenance!(), &bilibi_queue::Worker { max_attempts: 5, validate: crate::worker::validate }) }
inventory::submit! { Contribution::new(bilibi_queue::HANDLERS, crate::worker::WORKER, provenance!(), &bilibi_queue::HandlerBinding { service: crate::worker::SERVICE, capability: POLICIES_MANAGE }) }

inventory::submit! { Contribution::new(bilibi_module_registry::contributions::PAGE_WRITES, crate::web::PAGES, provenance!(), &bilibi_module_registry::contributions::PageWrites {
    service: crate::web::PAGES, contracts: &[],
    pending: bilibi_module_registry::contributions::Pending::Every { reason: "Leave requests, cancellations and decisions write through the Leave facade; their operation contracts have not been reviewed." },
}) }

inventory::submit! { Contribution::new(bilibi_module_registry::contributions::ROUTES, "people/leave.my", provenance!(), &bilibi_module_registry::contributions::Route { company_scope: bilibi_module_registry::contributions::CompanyScope::Company,
    method: bilibi_module_registry::contributions::Method::Get, path: "/people/leave/my", service: crate::web::PAGES, operation: "my",
    access: bilibi_module_registry::contributions::Access::Capability(SELF_VIEW), framed: true,
}) }
inventory::submit! { Contribution::new(bilibi_module_registry::contributions::ROUTES, "people/leave.requests", provenance!(), &bilibi_module_registry::contributions::Route { company_scope: bilibi_module_registry::contributions::CompanyScope::Tenant,
    method: bilibi_module_registry::contributions::Method::Get, path: "/people/leave/requests", service: crate::web::PAGES, operation: "requests",
    access: bilibi_module_registry::contributions::Access::Capability(REQUESTS_APPROVE), framed: true,
}) }

bilibi_menu::contribute!(
    "people/leave",
    bilibi_menu::MenuItem::page("people.leave.my", "My leave", "/people/leave/my")
        .company_scoped()
        .icon("clipboard-document-list")
        .under("people.my_work")
        .requires(bilibi_menu::Requirement::Capability(SELF_VIEW))
        .order(20)
);
bilibi_menu::contribute!(
    "people/leave",
    bilibi_menu::MenuItem::page(
        "people.leave.approvals",
        "Leave approvals",
        "/people/leave/requests"
    )
    .icon("shield-check")
    .under("people.team")
    .requires(bilibi_menu::Requirement::Capability(REQUESTS_APPROVE))
    .order(50)
);

// Bilimbi Leave.Web.PoliciesLive and priv/web_routes.exs.
inventory::submit! { Contribution::new(
    bilibi_module_registry::contributions::ROUTES, "people/leave.policies", provenance!(),
    &bilibi_module_registry::contributions::Route { company_scope: bilibi_module_registry::contributions::CompanyScope::Tenant,
        method: bilibi_module_registry::contributions::Method::Get,
        path: "/people/leave/policies", service: crate::policy_web::SERVICE, operation: "policies",
        access: bilibi_module_registry::contributions::Access::Capability(POLICIES_MANAGE), framed: true,
    }
) }
inventory::submit! { Contribution::new(
    bilibi_module_registry::contributions::PAGE_WRITES, crate::policy_web::SERVICE, provenance!(),
    &bilibi_module_registry::contributions::PageWrites {
        service: crate::policy_web::SERVICE, contracts: &[],
        pending: bilibi_module_registry::contributions::Pending::Every {
            reason: "Leave policy forms write through the Leave facade; policy operation contracts have not been reviewed.",
        },
    }
) }
bilibi_menu::contribute!(
    "people/leave",
    bilibi_menu::MenuItem::page(
        "people.leave.policies",
        "Leave policies",
        "/people/leave/policies"
    )
    .icon("clipboard-document-list")
    .under("people.settings")
    .requires(bilibi_menu::Requirement::Capability(POLICIES_MANAGE))
    .order(20)
);
