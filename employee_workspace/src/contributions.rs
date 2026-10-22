//! Bilimbi EmployeeWorkspace descriptor and capability vocabulary.
use crate::{CAPABILITIES, MANAGE, MODULE, REVIEW, VIEW};
use bilibi_authz::{CapabilityDefinition as C, RoleDefinition, VocabularyContribution, VOCABULARY};
use bilibi_module_registry::{Contribution, Layer, Module, Provenance};

inventory::submit! {
    Module {
        id: MODULE, layer: Layer::Domains,
        dependencies: &[
            "base/authz", "base/database", "base/menu", "base/module_registry",
            "base/party_ids", "base/tenancy", "base/ui", "core/affiliation",
            "core/company", "core/person", "people/settings", "people/workforce",
        ],
        migrations: &[("people-employee-workspace-001-employee-workspace", include_str!("../migrations/001-employee-workspace.sql"))],
        services: &[],
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
