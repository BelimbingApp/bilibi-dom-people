//! What reference data adds to the shared catalogs (Bilimbi
//! `Bilimbi.People.ReferenceData.Contributions`): the module descriptor with
//! its migration, facade and page services, the `people.references.manage`
//! capability, the two routes of `priv/web_routes.exs` and the Settings ›
//! People references menu leaf.
use crate::{ReferenceData, MODULE, SERVICE};
use bilibi_authz::{CapabilityDefinition as C, VocabularyContribution, VOCABULARY};
use bilibi_database::{Db, MutationBoundary};
use bilibi_menu::{MenuItem, Requirement};
use bilibi_module_registry::{
    contributions::{Access, Method, PageWrites, Pending, Route, PAGE_WRITES, ROUTES},
    Contribution, Layer, Module, Provenance, Service,
};
use bilibi_tenancy::Scope;
use bilibi_ui::page::PageService;
use std::sync::Arc;

/// Adds entries, aliases and calendar exceptions, and opens the page. No role
/// is granted it by default, as in Bilimbi: an administrator assigns it.
pub const MANAGE: &str = "people.references.manage";

/// The company chooser, and the page of one company's references.
pub const REFERENCES_PATH: &str = "/people/references";
pub(crate) const COMPANY_REFERENCES_PATH: &str = "/people/companies/{company_id}/references";

/// The registry service ID of the module's pages.
const PAGES: &str = "people/reference_data.pages";

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
            "base/authz", "base/database", "base/menu", "base/module_registry",
            "base/ports", "base/tenancy", "base/ui", "core/company", "people/settings", "people/workforce",
        ],
        migrations: &[(
            "people-reference-data-001-references",
            include_str!("../migrations/001-references.sql"),
        )],
        services: &[
            Service {
                id: SERVICE,
                factory: |services| {
                    Ok(Arc::new(ReferenceData::new(
                        services.get::<Db>("base/database.pool")?.as_ref().clone(),
                        services.get::<MutationBoundary>("base/database.mutations")?,
                    )))
                },
            },
            Service {
                id: PAGES,
                factory: |services| {
                    let db = services.get::<Db>("base/database.pool")?.as_ref().clone();
                    let references = ReferenceData::new(
                        db.clone(),
                        services.get::<MutationBoundary>("base/database.mutations")?,
                    );
                    Ok(Arc::new(PageService::<Scope, bilibi_database::Error>(Arc::new(
                        crate::web::Pages::new(db, references),
                    ))))
                },
            },
        ],
    }
}

// The Authz domain `people` is declared by Workforce; `manage` is a verb Base
// already declares.
inventory::submit! {
    Contribution::new(VOCABULARY, "people/reference_data.vocabulary", provenance!(), &VocabularyContribution {
        domains: &[],
        verbs: &[],
        capabilities: &[C::tenant(MANAGE)],
        roles: &[],
        system_principals: &[],
    })
}

// The pages still write through the facade rather than an operation contract
// (`base/agent_api` README, "Page writes declaration and check"); the host
// refuses a live-event write from a page service that declares none.
inventory::submit! {
    Contribution::new(PAGE_WRITES, PAGES, provenance!(), &PageWrites {
        service: PAGES, contracts: &[],
        pending: Pending::Every { reason: "Reference entries, aliases and calendar exceptions write through the ReferenceData facade; no operation contract has been reviewed for them." },
    })
}

inventory::submit! {
    Contribution::new(ROUTES, "people/reference_data.references", provenance!(), &Route { company_scope: bilibi_module_registry::contributions::CompanyScope::Tenant,
        method: Method::Get, path: REFERENCES_PATH, service: PAGES, operation: "references",
        access: Access::Capability(MANAGE), framed: true,
    })
}
inventory::submit! {
    Contribution::new(ROUTES, "people/reference_data.company_references", provenance!(), &Route { company_scope: bilibi_module_registry::contributions::CompanyScope::Tenant,
        method: Method::Get, path: COMPANY_REFERENCES_PATH, service: PAGES, operation: "references",
        access: Access::Capability(MANAGE), framed: true,
    })
}

// Settings › People references: Bilimbi's `people.settings.references`, icon
// `clipboard-document-list`, order 10, under the `people.settings` group that
// `people/settings` contributes.
bilibi_menu::contribute!(
    "people/reference_data",
    MenuItem::page(
        "people.settings.references",
        "People references",
        REFERENCES_PATH
    )
    .icon("clipboard-document-list")
    .under("people.settings")
    .requires(Requirement::Capability(MANAGE))
    .order(10)
);

inventory::submit! {
    Contribution::new(bilibi_ports::PORTS, "people/reference_data.employment_types", provenance!(), &bilibi_ports::Port { provider: &crate::port::EmploymentTypes })
}
