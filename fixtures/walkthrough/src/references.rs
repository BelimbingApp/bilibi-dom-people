//! The reference data block of the walkthrough (slice PD1): the manager is
//! granted `people.references.manage` (Bilimbi grants it to no role), enters a
//! company's own reference values, aliases and calendar exceptions through the
//! facade, and the facade and PostgreSQL refuse what they should (a viewer
//! without the grant, a taken code or alias, blank and overlong fields,
//! another tenant's company, a revoked grant, an archived company, a repeated
//! key, a restricted delete, an alias under another kind or company). The
//! grant stays for the running app.
use crate::{refused, saved, show, COMPANY, MANAGER, OTHER_COMPANY, OTHER_TENANT_COMPANY};
use bilibi_authz::grants::Grants;
use bilibi_database::{Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_reference_data::{
    NewCalendarException, NewEntry, ReferenceData, MANAGE as REFERENCES_MANAGE,
    SERVICE as REFERENCES_SERVICE,
};
use bilibi_tenancy::Scope;
use jiff::civil::date;

pub(crate) async fn run(
    services: &Services,
    grants: &Grants,
    client: &tokio_postgres::Client,
    admin: &Scope,
    manager: &Scope,
    viewer: &Scope,
) -> Result<()> {
    let references = services.get::<ReferenceData>(REFERENCES_SERVICE)?;
    println!("-- reference data");
    grants
        .set_direct(admin, MANAGER, REFERENCES_MANAGE, Some(true))
        .await?;
    println!("admin grants manager@example.test {REFERENCES_MANAGE}");

    show(
        "manager lists company 1's entries before any",
        references.list_entries(manager, COMPANY).await,
    );
    show(
        "viewer (no grant) lists company 1's calendar exceptions",
        references.list_calendar_exceptions(viewer, COMPANY).await,
    );
    show(
        "viewer adds an entry",
        references
            .create_entry(viewer, COMPANY, &NewEntry::new("employment_type", "x", "X"))
            .await,
    );
    let full_time = saved(show(
        "manager adds the employment_type entry full_time",
        references
            .create_entry(
                manager,
                COMPANY,
                &NewEntry::new("employment_type", "full_time", "Full time"),
            )
            .await,
    ))?;
    show(
        "manager adds the employment_type entry contractor",
        references
            .create_entry(
                manager,
                COMPANY,
                &NewEntry::new("employment_type", "contractor", "Contractor"),
            )
            .await,
    );
    show(
        "manager adds full_time again",
        references
            .create_entry(
                manager,
                COMPANY,
                &NewEntry::new("employment_type", "full_time", "Full time"),
            )
            .await,
    );
    show(
        "manager adds an entry with a blank code and a 81-character kind",
        references
            .create_entry(
                manager,
                COMPANY,
                &NewEntry::new("k".repeat(81), "  ", "Label"),
            )
            .await,
    );
    let other = saved(show(
        "manager adds full_time at company 2 (tenant-wide reach)",
        references
            .create_entry(
                manager,
                OTHER_COMPANY,
                &NewEntry::new("employment_type", "full_time", "Full time"),
            )
            .await,
    ))?;
    show(
        "viewer reads company 1's employment_type entries",
        references.list(viewer, COMPANY, "employment_type").await,
    );
    show(
        "viewer reads company 1's entries of an unused kind",
        references.list(viewer, COMPANY, "nothing").await,
    );

    show(
        "manager adds the alias Permanent to full_time",
        references
            .add_alias(manager, COMPANY, full_time.id, " Permanent ")
            .await,
    );
    show(
        "manager adds the alias Permanent again",
        references
            .add_alias(manager, COMPANY, full_time.id, "Permanent")
            .await,
    );
    show(
        "manager aliases company 2's entry at company 1",
        references
            .add_alias(manager, COMPANY, other.id, "Staff")
            .await,
    );
    show(
        "manager adds a blank alias",
        references
            .add_alias(manager, COMPANY, full_time.id, "")
            .await,
    );
    show(
        "manager lists company 1's aliases",
        references.list_aliases(manager, COMPANY).await,
    );

    show(
        "manager adds the calendar exception 2026-12-25 Company day",
        references
            .create_calendar_exception(
                manager,
                COMPANY,
                &NewCalendarException::new(date(2026, 12, 25), "Company day"),
            )
            .await,
    );
    show(
        "manager adds the same exception again",
        references
            .create_calendar_exception(
                manager,
                COMPANY,
                &NewCalendarException::new(date(2026, 12, 25), "Company day"),
            )
            .await,
    );
    show(
        "manager adds a second label on 2026-12-25",
        references
            .create_calendar_exception(
                manager,
                COMPANY,
                &NewCalendarException::new(date(2026, 12, 25), "Inventory"),
            )
            .await,
    );
    show(
        "manager adds an exception with a blank label",
        references
            .create_calendar_exception(
                manager,
                COMPANY,
                &NewCalendarException::new(date(2026, 1, 1), " "),
            )
            .await,
    );
    show(
        "viewer reads company 1's calendar exceptions",
        references.list_calendar_exceptions(viewer, COMPANY).await,
    );

    show(
        "manager reads another tenant's company 100",
        references.list_entries(manager, OTHER_TENANT_COMPANY).await,
    );
    show(
        "manager adds an entry at another tenant's company 100",
        references
            .create_entry(
                manager,
                OTHER_TENANT_COMPANY,
                &NewEntry::new("employment_type", "x", "X"),
            )
            .await,
    );

    // A grant revoked while a page stays open refuses the very next write.
    grants
        .set_direct(admin, MANAGER, REFERENCES_MANAGE, None)
        .await?;
    show(
        "manager adds an entry after the grant is revoked",
        references
            .create_entry(
                manager,
                COMPANY,
                &NewEntry::new("employment_type", "temp", "Temp"),
            )
            .await,
    );
    grants
        .set_direct(admin, MANAGER, REFERENCES_MANAGE, Some(true))
        .await?;

    // An archived company refuses the write; its references still read.
    client
        .execute(
            "UPDATE companies SET status = 'archived' WHERE id = $1",
            &[&OTHER_COMPANY],
        )
        .await
        .map_err(|_| Error::Internal)?;
    show(
        "manager adds an entry at archived company 2",
        references
            .create_entry(
                manager,
                OTHER_COMPANY,
                &NewEntry::new("employment_type", "temp", "Temp"),
            )
            .await,
    );
    show(
        "manager reads archived company 2's entries",
        references.list_entries(manager, OTHER_COMPANY).await,
    );
    client
        .execute(
            "UPDATE companies SET status = 'active' WHERE id = $1",
            &[&OTHER_COMPANY],
        )
        .await
        .map_err(|_| Error::Internal)?;

    // PostgreSQL itself refuses what the facade never attempts.
    for (step, sql) in [
        (
            "SQL inserts a duplicate (company, kind, code)",
            "INSERT INTO people_reference_data_entries(tenant_id,company_id,kind,code,label) SELECT tenant_id,company_id,kind,code,'Again' FROM people_reference_data_entries WHERE id=$1",
        ),
        (
            "SQL deletes an entry that has an alias",
            "DELETE FROM people_reference_data_entries WHERE id=$1",
        ),
        (
            "SQL aliases the entry under another kind",
            "INSERT INTO people_reference_data_aliases(tenant_id,company_id,entry_id,kind,label) SELECT tenant_id,company_id,id,'other_kind','Wrong kind' FROM people_reference_data_entries WHERE id=$1",
        ),
        (
            "SQL aliases the entry under another company",
            "INSERT INTO people_reference_data_aliases(tenant_id,company_id,entry_id,kind,label) SELECT tenant_id,2,id,kind,'Wrong company' FROM people_reference_data_entries WHERE id=$1",
        ),
        (
            "SQL repeats the alias label for the kind",
            "INSERT INTO people_reference_data_aliases(tenant_id,company_id,entry_id,kind,label) SELECT tenant_id,company_id,id,kind,'Permanent' FROM people_reference_data_entries WHERE id=$1",
        ),
        (
            "SQL repeats a calendar exception",
            "INSERT INTO people_reference_data_calendar_overrides(tenant_id,company_id,on_date,label) SELECT tenant_id,company_id,'2026-12-25','Company day' FROM people_reference_data_entries WHERE id=$1",
        ),
    ] {
        refused(client, step, sql, full_time.id).await;
    }
    println!(
        "reference data: manager@example.test now sees Settings > People references in the sidebar (/people/references); viewer@example.test holds no grant and does not."
    );
    Ok(())
}

walkthrough_step!(
    "references",
    after = ["claims_all_status"],
    |ctx| async move {
        let client = ctx.db.get().await?;
        crate::references::run(
            &ctx.services,
            ctx.services.get::<Grants>("base/authz.grants")?.as_ref(),
            &client,
            &ctx.admin,
            &ctx.manager,
            &ctx.viewer,
        )
        .await?;

        Ok(())
    }
);
