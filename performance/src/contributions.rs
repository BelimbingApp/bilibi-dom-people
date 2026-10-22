//! Bilimbi Performance.Contributions; the navigation leaf follows its page in PM2.
use crate::{Performance, MODULE, SERVICE};
use bilibi_authz::{CapabilityDefinition as C, VocabularyContribution, VOCABULARY};
use bilibi_database::{Db, MutationBoundary};
use bilibi_module_registry::{Contribution, Layer, Module, Provenance, Service};
use std::sync::Arc;

pub const VIEW: &str = "people.performance.view";
pub const DESCRIPTIONS_MANAGE: &str = "people.performance.descriptions.manage";
pub const KPIS_SUBMIT: &str = "people.performance.kpis.submit";
pub const KPIS_REVIEW: &str = "people.performance.kpis.review";
pub const KPIS_APPROVE: &str = "people.performance.kpis.approve";
pub const REVIEWS_SUBMIT: &str = "people.performance.reviews.submit";
pub const REVIEWS_APPROVE: &str = "people.performance.reviews.approve";
pub const SELF_VIEW: &str = "people.performance.self.view";
pub const CAPABILITIES: [&str; 8] = [
    VIEW,
    DESCRIPTIONS_MANAGE,
    KPIS_SUBMIT,
    KPIS_REVIEW,
    KPIS_APPROVE,
    REVIEWS_SUBMIT,
    REVIEWS_APPROVE,
    SELF_VIEW,
];

inventory::submit! { Module {
    id: MODULE, layer: Layer::Domains,
    dependencies: &["base/authz", "base/database", "base/module_registry", "base/tenancy", "core/company", "people/organisation", "people/settings", "people/skills", "people/workforce"],
    migrations: &[("people-performance-001-performance", include_str!("../migrations/001-performance.sql"))],
    services: &[Service {id: SERVICE, factory: |services| Ok(Arc::new(Performance {
        db: services.get::<Db>("base/database.pool")?.as_ref().clone(),
        mutations: services.get::<MutationBoundary>("base/database.mutations")?,
        workforce: services.get(bilibi_people_workforce::SERVICE)?,
        organisation: services.get(bilibi_people_organisation::SERVICE)?,
        skills: services.get(bilibi_people_skills::SERVICE)?,
    }))}],
}}
inventory::submit! { Contribution::new(VOCABULARY, "people/performance.vocabulary", Provenance {module: MODULE, source: file!(), line: line!()}, &VocabularyContribution {
    domains: &["people"], verbs: &["submit", "review", "approve"],
    capabilities: &[C::tenant(VIEW), C::tenant(DESCRIPTIONS_MANAGE), C::tenant(KPIS_SUBMIT), C::tenant(KPIS_REVIEW), C::tenant(KPIS_APPROVE), C::tenant(REVIEWS_SUBMIT), C::tenant(REVIEWS_APPROVE), C::tenant(SELF_VIEW)],
    roles: &[], system_principals: &[],
})}
