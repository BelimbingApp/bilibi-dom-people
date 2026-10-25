//! PT6 running-system proof: real page mounts, facade writes and current grants.
use crate::{ADMIN, COMPANY, MANAGER, TENANT, VIEWER};
use bilibi_authz::grants::Grants;
use bilibi_database::{Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_training::{
    NewCourse, NewEvent, NewSession, Training, COURSES_MANAGE, COURSES_VIEW, SESSIONS_VIEW,
};
use bilibi_tenancy::Authentication;
use bilibi_tenancy::Scope;
use bilibi_ui::page::{LiveBody, LiveInput, LiveRequest, LiveState, PageRequest, PageService};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

async fn send(
    state: &mut dyn LiveState<Scope, Error>,
    scope: &Scope,
    event: Value,
) -> Result<LiveBody> {
    state
        .update(LiveRequest {
            scope,
            credential: "walkthrough",
            input: LiveInput::Event(&event),
        })
        .await
}
fn prove(condition: bool, description: &str) -> Result<()> {
    if !condition {
        eprintln!("PT6 proof failed: {description}");
        return Err(Error::Internal);
    }
    println!("PT6 proved: {description}");
    Ok(())
}
pub(crate) async fn run(
    services: &Services,
    admin: &Scope,
    manager: &Scope,
    viewer: &Scope,
) -> Result<()> {
    let training = services.get::<Training>(bilibi_people_training::SERVICE)?;
    let pages = services.get::<PageService<Scope, Error>>("people/training.catalog_pages")?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    for cap in [
        COURSES_VIEW,
        SESSIONS_VIEW,
        "admin.company.tenant-wide.manage",
    ] {
        grants.set_direct(admin, VIEWER, cap, Some(true)).await?;
    }
    let path = HashMap::new();
    let query = HashMap::from([("company_id".into(), COMPANY.to_string())]);
    let request = |operation| PageRequest {
        operation,
        scope: Some(manager),
        path: &path,
        query: &query,
        body: &[],
        framed: false,
    };
    let mut courses = pages
        .0
        .mount(request("courses"))
        .await?
        .ok_or(Error::Internal)?;
    send(&mut *courses, manager, json!({"type":"open_course"})).await?;
    let invalid = send(
        &mut *courses,
        manager,
        json!({"type":"create_course","course[code]":"invalid code","course[name]":""}),
    )
    .await?;
    prove(
        invalid.html.contains("catalog-errors") && invalid.html.contains("course-modal"),
        "invalid course keeps dialog and field summary",
    )?;
    let description = "Detailed description ".repeat(100);
    let added=send(&mut *courses,manager,json!({"type":"create_course","course[code]":"pt6-page-course","course[name]":"Page catalog proof","course[description]":description})).await?;
    prove(
        added.html.contains("Course added."),
        "course dialog creates through facade",
    )?;
    let course = training
        .courses(manager, COMPANY)
        .await
        .map_err(|_| Error::Internal)?
        .into_iter()
        .find(|c| c.code == "pt6-page-course")
        .ok_or(Error::Internal)?;
    prove(
        course.description.as_deref() == Some(description.as_str()),
        "description longer than 1 KiB is preserved",
    )?;
    let old_version = course.edit_version();
    let saved=send(&mut *courses,manager,json!({"type":"save_course_field","id":course.id.to_string(),"field":"name","value":"Renamed page course","version":old_version})).await?;
    prove(
        saved.html.contains("Saved"),
        "inline name commits with field outcome",
    )?;
    let stale=send(&mut *courses,manager,json!({"type":"save_course_field","id":course.id.to_string(),"field":"name","value":"Stale draft","version":old_version})).await?;
    prove(
        stale.html.contains("This course changed."),
        "stale inline draft is refused under the course lock",
    )?;
    let changed = send(
        &mut *courses,
        manager,
        json!({"type":"set_active","value":format!("{}:false",course.id)}),
    )
    .await?;
    prove(
        changed.html.contains("Course updated.")
            && training
                .courses(manager, COMPANY)
                .await
                .map_err(|_| Error::Internal)?
                .iter()
                .find(|c| c.id == course.id)
                .is_some_and(|c| !c.active),
        "course deactivation follows Bilimbi's one-click workflow",
    )?;
    send(
        &mut *courses,
        manager,
        json!({"type":"set_active","value":format!("{}:false",course.id)}),
    )
    .await?;
    prove(
        training
            .courses(manager, COMPANY)
            .await
            .map_err(|_| Error::Internal)?
            .iter()
            .find(|c| c.id == course.id)
            .is_some_and(|c| !c.active),
        "repeated intent retains its requested status",
    )?;
    send(
        &mut *courses,
        manager,
        json!({"type":"set_active","value":format!("{}:true",course.id)}),
    )
    .await?;
    grants
        .set_direct(admin, MANAGER, COURSES_MANAGE, Some(false))
        .await?;
    let refused = send(
        &mut *courses,
        manager,
        json!({"type":"create_course","course[code]":"revoked","course[name]":"Revoked"}),
    )
    .await?;
    let not_written = training
        .courses(manager, COMPANY)
        .await
        .map_err(|_| Error::Internal)?
        .iter()
        .all(|c| c.code != "revoked");
    prove(
        refused.html.contains("You cannot change this company")
            && refused.html.contains("Read-only.")
            && not_written,
        "revoked grant refuses the next event and keeps the live page",
    )?;
    grants
        .set_direct(admin, MANAGER, COURSES_MANAGE, Some(true))
        .await?;
    for n in 0..28 {
        training
            .create_course(
                manager,
                COMPANY,
                &NewCourse {
                    code: format!("pt6-page-{n:02}"),
                    name: format!("Page proof {n:02}"),
                    description: None,
                },
            )
            .await
            .map_err(|_| Error::Internal)?;
    }
    let second = send(&mut *courses, manager, json!({"type":"page","page":"2"})).await?;
    prove(
        second.query.get("page").map(String::as_str) == Some("2"),
        "course pagination round-trips in the URL",
    )?;
    let readonly = pages
        .0
        .render(PageRequest {
            scope: Some(viewer),
            ..request("courses")
        })
        .await?;
    prove(
        readonly.html.contains("Read-only.")
            && !readonly.html.contains("data-event=\"open_course\""),
        "reader sees catalog with withheld controls",
    )?;
    let event = training
        .create_event(
            manager,
            COMPANY,
            &NewEvent {
                course_id: course.id,
                name: "Calendar page proof".into(),
                capacity: 8,
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    let zone = training
        .company_zone(manager, COMPANY)
        .await
        .map_err(|_| Error::Internal)?;
    let month = jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::get(&zone).map_err(|_| Error::Invalid)?)
        .date()
        .first_of_month();
    training
        .create_session(
            manager,
            COMPANY,
            &NewSession {
                event_id: event.id,
                name: "Midnight boundary proof".into(),
                capacity: 8,
                time_zone: zone,
                starts_local: format!("{month}T23:00"),
                ends_local: format!(
                    "{}T00:00",
                    month
                        .checked_add(jiff::Span::new().days(1))
                        .map_err(|_| Error::Invalid)?
                ),
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    let calendar_query = HashMap::from([
        ("company_id".into(), COMPANY.to_string()),
        ("date".into(), month.to_string()),
        ("view".into(), "calendar".into()),
    ]);
    let mut sessions = pages
        .0
        .mount(PageRequest {
            query: &calendar_query,
            ..request("sessions")
        })
        .await?
        .ok_or(Error::Internal)?;
    let calendar = sessions
        .update(LiveRequest {
            scope: manager,
            credential: "walkthrough",
            input: LiveInput::Refresh,
        })
        .await?;
    prove(
        calendar.html.matches("Midnight boundary proof").count() == 1,
        "exclusive midnight end appears on exactly one calendar day",
    )?;
    prove(
        calendar.query.get("view").map(String::as_str) == Some("calendar")
            && calendar.query.get("date") == Some(&month.to_string()),
        "calendar month and mode survive canonical URL state",
    )?;
    send(&mut *sessions, manager, json!({"type":"open_session"})).await?;
    let excessive=send(&mut *sessions,manager,json!({"type":"create_session","entry[event_id]":event.id.to_string(),"entry[name]":"Too many","entry[capacity]":"9","entry[time_zone]":"UTC","entry[starts_local]":format!("{month}T10:00"),"entry[ends_local]":format!("{month}T11:00")})).await?;
    prove(
        excessive
            .html
            .contains("Session capacity cannot exceed event capacity.")
            && excessive.html.contains("session-modal"),
        "capacity refusal preserves session dialog",
    )?;
    println!(
        "PT6 browser ready: manager can edit catalog and schedule; viewer can read both pages."
    );
    Ok(())
}

/// Run only the Training catalog and its pages on a disposable database.
/// Other slices' date-sensitive proofs must not gate a catalog page check.
async fn standalone(url: &str) -> Result<()> {
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = bilibi_database::pool(url)?;
    let modules = bilibi_module_registry::discover()?;
    bilibi_database::migrate(&db, &modules).await?;
    bilibi_walkthrough_fixture::install(&db).await?;
    let mut services = Services::default();
    services.provide("bootstrap.database", Arc::new(db.clone()))?;
    services.provide("bootstrap.composition", Arc::new(bilibi_web::composition()))?;
    services.install(&modules)?;
    if std::env::var("BILIBI_ENV").as_deref() == Ok("development") {
        services
            .get::<bilibi_base::step_up::StepUp>(bilibi_base::step_up::SERVICE)?
            .set_enforced(false);
    }
    services
        .get::<Grants>("base/authz.grants")?
        .reconcile()
        .await?;
    let client = db.get().await?;
    let auth = services.get::<Authentication>("base/tenancy.authentication")?;
    let admin = auth
        .authenticate(&**client, TENANT, ADMIN, "walkthrough".into())
        .await?;
    let manager = auth
        .authenticate(&**client, TENANT, MANAGER, "walkthrough".into())
        .await?;
    let viewer = auth
        .authenticate(&**client, TENANT, VIEWER, "walkthrough".into())
        .await?;
    crate::training::run(&services, &client, &admin, &manager, &viewer).await?;
    run(&services, &admin, &manager, &viewer).await
}

walkthrough_step!(
    "training_catalog_pages",
    after = ["training_evaluation"],
    |ctx| async move {
        // PT6: catalog page events and browser records, after other slices' grant proofs.
        crate::training_catalog_pages::run(&ctx.services, &ctx.admin, &ctx.manager, &ctx.viewer)
            .await?;
        Ok(())
    }
);

walkthrough_command!("training-catalog-pages", |url, _argument| async move {
    standalone(url).await
});
