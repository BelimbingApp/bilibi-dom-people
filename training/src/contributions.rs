//! Training catalog, participation, governance, evaluation and Learning contributions.
//! Mirrors Bilimbi Training.Contributions. No Training key is granted to a system role.
//! web_learning.rs contributes six Learning routes and three menu leaves;
//! evaluation.rs contributes its six capabilities, the answer verb and six settings.
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
/// Opens the records workspace without granting company-wide attendance access.
pub const RECORDS_WORKSPACE_VIEW: &str = "people.training.records.workspace.view";
/// Reads attendance revisions and private evidence.
pub const RECORDS_VIEW: &str = "people.training.records.view";
pub const RECORDS_MANAGE: &str = "people.training.records.manage";
pub const EVIDENCE_MANAGE: &str = "people.training.evidence.manage";
pub const RETENTION_MANAGE: &str = "people.training.retention.manage";
/// Every Training capability currently implemented.
pub const CAPABILITIES: [&str; 30] = [
    crate::LEARNING_VIEW,
    crate::PASSPORT_MY_VIEW,
    crate::PASSPORT_TEAM_VIEW,
    crate::PASSPORT_GENERATE,
    crate::INSIGHTS_VIEW,
    COURSES_VIEW,
    COURSES_MANAGE,
    SESSIONS_VIEW,
    SESSIONS_MANAGE,
    RECORDS_WORKSPACE_VIEW,
    RECORDS_VIEW,
    RECORDS_MANAGE,
    EVIDENCE_MANAGE,
    RETENTION_MANAGE,
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
    crate::EFFECTIVENESS_VIEW,
    crate::EVALUATION_SUBMIT,
    crate::EFFECTIVENESS_ANSWER,
    crate::EFFECTIVENESS_SUMMARY_VIEW,
    crate::EVALUATION_POLICY_MANAGE,
    crate::EVALUATION_REMINDERS_MANAGE,
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
            "base/artifacts", "base/authz", "base/database", "base/datetime", "base/module_registry",
            "base/party_ids", "base/settings", "base/tenancy", "base/menu", "base/system", "base/ui",
            "core/company",
            bilibi_people_settings::MODULE, bilibi_people_workforce::MODULE,
        ],
        migrations: &[
            ("people-training-001-catalog", include_str!("../migrations/001-catalog.sql")),
            ("people-training-002-participation", include_str!("../migrations/002-participation.sql")),
            ("people-training-003-learning-governance", include_str!("../migrations/003-learning-governance.sql")),
            ("people-training-004-evaluation", include_str!("../migrations/004-evaluation.sql")),
        ],
        services: &[Service {
            id: SERVICE,
            factory: |services| {
                Ok(Arc::new(Training {
                    db: services.get::<Db>("base/database.pool")?.as_ref().clone(),
                    mutations: services.get::<MutationBoundary>("base/database.mutations")?,
                    settings: services.get("base/settings.store")?,
                    workforce: services.get(bilibi_people_workforce::SERVICE)?,
                    artifacts: services.get("base/artifacts.store")?,
                }))
            },
        }, Service {
            id: crate::download::SERVICE,
            factory: |services| Ok(Arc::new(bilibi_ui::download::DownloadService::<bilibi_tenancy::Scope, bilibi_database::Error>(Arc::new(crate::download::EvidenceDownload {
                training: services.get(SERVICE)?,
            })))),
        }, Service {
            id: crate::web_catalog::PAGES,
            factory: |services| Ok(Arc::new(bilibi_ui::page::PageService::<bilibi_tenancy::Scope, bilibi_database::Error>(Arc::new(crate::web_catalog::Pages(services.get(SERVICE)?))))),
        }, Service {
            id: crate::web_records::PAGES,
            factory: |services| Ok(Arc::new(bilibi_ui::page::PageService::<bilibi_tenancy::Scope, bilibi_database::Error>(Arc::new(crate::web_records::Pages(services.get(SERVICE)?))))),
        }, Service {
            id: crate::web_learning::PAGES,
            factory: |services| Ok(Arc::new(bilibi_ui::page::PageService::<bilibi_tenancy::Scope, bilibi_database::Error>(Arc::new(crate::web_learning::Pages(services.get(SERVICE)?))))),
        }, Service {
            id: crate::web_effectiveness::PAGES,
            factory: |services| Ok(Arc::new(bilibi_ui::page::PageService::<bilibi_tenancy::Scope, bilibi_database::Error>(Arc::new(crate::web_effectiveness::Pages(services.get(SERVICE)?))))),
        }, Service {
            id: crate::web_insights::PAGES,
            factory: |services| Ok(Arc::new(bilibi_ui::page::PageService::<bilibi_tenancy::Scope, bilibi_database::Error>(Arc::new(crate::web_insights::Pages(services.get(SERVICE)?))))),
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
            C::tenant(RECORDS_WORKSPACE_VIEW), C::tenant(RECORDS_VIEW), C::tenant(RECORDS_MANAGE),
            C::tenant(EVIDENCE_MANAGE), C::tenant(RETENTION_MANAGE),
        ],
        roles: &[],
        system_principals: &[],
    })
}

// Bilimbi Training.Web.EvidenceController. Artifacts and the facade re-prove
// access on every download; FileBody supplies the shared private headers.
inventory::submit! {
    Contribution::new(bilibi_module_registry::contributions::DOWNLOADS,
        "people/training.evidence.download", provenance!(),
        &bilibi_module_registry::contributions::Download {
            path: "/people/training/evidence/{company_id}/{id}",
            service: crate::download::SERVICE,
            operation: "evidence",
            access: bilibi_module_registry::contributions::Access::Capability(RECORDS_VIEW),
        })
}

// Bilimbi Passport and Insights vocabulary. Pages/menu land with PT7/PT9.
inventory::submit! {
    Contribution::new(VOCABULARY, "people/training.passport_vocabulary", provenance!(), &VocabularyContribution {
        domains: &["people"], verbs: &["generate"],
        capabilities: &[C::tenant(crate::PASSPORT_MY_VIEW), C::tenant(crate::PASSPORT_TEAM_VIEW), C::tenant(crate::PASSPORT_GENERATE), C::tenant(crate::INSIGHTS_VIEW)],
        roles: &[], system_principals: &[],
    })
}
macro_rules! passport_download {
    ($key:literal,$path:literal,$operation:literal,$cap:expr) => {
        inventory::submit! {
            Contribution::new(bilibi_module_registry::contributions::DOWNLOADS, $key, provenance!(),
                &bilibi_module_registry::contributions::Download {
                    path:$path, service:crate::download::SERVICE, operation:$operation,
                    access:bilibi_module_registry::contributions::Access::Capability($cap),
                })
        }
    };
}
passport_download!(
    "people/training.passport.my.document",
    "/people/training/records/my/document/{company_id}/{id}",
    "my_document",
    crate::PASSPORT_MY_VIEW
);
passport_download!(
    "people/training.passport.my.evidence",
    "/people/training/records/my/evidence/{company_id}/{id}",
    "my_evidence",
    crate::PASSPORT_MY_VIEW
);
passport_download!(
    "people/training.passport.team.document",
    "/people/training/records/team/document/{company_id}/{id}",
    "team_document",
    crate::PASSPORT_TEAM_VIEW
);
passport_download!(
    "people/training.passport.team.evidence",
    "/people/training/records/team/evidence/{company_id}/{id}",
    "team_evidence",
    crate::PASSPORT_TEAM_VIEW
);
