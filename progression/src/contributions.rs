//! Bilimbi Progression.Contributions: policies and separately authorized standing.
use crate::{Progression, MODULE, SERVICE};
use bilibi_authz::{CapabilityDefinition as C, VocabularyContribution, VOCABULARY};
use bilibi_database::{Db, MutationBoundary};
use bilibi_menu::{MenuItem, Requirement};
use bilibi_module_registry::contributions::{
    Access, Method, PageWrites, Pending, Route, PAGE_WRITES, ROUTES,
};
use bilibi_tenancy::Scope;
use bilibi_ui::page::PageService;
const PAGES: &str = "people/progression.pages";
use bilibi_module_registry::{Contribution, Layer, Module, Provenance, Service};
use std::sync::Arc;

pub const POLICY_VIEW: &str = "people.progression.policy.view";
pub const POLICY_MANAGE: &str = "people.progression.policy.manage";
pub const SELF_VIEW: &str = "people.progression.self.view";
pub const CAPABILITIES: [&str; 3] = [POLICY_VIEW, POLICY_MANAGE, SELF_VIEW];

inventory::submit! { Module {
    id: MODULE, layer: Layer::Domains,
    dependencies: &["base/authz", "base/database", "base/datetime", "base/menu", "base/module_registry", "base/tenancy", "base/ui", "core/company", "people/performance", "people/settings", "people/skills", "people/workforce"],
    migrations: &[("people-progression-001-progression", include_str!("../migrations/001-progression.sql"))],
    services: &[Service {id: SERVICE, factory: |services| Ok(Arc::new(Progression {
        db: services.get::<Db>("base/database.pool")?.as_ref().clone(),
        mutations: services.get::<MutationBoundary>("base/database.mutations")?,
        workforce: services.get(bilibi_people_workforce::SERVICE)?,
        performance: services.get(bilibi_people_performance::SERVICE)?,
        skills: services.get(bilibi_people_skills::SERVICE)?,
    }))}, Service {id: PAGES, factory: |services| Ok(Arc::new(PageService::<Scope, bilibi_database::Error>(Arc::new(crate::web::Pages::new(
        services.get(SERVICE)?, services.get(bilibi_people_skills::SERVICE)?, services.get("base/datetime.display")?,
    )))))}],
}}
inventory::submit! { Contribution::new(VOCABULARY, "people/progression.vocabulary", Provenance {module: MODULE, source: file!(), line: line!()}, &VocabularyContribution {
    domains: &["people"], verbs: &[],
    capabilities: &[C::tenant(POLICY_VIEW), C::tenant(POLICY_MANAGE), C::tenant(SELF_VIEW)],
    roles: &[], system_principals: &[],
})}

macro_rules! provenance {
    () => {
        Provenance {
            module: MODULE,
            source: file!(),
            line: line!(),
        }
    };
}
inventory::submit! { Contribution::new(ROUTES, "people/progression.policies", provenance!(), &Route { company_scope: bilibi_module_registry::contributions::CompanyScope::Company,
    method: Method::Get, path: "/people/progression", service: PAGES, operation: "policies",
    access: Access::Capability(POLICY_VIEW), framed: true,
}) }
inventory::submit! { Contribution::new(ROUTES, "people/progression.my", provenance!(), &Route { company_scope: bilibi_module_registry::contributions::CompanyScope::Company,
    method: Method::Get, path: "/people/progression/my", service: PAGES, operation: "my",
    access: Access::AnyOf(&[SELF_VIEW, bilibi_people_performance::SELF_VIEW]), framed: true,
}) }
inventory::submit! { Contribution::new(PAGE_WRITES, PAGES, provenance!(), &PageWrites {
    service: PAGES, contracts: &[], pending: Pending::Every {
        reason: "Progression policy writes use the authorized facade; policy contracts await review.",
    },
}) }
bilibi_menu::contribute!(
    "people/progression",
    MenuItem::page(
        "people.development.progression",
        "Progression",
        "/people/progression"
    )
    .company_scoped()
    .under("people.development")
    .requires(Requirement::Capability(POLICY_VIEW))
    .icon("flag")
    .order(110)
);
bilibi_menu::contribute!(
    "people/progression",
    MenuItem::page(
        "people.my_work.standing",
        "My standing",
        "/people/progression/my"
    )
    .company_scoped()
    .under("people.my_work")
    .requires(Requirement::AnyOf(&[
        SELF_VIEW,
        bilibi_people_performance::SELF_VIEW
    ]))
    .icon("users")
    .order(70)
);
