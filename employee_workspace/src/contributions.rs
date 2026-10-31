//! Bilimbi EmployeeWorkspace descriptor and capability vocabulary.
use crate::{EmployeeWorkspace, SERVICE};
use crate::{CAPABILITIES, MANAGE, MODULE, REVIEW, VIEW};
use bilibi_authz::{CapabilityDefinition as C, RoleDefinition, VocabularyContribution, VOCABULARY};
use bilibi_database::{Db, MutationBoundary};
use bilibi_menu::{MenuItem, Requirement};
use bilibi_module_registry::contributions::{
    Access, Method, PageWrites, Pending, Route, PAGE_WRITES, ROUTES,
};
use bilibi_module_registry::{Contribution, Layer, Module, Provenance, Service};
use bilibi_tenancy::Scope;
use bilibi_ui::page::PageService;
use std::sync::Arc;
const PAGES: &str = "people/employee_workspace.pages";

inventory::submit! {
    Module {
        id: MODULE, layer: Layer::Domains,
        dependencies: &[
            "base/authz", "base/database", "base/menu", "base/module_registry",
            "base/party_ids", "base/tenancy", "base/ui", "core/affiliation",
            "core/company", "core/person", "people/settings", "people/workforce",
        ],
        migrations: &[("people-employee-workspace-001-employee-workspace", include_str!("../migrations/001-employee-workspace.sql")),
            ("people-employee-workspace-002-inline-versions", include_str!("../migrations/002-inline-versions.sql"))],
        services: &[
            Service { id: SERVICE, factory: |s| Ok(Arc::new(EmployeeWorkspace::new(
                s.get::<Db>("base/database.pool")?.as_ref().clone(),
                s.get::<MutationBoundary>("base/database.mutations")?,
                s.get(bilibi_people_workforce::SERVICE)?,
            ))) },
            Service { id: PAGES, factory: |s| Ok(Arc::new(PageService::<Scope, bilibi_database::Error>(Arc::new(crate::web::Pages::new(
                s.get::<Db>("base/database.pool")?.as_ref().clone(), s.get(SERVICE)?, s.get(bilibi_people_workforce::SERVICE)?,
                s.get(bilibi_person::SERVICE)?, s.get(bilibi_affiliation::SERVICE)?,
            ))))) },
        ],
    }
}
inventory::submit! {
    Contribution::new(VOCABULARY, "people/employee_workspace.vocabulary", Provenance {
        module: MODULE, source: file!(), line: line!(),
    }, &VocabularyContribution {
        domains: &[], verbs: &[],
        capabilities: &[C::tenant(VIEW), C::tenant(MANAGE), C::tenant(REVIEW)],
        roles: &[RoleDefinition {
            code: "tenant_owner", name: "Tenant Owner",
            description: "Contributed system role; reconciled explicitly",
            grant_all: false, capabilities: &CAPABILITIES,
        }], system_principals: &[],
    })
}

inventory::submit! {
    Contribution::new(ROUTES, "people/employee_workspace.employees", Provenance { module: MODULE, source: file!(), line: line!() }, &Route { company_scope: bilibi_module_registry::contributions::CompanyScope::Tenant,
        method: Method::Get, path: "/people/employees", service: PAGES, operation: "employees", access: Access::Capability(VIEW), framed: true,
    })
}
inventory::submit! {
    Contribution::new(PAGE_WRITES, PAGES, Provenance { module: MODULE, source: file!(), line: line!() }, &PageWrites {
        service: PAGES, contracts: &[], pending: Pending::Every { reason: "Employee workspace and employment writes use their owning facades; operation contracts are not yet reviewed." },
    })
}
bilibi_menu::contribute!(
    "people/employee_workspace",
    MenuItem::page("people.team.employees", "Employees", "/people/employees")
        .under("people.team")
        .requires(Requirement::Capability(VIEW))
        .order(10)
);

// Bilimbi EmployeeWorkspace.Web.ShowLive.
inventory::submit! {
    Contribution::new(ROUTES, "people/employee_workspace.employee", Provenance { module: MODULE, source: file!(), line: line!() }, &Route { company_scope: bilibi_module_registry::contributions::CompanyScope::Tenant,
        method: Method::Get, path: "/people/employees/{id}", service: PAGES, operation: "employee", access: Access::Capability(VIEW), framed: true,
    })
}
