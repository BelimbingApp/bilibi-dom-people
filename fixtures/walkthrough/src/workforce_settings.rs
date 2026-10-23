//! PW2 running-system proofs through Workforce, Base Ports and live page events.
use bilibi_authz::grants::Grants;
use bilibi_database::{pool, Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_workforce::{
    EmploymentChange, NewEmployment, Workforce, DIRECTORY_PORT, EMPLOYMENT_VIEW, SETTINGS_MANAGE,
};
use bilibi_ports::{PortError, Ports};
use bilibi_tenancy::{Authentication, Scope};
use bilibi_ui::page::{LiveBody, LiveInput, LiveRequest, LiveState, PageRequest, PageService};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc};

fn prove(ok: bool, label: &str) -> Result<()> {
    if !ok {
        eprintln!("PW2 FAILED: {label}");
        return Err(Error::Internal);
    }
    println!("PW2 proved: {label}");
    Ok(())
}
fn saved<T, E: std::fmt::Display>(result: std::result::Result<T, E>) -> Result<T> {
    result.map_err(|e| {
        eprintln!("PW2 unexpected refusal: {e}");
        Error::Internal
    })
}
async fn event(
    state: &mut dyn LiveState<Scope, Error>,
    scope: &Scope,
    payload: Value,
) -> Result<LiveBody> {
    state
        .update(LiveRequest {
            scope,
            credential: "PW2 fixture",
            input: LiveInput::Event(&payload),
        })
        .await
}

pub(super) async fn run(url: &str, action: &str) -> Result<()> {
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = pool(url)?;
    let modules = bilibi_module_registry::discover()?;
    if action == "setup" {
        bilibi_database::migrate(&db, &modules).await?;
        bilibi_walkthrough_fixture::install(&db).await?;
    }
    let mut services = Services::default();
    services.provide("bootstrap.database", Arc::new(db.clone()))?;
    services.provide("bootstrap.composition", Arc::new(bilibi_web::composition()))?;
    services.install(&modules)?;
    services
        .get::<bilibi_base::step_up::StepUp>(bilibi_base::step_up::SERVICE)?
        .set_enforced(false);
    let grants = services.get::<Grants>("base/authz.grants")?;
    grants.reconcile().await?;
    let authentication = services.get::<Authentication>("base/tenancy.authentication")?;
    let c = db.get().await?;
    let admin = authentication
        .authenticate(&**c, 1, 4, "PW2 fixture".into())
        .await?;
    let manager = authentication
        .authenticate(&**c, 1, 1, "PW2 fixture".into())
        .await?;
    let viewer = authentication
        .authenticate(&**c, 1, 2, "PW2 fixture".into())
        .await?;
    let workforce = services.get::<Workforce>(bilibi_people_workforce::SERVICE)?;
    match action {
        "revoke" | "restore" => {
            grants
                .set_direct(&admin, 1, SETTINGS_MANAGE, Some(action == "restore"))
                .await?;
            println!("PW2 changed manager settings grant: {action}");
            return Ok(());
        }
        "archive" => {
            saved(
                services
                    .get::<bilibi_company::CompanyService>(bilibi_company::SERVICE)?
                    .archive_company(&admin, 4, Some("PW2 mounted-page proof"))
                    .await,
            )?;
            println!("PW2 archived company 4 through Core");
            return Ok(());
        }
        "setup" => (),
        _ => return Err(Error::Invalid),
    }
    for key in [
        SETTINGS_MANAGE,
        EMPLOYMENT_VIEW,
        bilibi_company::TENANT_WIDE,
    ] {
        grants.set_direct(&admin, 1, key, Some(true)).await?;
    }
    let first = saved(
        workforce
            .create_employment(
                &admin,
                1,
                &NewEmployment::new(1, 1, "PW2-001", "fixture_type", "active"),
            )
            .await,
    )?;
    let people = services.get::<bilibi_person::PersonService>(bilibi_person::SERVICE)?;
    let affiliations =
        services.get::<bilibi_affiliation::AffiliationService>(bilibi_affiliation::SERVICE)?;
    let mut employees = Vec::new();
    for n in 2..=3 {
        let person = saved(
            people
                .create(
                    &admin,
                    &bilibi_person::PersonInput {
                        name: format!("PW2 employee {n}"),
                        kind: bilibi_person::PersonKind::Human,
                        email: None,
                        phone: None,
                    },
                )
                .await,
        )?;
        let affiliation = affiliations
            .create(
                &admin,
                &bilibi_affiliation::AffiliationInput {
                    person_id: person.id,
                    organisation_id: bilibi_party_ids::OrganisationId::new(1)
                        .map_err(|_| Error::Invalid)?,
                    title: None,
                    work_email: Some(format!("pw2-{n}@example.test")),
                    work_phone: None,
                    dates: bilibi_affiliation::EffectiveDates::new(None, None)
                        .map_err(|_| Error::Invalid)?,
                },
            )
            .await?;
        employees.push((person.id.get(), affiliation.id.get()));
    }
    let second = saved(
        workforce
            .create_employment(
                &admin,
                1,
                &NewEmployment {
                    supervisor_employment_id: Some(first.id),
                    ..NewEmployment::new(
                        employees[0].0,
                        employees[0].1,
                        "PW2-002",
                        "fixture_type",
                        "probation",
                    )
                },
            )
            .await,
    )?;
    saved(
        workforce
            .create_employment(
                &admin,
                2,
                &NewEmployment::new(2, 2, "PW2-OTHER", "fixture_type", "active"),
            )
            .await,
    )?;
    let ports = services.get::<Ports>(bilibi_ports::SERVICE)?;
    let company = ports
        .call_json(
            &c,
            &manager,
            DIRECTORY_PORT,
            json!({"kind":"company","company_id":1}),
        )
        .await
        .map_err(|_| Error::Internal)?;
    prove(
        company["company"]["platform_company_id"] == 1
            && company["company"]["workforce_company_id"] == 1,
        "company identity keeps both axes",
    )?;
    let start = json!({"kind":"employees","company_id":1,"page_size":1,"cursor":null});
    let page = ports
        .call_json(&c, &manager, DIRECTORY_PORT, start.clone())
        .await
        .map_err(|_| Error::Internal)?;
    prove(
        page["snapshot"] == true
            && page["employees"].as_array().is_some_and(|r| r.len() == 1)
            && page["next_cursor"].is_string(),
        "one-row SQL page has a continuation and snapshot marker",
    )?;
    let cursor = page["next_cursor"].clone();
    let third = saved(
        workforce
            .create_employment(
                &admin,
                1,
                &NewEmployment::new(
                    employees[1].0,
                    employees[1].1,
                    "PW2-003",
                    "fixture_type",
                    "active",
                ),
            )
            .await,
    )?;
    saved(
        workforce
            .update_employment(
                &admin,
                1,
                first.id,
                &EmploymentChange {
                    employee_number: Some("PW2-ZZZ".into()),
                    ..EmploymentChange::at(first.version)
                },
            )
            .await,
    )?;
    let next = ports
        .call_json(
            &c,
            &manager,
            DIRECTORY_PORT,
            json!({"kind":"employees","company_id":1,"page_size":1,"cursor":cursor}),
        )
        .await
        .map_err(|_| Error::Internal)?;
    prove(next["watermark"] == page["watermark"] && next["high_water_id"] == page["high_water_id"] && next["next_cursor"].is_null()
        && next["employees"][0]["reference"]["stable_id"].as_str().and_then(|s| s.parse::<i64>().ok()) == Some(second.id)
        && next["employees"][0]["supervisor_reference"]["stable_id"].as_str().and_then(|s| s.parse::<i64>().ok()) == Some(first.id), "renumbering and new rows cannot move the immutable keyset; watermark and supervisor survive")?;
    prove(
        next["employees"][0]["email"] == "pw2-2@example.test",
        "Core supplies the work email",
    )?;
    // Change a Core contact on the caller's audited transaction, then roll it
    // back. The port must see the pending owner write without another rental.
    let mutations = services.get::<bilibi_database::MutationBoundary>("base/database.mutations")?;
    let mut contact_client = db.get().await?;
    let mut contact_tx = mutations
        .begin(
            &mut contact_client,
            admin.mutation_context("people.workforce.directory_visibility_proof"),
        )
        .await?;
    let affiliation_id =
        bilibi_party_ids::AffiliationId::new(second.affiliation_id).map_err(|_| Error::Invalid)?;
    let affiliation =
        bilibi_affiliation::identity::read(contact_tx.sql(), &admin, affiliation_id).await?;
    let mut contact = affiliation.affiliation;
    contact.work_email = Some("pw2-pending@example.test".into());
    bilibi_affiliation::identity::update(
        &mut contact_tx,
        &admin,
        affiliation_id,
        &bilibi_affiliation::Update {
            version: affiliation.version,
            affiliation: contact,
        },
    )
    .await?;
    let pending = ports
        .call_json(
            &contact_tx,
            &admin,
            DIRECTORY_PORT,
            json!({"kind":"employees","company_id":1}),
        )
        .await
        .map_err(|_| Error::Internal)?;
    prove(
        pending["employees"].as_array().is_some_and(|rows| {
            rows.iter()
                .any(|r| r["email"] == "pw2-pending@example.test")
        }),
        "port sees pending Core contact changes on the caller's transaction",
    )?;
    contact_tx.rollback().await?;
    let hidden = ports
        .call_json(
            &c,
            &viewer,
            DIRECTORY_PORT,
            json!({"kind":"employees","company_id":1}),
        )
        .await
        .map_err(|_| Error::Internal)?;
    prove(
        hidden["employees"]
            .as_array()
            .is_some_and(|rows| rows.iter().all(|r| r["email"].is_null())),
        "Core contact permissions are preserved",
    )?;
    for bad in [
        json!({"kind":"employees","company_id":1,"page_size":0}),
        json!({"kind":"employees","company_id":1,"page_size":1001}),
        json!({"kind":"employees","company_id":1,"page_size":2,"cursor":cursor}),
        json!({"kind":"employees","company_id":2,"page_size":1,"cursor":cursor}),
        json!({"kind":"employees","company_id":1,"cursor":"broken"}),
        json!({"kind":"company","company_id":1,"actor_id":4}),
    ] {
        prove(
            matches!(
                ports.call_json(&c, &manager, DIRECTORY_PORT, bad).await,
                Err(PortError::Refused(Error::Invalid))
            ),
            "malformed requests and mismatched scans refuse",
        )?;
    }
    prove(
        ports
            .call_json(
                &c,
                &manager,
                DIRECTORY_PORT,
                json!({"kind":"company","company_id":100}),
            )
            .await
            .is_err(),
        "another tenant's company refuses",
    )?;
    saved(
        workforce
            .put_working_statuses(&admin, 1, &["active".into()])
            .await,
    )?;
    prove(
        ports
            .call_json(
                &c,
                &manager,
                DIRECTORY_PORT,
                json!({"kind":"employees","company_id":1,"page_size":1,"cursor":cursor}),
            )
            .await
            .is_err(),
        "changed working statuses invalidate an unfinished scan",
    )?;
    saved(
        workforce
            .put_working_statuses(&admin, 1, &["probation".into(), "active".into()])
            .await,
    )?;
    let org = services
        .get::<bilibi_people_organisation::Organisation>(bilibi_people_organisation::SERVICE)?;
    for code in ["pw2-position-one", "pw2-position-two"] {
        saved(
            org.create_position(
                &admin,
                1,
                &bilibi_people_organisation::NewPosition::new(code),
            )
            .await,
        )?;
    }
    let positions = ports
        .call_json(
            &c,
            &manager,
            DIRECTORY_PORT,
            json!({"kind":"positions","company_id":1,"as_of":"2026-10-24","page_size":1}),
        )
        .await
        .map_err(|_| Error::Internal)?;
    let continued = ports.call_json(&c, &manager, DIRECTORY_PORT, json!({"kind":"positions","company_id":1,"as_of":"2026-10-24","page_size":1,"cursor":positions["next_cursor"]})).await.map_err(|_| Error::Internal)?;
    prove(
        positions["positions"]
            .as_array()
            .is_some_and(|r| r.len() == 1)
            && continued["positions"]
                .as_array()
                .is_some_and(|r| r.len() == 1)
            && continued["next_cursor"].is_null()
            && positions["watermark"] == continued["watermark"],
        "Organisation pages pass through with one observation watermark",
    )?;

    let pages = services.get::<PageService<Scope, Error>>("people/workforce.pages")?;
    let empty = HashMap::new();
    let mut state = pages
        .0
        .mount(PageRequest {
            operation: "settings",
            scope: Some(&manager),
            path: &empty,
            query: &empty,
            body: &[],
            framed: false,
        })
        .await?
        .ok_or(Error::Internal)?;
    let body = event(&mut *state, &manager, json!({"type":"save_statuses"})).await?;
    prove(
        body.html.contains("Choose at least one employee status."),
        "empty selection is a recoverable refusal",
    )?;
    let body = event(
        &mut *state,
        &manager,
        json!({"type":"save_statuses","company_id":"2","statuses[active]":"true"}),
    )
    .await?;
    prove(
        body.html.contains("Working statuses saved.")
            && body.html.contains("workforce-employment-counts")
            && body.html.contains("Terminated")
            && body.query.get("company_id").is_some_and(|id| id == "1"),
        "status save and counts table retain server-owned company selection",
    )?;
    let body = event(
        &mut *state,
        &manager,
        json!({"type":"counts_grid","op":"zoom_preset","preset":"compact"}),
    )
    .await?;
    prove(
        body.html.contains("Working statuses saved."),
        "table events retain success flash",
    )?;
    grants
        .set_direct(&admin, 1, SETTINGS_MANAGE, Some(false))
        .await?;
    let body = event(
        &mut *state,
        &manager,
        json!({"type":"save_statuses","statuses[terminated]":"true"}),
    )
    .await?;
    prove(
        body.html.contains("Working statuses could not be saved.")
            && saved(workforce.working_statuses(&admin, 1).await)?.value
                == Some(vec!["active".into()]),
        "revoked settings grant refuses the next live event",
    )?;
    grants
        .set_direct(&admin, 1, SETTINGS_MANAGE, Some(true))
        .await?;
    saved(
        workforce
            .put_working_statuses(&admin, 1, &["probation".into(), "active".into()])
            .await,
    )?;
    grants
        .set_direct(&admin, 2, SETTINGS_MANAGE, Some(true))
        .await?;
    grants
        .set_direct(&admin, 2, bilibi_company::TENANT_WIDE, Some(true))
        .await?;
    // The viewer can save settings but has no grant for private employment counts.
    let body = pages
        .0
        .render(PageRequest {
            operation: "settings",
            scope: Some(&viewer),
            path: &empty,
            query: &empty,
            body: &[],
            framed: false,
        })
        .await?;
    prove(
        !body.html.contains("workforce-employment-counts")
            && body.html.contains("workforce-settings-form"),
        "employment counts require their own read grant",
    )?;
    event(
        &mut *state,
        &manager,
        json!({"type":"select_company","company_id":"3"}),
    )
    .await?;
    saved(
        services
            .get::<bilibi_company::CompanyService>(bilibi_company::SERVICE)?
            .archive_company(&admin, 3, Some("PW2 archive refusal proof"))
            .await,
    )?;
    let archived = event(
        &mut *state,
        &manager,
        json!({"type":"save_statuses","statuses[active]":"true"}),
    )
    .await?;
    prove(
        archived
            .html
            .contains("Working statuses could not be saved.")
            && !archived.html.contains("workforce-settings-form"),
        "a company archived after selection refuses the next save",
    )?;
    prove(
        matches!(
            ports
                .call_json(
                    &c,
                    &manager,
                    DIRECTORY_PORT,
                    json!({"kind":"company","company_id":3})
                )
                .await,
            Err(PortError::Refused(Error::NotFound))
        ),
        "directory refuses an archived company",
    )?;
    let _ = third;
    println!("PW2 browser fixture ready: admin@example.test, manager@example.test and viewer@example.test; password bilibi-dev.");
    Ok(())
}
