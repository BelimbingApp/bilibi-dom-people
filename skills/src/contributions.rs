//! What Skills adds to the shared catalogs (Bilimbi
//! `Bilimbi.People.Skills.Contributions`): the module descriptor with its
//! migration and facade service, the seventeen `people.skills.*`
//! capabilities under the `people` Authz domain with the verbs Base does not
//! declare, and the seven company-scoped policy settings. The menu leaves
//! land with their pages (PK4–PK6).
use crate::{policy, Skills, MODULE, SERVICE};
use bilibi_authz::{CapabilityDefinition as C, VocabularyContribution, VOCABULARY};
use bilibi_database::{Db, MutationBoundary};
use bilibi_module_registry::{Contribution, Layer, Module, Provenance, Service};
use bilibi_settings::{Definition, Editor, ScopeKind, Setting, ValueType, DEFINITIONS};
use serde_json::json;
use std::sync::Arc;

/// Opens `/people/skills` and each profile page.
pub const CATALOG_VIEW: &str = "people.skills.catalog.view";
/// Edits the catalog, scales, profile drafts and action types.
pub const CATALOG_MANAGE: &str = "people.skills.catalog.manage";
/// Publishes and retires requirement profiles.
pub const PROFILES_PUBLISH: &str = "people.skills.profiles.publish";
pub const ASSESSMENTS_VIEW: &str = "people.skills.assessments.view";
pub const ASSESSMENTS_SUBMIT: &str = "people.skills.assessments.submit";
pub const ASSESSMENTS_REVIEW: &str = "people.skills.assessments.review";
pub const ASSESSMENTS_APPROVE: &str = "people.skills.assessments.approve";
pub const ASSESSMENTS_MANAGE: &str = "people.skills.assessments.manage";
pub const REASSESSMENTS_SUBMIT: &str = "people.skills.reassessments.submit";
pub const REASSESSMENTS_EXECUTE: &str = "people.skills.reassessments.execute";
pub const ACTIONS_VIEW: &str = "people.skills.actions.view";
pub const ACTIONS_UPDATE: &str = "people.skills.actions.update";
pub const ACTIONS_MANAGE: &str = "people.skills.actions.manage";
pub const ACTIONS_APPROVE: &str = "people.skills.actions.approve";
pub const SELF_VIEW: &str = "people.skills.self.view";
pub const REMINDERS_SEND: &str = "people.skills.reminders.send";
/// Edits the seven policy settings and opens the Skills policy page.
pub const POLICY_MANAGE: &str = "people.skills.policy.manage";
/// Every Skills capability, in Bilimbi's order. None is granted to a system
/// role: an administrator grants them, split across roles where duties
/// need separate people.
pub const CAPABILITIES: [&str; 17] = [
    CATALOG_VIEW,
    CATALOG_MANAGE,
    PROFILES_PUBLISH,
    ASSESSMENTS_VIEW,
    ASSESSMENTS_SUBMIT,
    ASSESSMENTS_REVIEW,
    ASSESSMENTS_APPROVE,
    ASSESSMENTS_MANAGE,
    REASSESSMENTS_SUBMIT,
    REASSESSMENTS_EXECUTE,
    ACTIONS_VIEW,
    ACTIONS_UPDATE,
    ACTIONS_MANAGE,
    ACTIONS_APPROVE,
    SELF_VIEW,
    REMINDERS_SEND,
    POLICY_MANAGE,
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
            "base/authz", "base/database", "base/module_registry", "base/party_ids",
            "base/settings", "base/tenancy", "base/ui", "base/menu", "base/system", "base/notifications", "base/queue", "core/user", "core/company",
            "people/settings", "people/workforce",
        ],
        migrations: &[(
            "people-skills-001-catalog",
            include_str!("../migrations/001-catalog.sql"),
        ), (
            "people-skills-002-assessments",
            include_str!("../migrations/002-assessments.sql"),
        )],
        // Factories resolve earlier services in this module; install the facade first.
        services: &[Service {
            id: SERVICE,
            factory: |services| {
                Ok(Arc::new(Skills {
                    db: services.get::<Db>("base/database.pool")?.as_ref().clone(),
                    mutations: services.get::<MutationBoundary>("base/database.mutations")?,
                    settings: services.get("base/settings.store")?,
                    workforce: services.get(bilibi_people_workforce::SERVICE)?,
                    notifications: services.get("base/notifications.sink")?,
                    queue: services.get("base/queue.store")?,
                }))
            },
        }, Service {
            id: crate::pk4::PAGES,
            factory: |services| Ok(Arc::new(bilibi_ui::page::PageService(Arc::new(crate::pk4::Pages::new(
                services.get::<Db>("base/database.pool")?.as_ref().clone(), services.get(SERVICE)?, services.get(bilibi_system::SERVICE)?,
            ))))),
        }, Service {
            id: crate::pk5::PAGES,
            factory: |services| Ok(Arc::new(bilibi_ui::page::PageService(Arc::new(crate::pk5::Pages::new(services.get::<Db>("base/database.pool")?.as_ref().clone(), services.get(SERVICE)?, services.get(bilibi_system::SERVICE)?))))),

        }, Service {
            id: crate::pk6::PAGES,
            factory: |services| Ok(Arc::new(bilibi_ui::page::PageService(Arc::new(crate::pk6::Pages::new(services.get::<Db>("base/database.pool")?.as_ref().clone(), services.get(SERVICE)?, services.get(bilibi_system::SERVICE)?))))),
        }, Service {
            id: crate::reminder_worker::SERVICE,
            factory: |services| Ok(Arc::new(bilibi_queue::HandlerService(Arc::new(crate::reminder_worker::ReminderWorker(services.get(SERVICE)?))))),
        }],
    }
}

// The Authz domain is `people`, as Bilimbi declares it; the vocabulary refuses
// a key whose last segment no module declared, so the verbs Base lacks
// (`submit`, `review`, `approve`, `send`) are declared here.
inventory::submit! {
    Contribution::new(VOCABULARY, "people/skills.vocabulary", provenance!(), &VocabularyContribution {
        domains: &["people"],
        verbs: &["submit", "review", "approve", "send"],
        capabilities: &[
            C::tenant(CATALOG_VIEW), C::tenant(CATALOG_MANAGE), C::tenant(PROFILES_PUBLISH),
            C::tenant(ASSESSMENTS_VIEW), C::tenant(ASSESSMENTS_SUBMIT), C::tenant(ASSESSMENTS_REVIEW),
            C::tenant(ASSESSMENTS_APPROVE), C::tenant(ASSESSMENTS_MANAGE),
            C::tenant(REASSESSMENTS_SUBMIT), C::tenant(REASSESSMENTS_EXECUTE),
            C::tenant(ACTIONS_VIEW), C::tenant(ACTIONS_UPDATE), C::tenant(ACTIONS_MANAGE), C::tenant(ACTIONS_APPROVE),
            C::tenant(SELF_VIEW), C::tenant(REMINDERS_SEND), C::tenant(POLICY_MANAGE),
        ],
        roles: &[],
        system_principals: &[],
    })
}

// The seven policy values: company-scoped integers with Bilimbi's defaults,
// ranges, labels and help, edited by holders of `people.skills.policy.manage`.
macro_rules! policy_setting {
    ($key:expr) => {
        inventory::submit! { Contribution::new(DEFINITIONS, $key.key, provenance!(), &Setting::Definition(Definition {
            value_type: ValueType::Integer, scopes: &[ScopeKind::Company], default: || json!($key.default),
            nullable: false, encrypted: false, minimum: Some($key.min as f64), maximum: Some($key.max as f64),
            validator: None,
            editor: Some(Editor { group: "People", label: $key.label, help: $key.help, capability: Some(POLICY_MANAGE), reveal_duration_ms: 5_000 }),
        })) }
    };
}
policy_setting!(policy::REASSESSMENT_DUE_DAYS);
policy_setting!(policy::DEFAULT_REASSESSMENT_MONTHS);
policy_setting!(policy::REMINDER_WINDOW_DAYS);
policy_setting!(policy::BACKUP_MINIMUM);
policy_setting!(policy::MULTIPLIER_CRITICAL);
policy_setting!(policy::MULTIPLIER_ESSENTIAL);
policy_setting!(policy::MULTIPLIER_DEVELOPMENT);

// Bilimbi Skills.ReminderWorker: five attempts as the enqueuing operator.
inventory::submit! { Contribution::new(bilibi_queue::WORKERS, crate::reminder_worker::WORKER, provenance!(), &bilibi_queue::Worker { max_attempts: 5, validate: crate::reminder_worker::validate }) }
inventory::submit! { Contribution::new(bilibi_queue::HANDLERS, crate::reminder_worker::WORKER, provenance!(), &bilibi_queue::HandlerBinding { service: crate::reminder_worker::SERVICE, capability: REMINDERS_SEND }) }

// PK6 mirrors Skills.Web.ActionsLive and Skills.Web.PolicyLive.
inventory::submit! { Contribution::new(bilibi_module_registry::contributions::PAGE_WRITES, crate::pk6::PAGES, provenance!(), &bilibi_module_registry::contributions::PageWrites {
    service: crate::pk6::PAGES, contracts: &[],
    pending: bilibi_module_registry::contributions::Pending::Every { reason: "Skills action, policy, reassessment and reminder writes call the Skills facade; operation contracts await review." },
}) }
macro_rules! pk6_page {
    ($id:literal, $path:literal, $operation:literal, $cap:expr) => {
        inventory::submit! { Contribution::new(bilibi_module_registry::contributions::ROUTES, $id, provenance!(), &bilibi_module_registry::contributions::Route { company_scope: bilibi_module_registry::contributions::CompanyScope::Tenant,
            method: bilibi_module_registry::contributions::Method::Get, path: $path,
            service: crate::pk6::PAGES, operation: $operation,
            access: bilibi_module_registry::contributions::Access::Capability($cap), framed: true,
        }) }
    };
}
pk6_page!(
    "people/skills.actions",
    "/people/skills/actions",
    "actions",
    ACTIONS_VIEW
);
pk6_page!(
    "people/skills.policy",
    "/people/skills/policy",
    "policy",
    POLICY_MANAGE
);
bilibi_menu::contribute!(
    "people/skills",
    bilibi_menu::MenuItem::page(
        "people.development.actions",
        "Development actions",
        "/people/skills/actions"
    )
    .icon("flag")
    .under("people.development")
    .requires(bilibi_menu::Requirement::Capability(ACTIONS_VIEW))
    .order(30)
);
bilibi_menu::contribute!(
    "people/skills",
    bilibi_menu::MenuItem::page(
        "people.settings.skills_policy",
        "Skills policy",
        "/people/skills/policy"
    )
    .icon("cog-6-tooth")
    .under("people.settings")
    .requires(bilibi_menu::Requirement::Capability(POLICY_MANAGE))
    .order(40)
);

// PK4 mirrors Skills.Web.CatalogLive and Skills.Web.ProfileLive.
inventory::submit! { Contribution::new(bilibi_module_registry::contributions::PAGE_WRITES, crate::pk4::PAGES, provenance!(), &bilibi_module_registry::contributions::PageWrites {
    service: crate::pk4::PAGES, contracts: &[],
    pending: bilibi_module_registry::contributions::Pending::Every { reason: "Skills catalog and profile writes call the authorized Skills facade; operation contracts await review." },
}) }
macro_rules! pk4_page {
    ($id:literal, $path:literal, $operation:literal) => {
        inventory::submit! { Contribution::new(bilibi_module_registry::contributions::ROUTES, $id, provenance!(), &bilibi_module_registry::contributions::Route { company_scope: bilibi_module_registry::contributions::CompanyScope::Tenant,
            method: bilibi_module_registry::contributions::Method::Get, path: $path,
            service: crate::pk4::PAGES, operation: $operation,
            access: bilibi_module_registry::contributions::Access::Capability(CATALOG_VIEW), framed: true,
        }) }
    };
}
pk4_page!("people/skills.catalog", "/people/skills", "catalog");
pk4_page!(
    "people/skills.profile",
    "/people/skills/profiles/{id}",
    "profile"
);
bilibi_menu::contribute!(
    "people/skills",
    bilibi_menu::MenuItem::page("people.development.skills", "Skills", "/people/skills")
        .icon("puzzle-piece")
        .under("people.development")
        .requires(bilibi_menu::Requirement::Capability(CATALOG_VIEW))
        .order(10)
);

// PK5 mirrors Skills.Web.AssessmentsLive and Skills.Web.MySkillsLive.
inventory::submit! { Contribution::new(bilibi_module_registry::contributions::PAGE_WRITES, crate::pk5::PAGES, provenance!(), &bilibi_module_registry::contributions::PageWrites {
    service: crate::pk5::PAGES, contracts: &[],
    pending: bilibi_module_registry::contributions::Pending::Every { reason: "Skills assessment and reassessment writes call the Skills facade; operation contracts await review." },
}) }
inventory::submit! { Contribution::new(bilibi_module_registry::contributions::ROUTES, "people/skills.assessments", provenance!(), &bilibi_module_registry::contributions::Route { company_scope: bilibi_module_registry::contributions::CompanyScope::Tenant,
    method: bilibi_module_registry::contributions::Method::Get, path: "/people/skills/assessments",
    service: crate::pk5::PAGES, operation: "assessments",
    access: bilibi_module_registry::contributions::Access::Capability(ASSESSMENTS_VIEW), framed: true,
}) }
inventory::submit! { Contribution::new(bilibi_module_registry::contributions::ROUTES, "people/skills.my", provenance!(), &bilibi_module_registry::contributions::Route { company_scope: bilibi_module_registry::contributions::CompanyScope::Tenant,
    method: bilibi_module_registry::contributions::Method::Get, path: "/people/skills/my",
    service: crate::pk5::PAGES, operation: "my",
    access: bilibi_module_registry::contributions::Access::Capability(SELF_VIEW), framed: true,
}) }
bilibi_menu::contribute!(
    "people/skills",
    bilibi_menu::MenuItem::page(
        "people.development.assessments",
        "Assessments",
        "/people/skills/assessments"
    )
    .icon("clipboard-document-list")
    .under("people.development")
    .requires(bilibi_menu::Requirement::Capability(ASSESSMENTS_VIEW))
    .order(20)
);
bilibi_menu::contribute!(
    "people/skills",
    bilibi_menu::MenuItem::page("people.my_work.skills", "My skills", "/people/skills/my")
        .icon("puzzle-piece")
        .under("people.my_work")
        .requires(bilibi_menu::Requirement::Capability(SELF_VIEW))
        .order(40)
);
