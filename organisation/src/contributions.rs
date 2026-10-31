//! What Organisation adds to the shared catalogs (Bilimbi
//! `Bilimbi.People.Organisation.Contributions`): the module descriptor with
//! its migration and facade service, the two capabilities under the `people`
//! Authz domain, and the `people/organisation.positions` port Workforce's
//! position scan reads through, and the Organisation explorer under Team.
use crate::{agent::DEFINITIONS_VALUE, port::PositionsPort, Organisation, MODULE, SERVICE};
use bilibi_authz::{CapabilityDefinition as C, VocabularyContribution, VOCABULARY};
use bilibi_database::{Db, MutationBoundary};
use bilibi_menu::{MenuItem, Requirement};
use bilibi_module_registry::contributions::{
    Access, Method, PageWrites, Pending, Route, PAGE_WRITES, ROUTES,
};
use bilibi_module_registry::{Contribution, Layer, Module, Provenance, Service};
use bilibi_ports::{Port, PORTS};
use bilibi_tenancy::Scope;
use bilibi_ui::page::PageService;
const PAGES: &str = "people/organisation.pages";
use std::sync::Arc;

/// Opens the organisation explorer and reads its positions.
pub const VIEW: &str = "people.organisation.view";
/// Creates and moves positions, records versions, assigns and ends.
pub const MANAGE: &str = "people.organisation.manage";
/// Every Organisation capability. Bilimbi grants neither to a system role; an
/// administrator grants them.
pub const CAPABILITIES: [&str; 2] = [VIEW, MANAGE];
/// The port through which Workforce's `positions` scan reads this module's
/// positions without an edge. Its request and answer are in `README.md`.
pub const POSITIONS_PORT: &str = "people/organisation.positions";

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
            "base/agent_api", "base/audit", "base/authz", "base/database", "base/module_registry", "base/ports",
            "base/tenancy", "base/menu", "base/ui", "core/company", "people/settings", "people/workforce",
        ],
        migrations: &[(
            "people-organisation-001-positions",
            include_str!("../migrations/001-positions.sql"),
        )],
        services: &[Service {
            id: SERVICE,
            factory: |services| {
                let facade = Arc::new(Organisation {
                    db: services.get::<Db>("base/database.pool")?.as_ref().clone(),
                    mutations: services.get::<MutationBoundary>("base/database.mutations")?,
                    workforce: services.get(bilibi_people_workforce::SERVICE)?,
                });
                crate::agent::bind(facade.clone())?;
                // The port provider is a static; this is how it reaches the
                // facade it answers through.
                crate::port::install(facade.clone());
                Ok(facade)
            },
        }, Service {
            id: PAGES,
            factory: |services| Ok(Arc::new(PageService::<Scope, bilibi_database::Error>(Arc::new(crate::web::Pages::new(
                services.get::<Db>("base/database.pool")?.as_ref().clone(),
                services.get(SERVICE)?,
                services.get(bilibi_people_workforce::SERVICE)?,
            ))))),
        }],
    }
}

// The Authz domain is `people`, as every People module declares it; `view`
// and `manage` are verbs Base already declares.
inventory::submit! {
    Contribution::new(VOCABULARY, "people/organisation.vocabulary", provenance!(), &VocabularyContribution {
        domains: &["people"],
        verbs: &[],
        capabilities: &[C::tenant(VIEW), C::tenant(MANAGE)],
        roles: &[],
        system_principals: &[],
    })
}

inventory::submit! {
    Contribution::new(PORTS, POSITIONS_PORT, provenance!(), &Port { provider: &PositionsPort })
}

inventory::submit! {
    Contribution::new(bilibi_agent_api::DEFINITIONS, "people/organisation.agent", provenance!(), &DEFINITIONS_VALUE)
}

// Bilimbi Organisation.Web.ExplorerLive and its Team menu leaf.
inventory::submit! {
    Contribution::new(ROUTES, "people/organisation.explorer", provenance!(), &Route { company_scope: bilibi_module_registry::contributions::CompanyScope::Tenant,
        method: Method::Get, path: "/people/organisation", service: PAGES,
        operation: "explorer", access: Access::Capability(VIEW), framed: true,
    })
}
inventory::submit! {
    Contribution::new(PAGE_WRITES, PAGES, provenance!(), &PageWrites {
        service: PAGES, contracts: &[], pending: Pending::Every {
            reason: "Explorer assignment writes use the authorised Organisation facade; assignment contracts are not yet reviewed.",
        },
    })
}
bilibi_menu::contribute!(
    "people/organisation",
    MenuItem::page(
        "people.team.organisation",
        "Organisation",
        "/people/organisation"
    )
    .under("people.team")
    .requires(Requirement::Capability(VIEW))
    .order(20)
);
