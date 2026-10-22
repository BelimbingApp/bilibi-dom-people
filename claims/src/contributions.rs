//! What Claims adds to the shared catalogs (Bilimbi
//! `Bilimbi.People.Claims.Contributions`): the module descriptor with its three
//! migrations and the facade service, the four `people.claims.*`
//! capabilities with the verb `reimburse` and the `tenant_owner` grants, and
//! the `people.claims.currencies` company setting. The three menu leaves
//! (My claims, Claim operations, Claim policies) land with their pages.
use crate::{Claims, MAX_CURRENCIES, MODULE, SERVICE};
use bilibi_authz::{CapabilityDefinition as C, RoleDefinition, VocabularyContribution, VOCABULARY};
use bilibi_database::{Db, MutationBoundary};
use bilibi_module_registry::{Contribution, Layer, Module, Provenance, Service};
use bilibi_settings::{Definition, Editor, ScopeKind, Setting, Validator, ValueType, DEFINITIONS};
use serde_json::{json, Value};
use std::sync::Arc;

/// Submits and withdraws one's own claims (`/people/claims`).
pub const SUBMIT: &str = "people.claims.submit";
/// Edits a company's currencies, catalog, policies and assignments
/// (`/people/claims/setup`).
pub const MANAGE: &str = "people.claims.manage";
/// Decides claims and reads the operator queue (`/people/claims/operations`).
pub const APPROVE: &str = "people.claims.approve";
/// Pays claims, hands them off and exports a batch.
pub const REIMBURSE: &str = "people.claims.reimburse";
/// Every Claims capability; Bilimbi grants all four to the Tenant Owner role.
pub const CAPABILITIES: [&str; 4] = [SUBMIT, MANAGE, APPROVE, REIMBURSE];

/// The company-scoped setting listing the ISO 4217 codes the company accepts
/// on claims; default none.
pub const CURRENCIES_KEY: &str = "people.claims.currencies";

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
            "base/authz", "base/menu", "base/database", "base/datetime", "base/events", "base/ui", "base/module_registry",
            "base/party_ids", "base/settings", "base/system", "base/tenancy", "core/company",
            bilibi_people_settings::MODULE, bilibi_people_workforce::MODULE,
        ],
        migrations: &[
            (
                "people-claims-001-catalog-policies-requests",
                include_str!("../migrations/001-catalog-policies-requests.sql"),
            ),
            (
                "people-claims-002-assignments-handoff",
                include_str!("../migrations/002-assignments-handoff.sql"),
            ),
            (
                "people-claims-003-workflow-history",
                include_str!("../migrations/003-workflow-history.sql"),
            ),
        ],
        services: &[Service {
            id: SERVICE,
            factory: |services| {
                Ok(Arc::new(Claims {
                    db: services.get::<Db>("base/database.pool")?.as_ref().clone(),
                    mutations: services.get::<MutationBoundary>("base/database.mutations")?,
                    settings: services.get("base/settings.store")?,
                    events: services.get(bilibi_events::SERVICE)?,
                    workforce: services.get(bilibi_people_workforce::SERVICE)?,
                }))
            },
        }, crate::downloads::SERVICE_DEFINITION, Service {
            id: "people/claims.operations_pages",
            factory: |services| Ok(Arc::new(bilibi_ui::page::PageService::<bilibi_tenancy::Scope, bilibi_database::Error>(Arc::new(crate::web::Pages::new(
                services.get::<Db>("base/database.pool")?.as_ref().clone(), services.get(SERVICE)?,
            ))))),
        }],
    }
}

// The Authz domain is `people`, shared by every People module. The
// vocabulary refuses a key whose last segment no module declared, and Base
// declares neither `submit`, `approve` nor `reimburse` (`approve` comes from
// Base Workflow, which this module does not depend on), so all three are
// declared here; verbs are a set, so a repeat is harmless.
inventory::submit! {
    Contribution::new(VOCABULARY, "people/claims.vocabulary", provenance!(), &VocabularyContribution {
        domains: &["people"],
        verbs: &["submit", "approve", "reimburse"],
        capabilities: &[C::tenant(SUBMIT), C::tenant(MANAGE), C::tenant(APPROVE), C::tenant(REIMBURSE)],
        roles: &[RoleDefinition {
            code: "tenant_owner",
            name: "Tenant Owner",
            description: "Contributed system role; reconciled explicitly",
            grant_all: false,
            capabilities: &CAPABILITIES,
        }],
        system_principals: &[],
    })
}

/// At most twenty distinct codes of three uppercase ASCII letters.
pub(crate) fn currencies_valid(value: &Value) -> bool {
    value.as_array().is_some_and(|codes| {
        codes.len() <= MAX_CURRENCIES
            && codes.iter().enumerate().all(|(index, code)| {
                code.as_str().is_some_and(crate::currency_shaped) && !codes[..index].contains(code)
            })
    })
}

inventory::submit! { Contribution::new(DEFINITIONS, CURRENCIES_KEY, provenance!(), &Setting::Definition(Definition {
    value_type: ValueType::Array, scopes: &[ScopeKind::Company], default: || json!([]),
    nullable: false, encrypted: false, minimum: None, maximum: None,
    validator: Some(Validator { accepts: currencies_valid, message: "Give up to twenty ISO 4217 codes of three uppercase letters, each once" }),
    editor: Some(Editor { group: "People", label: "Claim currencies", help: "ISO 4217 currency codes this company accepts on claims.", capability: Some(MANAGE), reveal_duration_ms: 5_000 }),
})) }

// Facts for independently composed subscribers, committed with each decision.
inventory::submit! { Contribution::new(bilibi_events::EVENTS, crate::REQUEST_APPROVED, provenance!(), &bilibi_events::EventType { version: 1 }) }
inventory::submit! { Contribution::new(bilibi_events::EVENTS, crate::REQUEST_REIMBURSED, provenance!(), &bilibi_events::EventType { version: 1 }) }

// Bilimbi Claims.Web.OperationsLive and its Time and expenses menu leaf.
inventory::submit! { Contribution::new(bilibi_module_registry::contributions::ROUTES, "people/claims.operations", provenance!(), &bilibi_module_registry::contributions::Route {
    method: bilibi_module_registry::contributions::Method::Get,
    path: "/people/claims/operations", service: "people/claims.operations_pages", operation: "operations",
    access: bilibi_module_registry::contributions::Access::Capability(APPROVE), framed: true,
}) }
inventory::submit! { Contribution::new(bilibi_module_registry::contributions::PAGE_WRITES, "people/claims.operations_pages", provenance!(), &bilibi_module_registry::contributions::PageWrites {
    service: "people/claims.operations_pages", contracts: &[], pending: bilibi_module_registry::contributions::Pending::Every {
        reason: "Claim operations use the authorized Claims facade; decision and reimbursement contracts are not yet reviewed.",
    },
}) }
bilibi_menu::contribute!(
    "people/claims",
    bilibi_menu::MenuItem::page(
        "people.time_and_expenses.claim_operations",
        "Claim operations",
        "/people/claims/operations"
    )
    .under("people.time_and_expenses")
    .requires(bilibi_menu::Requirement::Capability(APPROVE))
    .order(30)
);
