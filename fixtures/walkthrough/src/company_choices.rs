//! Bilimbi People company pickers: lifecycle options and existing record context.
//! Standalone proof on an empty disposable database, with browser records.
use bilibi_authz::grants::Grants;
use bilibi_company::{CompanyService, CreateOutcome, NewCompany};
use bilibi_database::{pool, Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_reference_data::{NewEntry, ReferenceData};
use bilibi_people_workforce::{authorization, SETTINGS_MANAGE};
use bilibi_tenancy::Authentication;
use std::sync::Arc;

async fn run(url: &str) -> Result<()> {
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = pool(url)?;
    let modules = bilibi_module_registry::discover()?;
    bilibi_database::migrate(&db, &modules).await?;
    bilibi_walkthrough_fixture::install(&db).await?;
    let mut services = Services::default();
    services.provide("bootstrap.database", Arc::new(db.clone()))?;
    services.provide("bootstrap.composition", Arc::new(bilibi_web::composition()))?;
    services.install(&modules)?;
    services
        .get::<bilibi_base::step_up::StepUp>(bilibi_base::step_up::SERVICE)?
        .set_enforced(false);
    let client = db.get().await?;
    let auth = services.get::<Authentication>("base/tenancy.authentication")?;
    let admin = auth
        .authenticate(&**client, 1, 4, "company choices proof".into())
        .await?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    grants.reconcile().await?;
    for key in grants.vocabulary().capabilities.keys() {
        if key.starts_with("people.") || *key == bilibi_company::TENANT_WIDE {
            grants.set_direct(&admin, 1, key, Some(true)).await?;
        }
    }
    let manager = auth
        .authenticate(&**client, 1, 1, "company choices proof".into())
        .await?;
    let viewer = auth
        .authenticate(&**client, 1, 2, "company choices proof".into())
        .await?;
    let companies = services.get::<CompanyService>(bilibi_company::SERVICE)?;
    let references = services.get::<ReferenceData>(bilibi_people_reference_data::SERVICE)?;
    let mut ids = Vec::new();
    for status in ["active", "pending", "suspended", "archived"] {
        let new = NewCompany {
            name: format!("Chooser {status}"),
            code: Some(format!("chooser-{status}")),
            status: Some(
                if status == "pending" {
                    "pending"
                } else {
                    "active"
                }
                .into(),
            ),
            ..Default::default()
        };
        let company = match companies.create_company(&admin, &new).await? {
            CreateOutcome::Created(company) => company,
            CreateOutcome::Refused(errors) => {
                eprintln!("company refused: {errors:?}");
                return Err(Error::Internal);
            }
        };
        references
            .create_entry(
                &manager,
                company.id,
                &NewEntry::new(
                    "employment_type",
                    "chooser",
                    format!("Existing {status} reference"),
                ),
            )
            .await
            .map_err(|e| {
                eprintln!("reference: {e}");
                Error::Internal
            })?;
        if status == "suspended" {
            companies
                .suspend_company(&admin, company.id, Some("Chooser walkthrough"))
                .await
                .map_err(|e| {
                    eprintln!("suspend: {e}");
                    Error::Internal
                })?;
        } else if status == "archived" {
            companies
                .archive_company(&admin, company.id, Some("Chooser walkthrough"))
                .await
                .map_err(|e| {
                    eprintln!("archive: {e}");
                    Error::Internal
                })?;
        }
        ids.push((status, company.id));
    }
    for (status, id) in &ids {
        let summary = authorization::authorize_company(&**client, &manager, *id, SETTINGS_MANAGE)
            .await
            .map_err(failed)?;
        if summary.status != *status || summary.archived != (*status == "archived") {
            return Err(Error::Internal);
        }
        let choices = authorization::company_choices(&**client, &manager, SETTINGS_MANAGE, None)
            .await
            .map_err(failed)?;
        if choices.iter().any(|c| c.status != "active")
            || choices.iter().any(|c| c.id == *id) != (*status == "active")
        {
            return Err(Error::Internal);
        }
        let retained =
            authorization::company_choices(&**client, &manager, SETTINGS_MANAGE, Some(*id))
                .await
                .map_err(failed)?;
        if retained.iter().any(|c| c.id == *id && c.status == *status) != (*status != "archived")
            || retained.iter().any(|c| c.status != "active" && c.id != *id)
        {
            return Err(Error::Internal);
        }
        if !authorization::company_choices(&**client, &viewer, SETTINGS_MANAGE, Some(*id))
            .await
            .map_err(failed)?
            .is_empty()
        {
            return Err(Error::Internal);
        }
        println!("{status}: company {id}; existing record /people/companies/{id}/references");
    }
    // Retaining a selected company must not circumvent another tenant's reach.
    if authorization::company_choices(&**client, &manager, SETTINGS_MANAGE, Some(100))
        .await
        .map_err(failed)?
        .iter()
        .any(|c| c.id == 100)
    {
        return Err(Error::Internal);
    }
    // The record-directory API still carries pending/suspended companies.
    let records = authorization::selectable_companies(&**client, &manager, SETTINGS_MANAGE)
        .await
        .map_err(failed)?;
    if ids
        .iter()
        .filter(|(status, _)| *status != "archived")
        .any(|(_, id)| !records.iter().any(|c| c.id == *id))
    {
        return Err(Error::Internal);
    }
    println!("PASS: active-only choices, retained pending and suspended selections, hidden archived selection, denied capability and cross-tenant selection; manager@example.test ready for browser proof");
    Ok(())
}
fn failed(error: bilibi_people_workforce::WorkforceError) -> Error {
    eprintln!("chooser: {error}");
    Error::Internal
}
walkthrough_command!(
    "company-choices",
    |url, _argument| async move { run(url).await }
);
