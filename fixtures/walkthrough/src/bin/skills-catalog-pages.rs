//! PK4 executable proof on a disposable database, followed by browser use.
//! Run `prepare` once; `prove` exercises the live pages. Grant commands support
//! a second session revoking a permission while a confirmation stays open.
use bilibi_authz::grants::Grants;
use bilibi_database::{Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_skills::{Skills, CATALOG_MANAGE, CATALOG_VIEW, PROFILES_PUBLISH};
use bilibi_tenancy::{Authentication, Scope};
use bilibi_ui::page::{LiveBody, LiveInput, LiveRequest, LiveState, PageRequest, PageService};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc};

fn check(value: bool, label: &str) -> Result<()> {
    if !value {
        eprintln!("PK4 FAILED: {label}");
        return Err(Error::Internal);
    }
    println!("PK4 proved: {label}");
    Ok(())
}
fn saved<T>(result: bilibi_people_skills::Result<T>) -> Result<T> {
    result.map_err(|e| {
        eprintln!("PK4 unexpected: {e:?}");
        Error::Internal
    })
}
async fn event(
    page: &mut Box<dyn LiveState<Scope, Error>>,
    scope: &Scope,
    value: Value,
) -> Result<LiveBody> {
    page.update(LiveRequest {
        scope,
        credential: "walkthrough",
        input: LiveInput::Event(&value),
    })
    .await
}
async fn mount(
    services: &Services,
    scope: &Scope,
    operation: &'static str,
    profile: Option<i64>,
    company: i64,
) -> Result<Box<dyn LiveState<Scope, Error>>> {
    let pages = services.get::<PageService<Scope, Error>>("people/skills.catalog-profile-pages")?;
    let query = HashMap::from([("company_id".into(), company.to_string())]);
    let path = profile
        .map(|id| HashMap::from([("id".into(), id.to_string())]))
        .unwrap_or_default();
    let mut page = pages
        .0
        .mount(PageRequest {
            operation,
            scope: Some(scope),
            path: &path,
            query: &query,
            body: &[],
            framed: false,
        })
        .await?
        .ok_or(Error::Internal)?;
    page.update(LiveRequest {
        scope,
        credential: "walkthrough",
        input: LiveInput::Refresh,
    })
    .await?;
    Ok(page)
}
#[tokio::main]
async fn main() -> Result<()> {
    let url = std::env::var("DATABASE_URL").map_err(|_| Error::Invalid)?;
    let command = std::env::args().nth(1).unwrap_or("prove".into());
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = bilibi_database::pool(&url)?;
    let modules = bilibi_module_registry::discover()?;
    if command == "prepare" {
        bilibi_database::migrate(&db, &modules).await?;
        bilibi_walkthrough_fixture::install(&db).await?;
    }
    let mut services = Services::default();
    services.provide("bootstrap.database", Arc::new(db.clone()))?;
    services.provide("bootstrap.composition", Arc::new(bilibi_web::composition()))?;
    services.install(&modules)?;
    if std::env::var("BILIBI_ENV").as_deref() == Ok("development") {
        services
            .get::<bilibi_base::step_up::StepUp>(bilibi_base::step_up::SERVICE)?
            .set_enforced(false);
    }
    let client = db.get().await?;
    let auth = services.get::<Authentication>("base/tenancy.authentication")?;
    let admin = auth
        .authenticate(&**client, 1, 4, "walkthrough".into())
        .await?;
    let manager = auth
        .authenticate(&**client, 1, 1, "walkthrough".into())
        .await?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    if command == "revoke-manage"
        || command == "restore-manage"
        || command == "revoke-publish"
        || command == "restore-publish"
    {
        let cap = if command.ends_with("manage") {
            CATALOG_MANAGE
        } else {
            PROFILES_PUBLISH
        };
        grants
            .set_direct(&admin, 1, cap, Some(command.starts_with("restore")))
            .await?;
        println!("PK4 {command}");
        return Ok(());
    }
    for cap in [CATALOG_VIEW, CATALOG_MANAGE, PROFILES_PUBLISH] {
        grants.set_direct(&admin, 1, cap, Some(true)).await?;
    }
    if command == "prepare" {
        println!("PK4 prepared: manager can use the Skills catalog and publish profiles; no Skills records added.");
        return Ok(());
    }
    let skills = services.get::<Skills>(bilibi_people_skills::SERVICE)?;
    let mut catalog = mount(&services, &manager, "catalog", None, 1).await?;
    let bad=event(&mut catalog,&manager,json!({"type":"create_category","category[code]":"UPPER","category[name]":"Keep this draft"})).await?;
    check(
        bad.html.contains("Keep this draft") && bad.html.contains("skill-category-form-errors"),
        "invalid category retains values and shows field errors",
    )?;
    let h = event(
        &mut catalog,
        &manager,
        json!({"type":"create_category","category[code]":"pk4","category[name]":"PK4 category"}),
    )
    .await?;
    check(
        h.html.contains("Category added."),
        "category created through live form",
    )?;
    let category = saved(skills.list_categories(&manager, 1).await)?
        .into_iter()
        .find(|c| c.code == "pk4")
        .ok_or(Error::Internal)?;
    event(&mut catalog,&manager,json!({"type":"create_skill","skill[category_id]":category.id.to_string(),"skill[code]":"pk4","skill[name]":"PK4 skill","skill[definition]":"Observable work"})).await?;
    let skill = saved(skills.list_skills(&manager, 1).await)?
        .into_iter()
        .find(|s| s.code == "pk4")
        .ok_or(Error::Internal)?;
    let h = event(
        &mut catalog,
        &manager,
        json!({"type":"set_category_active","id":category.id.to_string()}),
    )
    .await?;
    check(
        h.html.contains("Deactivate the category"),
        "category with active skills refuses deactivation",
    )?;
    event(
        &mut catalog,
        &manager,
        json!({"type":"create_scale","scale[code]":"pk4","scale[name]":"PK4 scale"}),
    )
    .await?;
    let scale = saved(skills.list_scales(&manager, 1).await)?
        .into_iter()
        .find(|s| s.code == "pk4")
        .ok_or(Error::Internal)?;
    let h = event(
        &mut catalog,
        &manager,
        json!({"type":"scale_publish","id":scale.id.to_string()}),
    )
    .await?;
    check(
        h.html.contains("skills-catalog-confirm"),
        "scale publication opens confirmation",
    )?;
    event(&mut catalog, &manager, json!({"type":"confirm"})).await?;
    check(
        saved(skills.list_scales(&manager, 1).await)?
            .iter()
            .find(|s| s.id == scale.id)
            .is_some_and(|s| s.status == "draft"),
        "invalid scale stays draft",
    )?;
    for (n, name) in [(0, "Learning"), (1, "Independent")] {
        event(&mut catalog,&manager,json!({"type":"put_level","level[scale_id]":scale.id.to_string(),"level[level]":n.to_string(),"level[name]":name,"level[anchor]":"Observable behaviour","level[authority]":"Recorded authority"})).await?;
    }
    event(
        &mut catalog,
        &manager,
        json!({"type":"scale_publish","id":scale.id.to_string()}),
    )
    .await?;
    let h = event(&mut catalog, &manager, json!({"type":"confirm"})).await?;
    check(
        h.html.contains("Scale published."),
        "valid scale publishes after confirmation",
    )?;
    event(&mut catalog,&manager,json!({"type":"create_profile","profile[code]":"pk4","profile[name]":"PK4 profile","profile[scale_id]":scale.id.to_string()})).await?;
    let profile = saved(skills.list_profiles(&manager, 1).await)?
        .into_iter()
        .find(|p| p.code == "pk4")
        .ok_or(Error::Internal)?;
    let mut page = mount(&services, &manager, "profile", Some(profile.id), 1).await?;
    event(&mut page,&manager,json!({"type":"put_item","item[skill_id]":skill.id.to_string(),"item[required_level]":"1","item[criticality]":"essential","item[weight_percent]":"60","item[mandatory]":"true"})).await?;
    event(
        &mut page,
        &manager,
        json!({"type":"add_selector","selector[target]":"company"}),
    )
    .await?;
    event(
        &mut page,
        &manager,
        json!({"type":"publish","effective_from":"2026-01-01"}),
    )
    .await?;
    let h = event(&mut page, &manager, json!({"type":"confirm"})).await?;
    check(
        h.html.contains("Requirement weights must total 100."),
        "publication refuses weights other than 100",
    )?;
    event(&mut page,&manager,json!({"type":"put_item","item[skill_id]":skill.id.to_string(),"item[required_level]":"1","item[criticality]":"essential","item[weight_percent]":"100","item[mandatory]":"true"})).await?;
    event(
        &mut page,
        &manager,
        json!({"type":"publish","effective_from":"2026-01-01"}),
    )
    .await?;
    grants
        .set_direct(&admin, 1, PROFILES_PUBLISH, Some(false))
        .await?;
    let h = event(&mut page, &manager, json!({"type":"confirm"})).await?;
    check(
        h.html.contains("You cannot publish or retire"),
        "revoked publisher refuses confirmation in an already mounted page",
    )?;
    check(
        saved(skills.get_profile(&manager, 1, profile.id).await)?
            .profile
            .status
            == "draft",
        "revocation leaves profile draft",
    )?;
    grants
        .set_direct(&admin, 1, PROFILES_PUBLISH, Some(true))
        .await?;
    event(
        &mut page,
        &manager,
        json!({"type":"publish","effective_from":"2026-01-01"}),
    )
    .await?;
    event(
        &mut page,
        &manager,
        json!({"type":"confirm","effective_from":"2030-01-01","company_id":"100","id":"99999"}),
    )
    .await?;
    let stored = saved(skills.get_profile(&manager, 1, profile.id).await)?;
    check(
        stored.profile.status == "published"
            && stored.profile.effective_from.map(|d| d.to_string()) == Some("2026-01-01".into()),
        "confirmation ignores client replacement dates, record and company",
    )?;
    let sql=client.execute("UPDATE people_skill_profile_items SET required_level=0 WHERE tenant_id=1 AND company_id=1 AND profile_id=$1",&[&profile.id]).await;
    check(
        sql.is_err(),
        "PostgreSQL refuses editing published requirements",
    )?;
    event(&mut page, &manager, json!({"type":"new_version"})).await?;
    let h = event(&mut page, &manager, json!({"type":"confirm"})).await?;
    check(
        h.path
            .as_deref()
            .is_some_and(|p| p.starts_with("/people/skills/profiles/"))
            && h.html.contains("New draft version added."),
        "new version opens copied draft with its canonical path",
    )?;
    event(&mut page, &manager, json!({"type":"discard"})).await?;
    let h = event(&mut page, &manager, json!({"type":"confirm"})).await?;
    check(
        h.redirect.as_deref() == Some("/people/skills?company_id=1") && h.flash.is_some(),
        "discard redirects with success flash",
    )?;
    let mut page = mount(&services, &manager, "profile", Some(profile.id), 100).await?;
    let h = event(
        &mut page,
        &manager,
        json!({"type":"publish","effective_from":"2026-01-01"}),
    )
    .await?;
    check(
        h.html
            .contains("This requirement profile is not available."),
        "foreign company cannot expose profile",
    )?;
    grants
        .set_direct(&admin, 1, CATALOG_MANAGE, Some(false))
        .await?;
    let h=event(&mut catalog,&manager,json!({"type":"create_category","category[code]":"revoked","category[name]":"Never created"})).await?;
    check(
        h.html.contains("You cannot change this company")
            && !h.html.contains("id=\"skill-category-form\""),
        "revoked manager sees read-only catalog and write refusal",
    )?;
    grants
        .set_direct(&admin, 1, CATALOG_MANAGE, Some(true))
        .await?;
    println!("PK4 page proof complete; the manager may now use catalog records in the browser.");
    Ok(())
}
