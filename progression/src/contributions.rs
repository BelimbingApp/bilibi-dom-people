//! Bilimbi Progression.Contributions; the navigation leaf follows its page in PG2.
use crate::{Progression, MODULE, SERVICE};
use bilibi_authz::{CapabilityDefinition as C, VocabularyContribution, VOCABULARY};
use bilibi_database::{Db, MutationBoundary};
use bilibi_module_registry::{Contribution, Layer, Module, Provenance, Service};
use std::sync::Arc;

pub const POLICY_VIEW: &str = "people.progression.policy.view";
pub const POLICY_MANAGE: &str = "people.progression.policy.manage";
pub const SELF_VIEW: &str = "people.progression.self.view";
pub const CAPABILITIES: [&str; 3] = [POLICY_VIEW, POLICY_MANAGE, SELF_VIEW];

inventory::submit! { Module {
    id: MODULE, layer: Layer::Domains,
    dependencies: &["base/authz", "base/database", "base/module_registry", "base/tenancy", "core/company", "people/performance", "people/settings", "people/skills", "people/workforce"],
    migrations: &[("people-progression-001-progression", include_str!("../migrations/001-progression.sql"))],
    services: &[Service {id: SERVICE, factory: |services| Ok(Arc::new(Progression {
        db: services.get::<Db>("base/database.pool")?.as_ref().clone(),
        mutations: services.get::<MutationBoundary>("base/database.mutations")?,
        workforce: services.get(bilibi_people_workforce::SERVICE)?,
        performance: services.get(bilibi_people_performance::SERVICE)?,
        skills: services.get(bilibi_people_skills::SERVICE)?,
    }))}],
}}
inventory::submit! { Contribution::new(VOCABULARY, "people/progression.vocabulary", Provenance {module: MODULE, source: file!(), line: line!()}, &VocabularyContribution {
    domains: &["people"], verbs: &[],
    capabilities: &[C::tenant(POLICY_VIEW), C::tenant(POLICY_MANAGE), C::tenant(SELF_VIEW)],
    roles: &[], system_principals: &[],
})}
