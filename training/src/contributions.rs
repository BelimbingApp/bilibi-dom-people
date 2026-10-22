//! What Training adds to the shared catalogs (Bilimbi
//! `Bilimbi.People.Training.Contributions`): the module descriptor with its
//! catalog migration and facade service, and the catalog capabilities under
//! the `people` Authz domain. Bilimbi grants no Training key to a system
//! role: an administrator grants them. The participation, governance,
//! evaluation and passport keys, the three verbs they add (`recommend`,
//! `answer`, `generate`), the eight menu leaves and the company settings land
//! with their slices beside this block.
use crate::{Training, MODULE, SERVICE};
use bilibi_authz::{CapabilityDefinition as C, VocabularyContribution, VOCABULARY};
use bilibi_database::{Db, MutationBoundary};
use bilibi_module_registry::{Contribution, Layer, Module, Provenance, Service};
use std::sync::Arc;

/// Reads the company's course catalog (`/people/training/courses`).
pub const COURSES_VIEW: &str = "people.training.courses.view";
/// Adds and changes courses. Manage does not silently grant view.
pub const COURSES_MANAGE: &str = "people.training.courses.manage";
/// Reads events and the session calendar (`/people/training/sessions`).
pub const SESSIONS_VIEW: &str = "people.training.sessions.view";
/// Adds events and sessions, and picks among the active courses for them.
pub const SESSIONS_MANAGE: &str = "people.training.sessions.manage";
/// Every catalog capability.
pub const CAPABILITIES: [&str; 14] = [
    COURSES_VIEW,
    COURSES_MANAGE,
    SESSIONS_VIEW,
    SESSIONS_MANAGE,
    crate::REQUESTS_SUBMIT,
    crate::REQUESTS_RECOMMEND,
    crate::REQUESTS_REVIEW,
    crate::REQUESTS_APPROVE,
    crate::REQUESTS_VIEW,
    crate::PLANS_SUBMIT,
    crate::PLANS_APPROVE,
    crate::PLANS_VIEW,
    crate::BUDGETS_VIEW,
    crate::BUDGETS_MANAGE,
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
            "base/authz", "base/database", "base/datetime", "base/module_registry",
            "base/party_ids", "base/settings", "base/tenancy",
            "core/company",
            bilibi_people_settings::MODULE, bilibi_people_workforce::MODULE,
        ],
        migrations: &[(
            "people-training-001-catalog",
            include_str!("../migrations/001-catalog.sql"),
        ), ("people-training-003-learning-governance", include_str!("../migrations/003-learning-governance.sql"))],
        services: &[Service {
            id: SERVICE,
            factory: |services| {
                Ok(Arc::new(Training {
                    db: services.get::<Db>("base/database.pool")?.as_ref().clone(),
                    mutations: services.get::<MutationBoundary>("base/database.mutations")?,
                    settings: services.get("base/settings.store")?,
                    workforce: services.get(bilibi_people_workforce::SERVICE)?,
                }))
            },
        }],
    }
}

// The Authz domain is `people`, declared by every People module as Bilimbi
// does; `view` and `manage` are verbs Base already declares.
inventory::submit! {
    Contribution::new(VOCABULARY, "people/training.vocabulary", provenance!(), &VocabularyContribution {
        domains: &["people"],
        verbs: &[],
        capabilities: &[
            C::tenant(COURSES_VIEW), C::tenant(COURSES_MANAGE),
            C::tenant(SESSIONS_VIEW), C::tenant(SESSIONS_MANAGE),
        ],
        roles: &[],
        system_principals: &[],
    })
}
