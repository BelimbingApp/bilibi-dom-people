//! PO2 explorer browser fixture. Records are created only through the facade
//! in the disposable walkthrough database, never in a product seed.
use bilibi_authz::grants::Grants;
use bilibi_database::{Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_organisation::{NewPosition, NewVersion, Organisation, SERVICE, VIEW};
use bilibi_tenancy::Scope;
use jiff::civil::date;

pub(crate) async fn run(services: &Services, admin: &Scope, manager: &Scope) -> Result<()> {
    services
        .get::<Grants>("base/authz.grants")?
        .set_direct(admin, 2, VIEW, Some(true))
        .await?;
    // The viewer has no affiliation, so give the company chooser reach
    // without granting permission to change Organisation assignments.
    services
        .get::<Grants>("base/authz.grants")?
        .set_direct(admin, 2, "admin.company.tenant-wide.manage", Some(true))
        .await?;
    let organisation = services.get::<Organisation>(SERVICE)?;
    // More than two pages at 25 rows; position identity order stays stable.
    for number in 1..=51 {
        let position = organisation
            .create_position(
                manager,
                1,
                &NewPosition::new(format!("EXPLORER-{number:03}")),
            )
            .await
            .map_err(|_| Error::Internal)?;
        organisation
            .record_version(
                manager,
                1,
                position.id,
                &NewVersion {
                    version: 1,
                    title: format!("Position {number:03}"),
                    effective_from: date(2026, 1, 1),
                    effective_to: if number == 1 {
                        Some(date(2026, 6, 30))
                    } else {
                        None
                    },
                },
            )
            .await
            .map_err(|_| Error::Internal)?;
        if number == 1 {
            organisation
                .record_version(
                    manager,
                    1,
                    position.id,
                    &NewVersion {
                        version: 2,
                        title: "Position 001 revised".into(),
                        effective_from: date(2026, 7, 1),
                        effective_to: None,
                    },
                )
                .await
                .map_err(|_| Error::Internal)?;
        }
    }
    println!("Organisation explorer: 51 additional positions, a dated title change, and viewer read-only access are ready for browser checks.");
    Ok(())
}

walkthrough_step!(
    "organisation_explorer",
    after = ["training_governance"],
    |ctx| async move {
        // PO2: paginated explorer records and a read-only browser account.
        crate::organisation_explorer::run(&ctx.services, &ctx.admin, &ctx.manager).await?;

        Ok(())
    }
);
