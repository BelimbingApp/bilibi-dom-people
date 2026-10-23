//! What Workforce adds to the shared catalogs (Bilimbi
//! `Bilimbi.People.Workforce.Contributions`): the module descriptor with its
//! migration and facade service, the `people` Authz domain with the three
//! capabilities and the `tenant_owner` grants, and the
//! `people.workforce.working_statuses` company setting.
use crate::{Workforce, EMPLOYEE_STATUSES, MODULE, SERVICE};
use bilibi_authz::{CapabilityDefinition as C, RoleDefinition, VocabularyContribution, VOCABULARY};
use bilibi_database::{Db, MutationBoundary};
use bilibi_module_registry::{
    contributions::{Access, Method, PageWrites, Pending, Route, PAGE_WRITES, ROUTES},
    Contribution, Layer, Module, Provenance, Service,
};
use bilibi_settings::{Definition, Editor, ScopeKind, Setting, Validator, ValueType, DEFINITIONS};
use bilibi_tenancy::Scope;
use bilibi_ui::page::PageService;
use serde_json::{json, Value};
use std::sync::Arc;

/// Edits a company's working statuses (`/people/workforce/settings`).
pub const SETTINGS_MANAGE: &str = "people.workforce.settings.manage";
/// Reads an employee's private employment facts, at any status.
pub const EMPLOYMENT_VIEW: &str = "people.workforce.employment.view";
/// Creates and changes employment records.
pub const EMPLOYMENT_MANAGE: &str = "people.workforce.employment.manage";
/// Every Workforce capability.
pub const CAPABILITIES: [&str; 3] = [SETTINGS_MANAGE, EMPLOYMENT_VIEW, EMPLOYMENT_MANAGE];
/// What Bilimbi grants the Tenant Owner role: the two employment keys; the
/// settings key is granted by an administrator.
const TENANT_OWNER: [&str; 2] = [EMPLOYMENT_VIEW, EMPLOYMENT_MANAGE];

/// The company-scoped setting naming which employment statuses count as
/// working staff; default `probation` and `active`.
pub const WORKING_STATUSES_KEY: &str = "people.workforce.working_statuses";

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
            "base/ports", "base/principal_directory", "base/settings", "base/tenancy", "base/ui",
            "core/affiliation", "core/company", "core/person",
        ],
        migrations: &[(
            "people-workforce-001-employments",
            include_str!("../migrations/001-employments.sql"),
        )],
        services: &[Service {
            id: SERVICE,
            factory: |services| {
                let facade = Arc::new(Workforce {
                    db: services.get::<Db>("base/database.pool")?.as_ref().clone(),
                    mutations: services.get::<MutationBoundary>("base/database.mutations")?,
                    settings: services.get("base/settings.store")?,
                    affiliations: services.get(bilibi_affiliation::SERVICE)?,
                    directory: services.get("base/principal_directory.directory")?,
                    authentication: services.get("base/tenancy.authentication")?,
                    ports: services.get(bilibi_ports::SERVICE)?,
                });
                crate::directory_port::install(facade.clone());
                Ok(facade)
            },
        }, Service {
            id: PAGES,
            factory: |services| Ok(Arc::new(PageService::<Scope, bilibi_database::Error>(Arc::new(crate::web::Pages::new(
                services.get::<Db>("base/database.pool")?.as_ref().clone(),
                services.get(SERVICE)?,
            ))))),
        }],
    }
}

// The Authz domain is `people`, as Bilimbi declares it ("People domain
// modules"); every People module's keys live under it. `view` and `manage`
// are verbs Base already declares.
inventory::submit! {
    Contribution::new(VOCABULARY, "people/workforce.vocabulary", provenance!(), &VocabularyContribution {
        domains: &["people"],
        verbs: &[],
        capabilities: &[C::tenant(SETTINGS_MANAGE), C::tenant(EMPLOYMENT_VIEW), C::tenant(EMPLOYMENT_MANAGE)],
        roles: &[RoleDefinition {
            code: "tenant_owner",
            name: "Tenant Owner",
            description: "Contributed system role; reconciled explicitly",
            grant_all: false,
            capabilities: &TENANT_OWNER,
        }],
        system_principals: &[],
    })
}

/// A non-empty list of distinct employment statuses, each one of the five.
pub(crate) fn working_statuses_valid(value: &Value) -> bool {
    value.as_array().is_some_and(|statuses| {
        !statuses.is_empty()
            && statuses.iter().enumerate().all(|(index, status)| {
                status
                    .as_str()
                    .is_some_and(|status| EMPLOYEE_STATUSES.contains(&status))
                    && !statuses[..index].contains(status)
            })
    })
}

inventory::submit! { Contribution::new(DEFINITIONS, WORKING_STATUSES_KEY, provenance!(), &Setting::Definition(Definition {
    value_type: ValueType::Array, scopes: &[ScopeKind::Company], default: || json!(["probation", "active"]),
    nullable: false, encrypted: false, minimum: None, maximum: None,
    validator: Some(Validator { accepts: working_statuses_valid, message: "Give one or more of pending, probation, active, inactive and terminated, each once" }),
    editor: Some(Editor { group: "People", label: "Working employee statuses", help: "Employee statuses People treats as working staff for this company.", capability: Some(SETTINGS_MANAGE), reveal_duration_ms: 5_000 }),
})) }

// Bilimbi Workforce.Web.SettingsLive; this page has no menu leaf.
const PAGES: &str = "people/workforce.pages";
inventory::submit! {
    Contribution::new(ROUTES, "people/workforce.settings", provenance!(), &Route {
        method: Method::Get, path: "/people/workforce/settings", service: PAGES, operation: "settings",
        access: Access::Capability(SETTINGS_MANAGE), framed: true,
    })
}
inventory::submit! {
    Contribution::new(PAGE_WRITES, PAGES, provenance!(), &PageWrites {
        service: PAGES, contracts: &[],
        pending: Pending::Every { reason: "Working statuses write through the Workforce facade; no operation contract has been reviewed for this setting." },
    })
}
inventory::submit! {
    Contribution::new(bilibi_ports::PORTS, crate::DIRECTORY_PORT, provenance!(), &bilibi_ports::Port {
        provider: &crate::directory_port::DirectoryPort,
    })
}
