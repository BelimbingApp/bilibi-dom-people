//! PV2: run after the default, payroll and payroll_calculation walkthroughs.
//! Real page commands, operator jobs and optional Finance event delivery.
use bilibi_authz::grants::Grants;
use bilibi_database::{pool, Error, Result};
use bilibi_module_registry::{
    contributions::{Route, ROUTES},
    Catalog, Services,
};
use bilibi_queue::execution::{Config, Executor};
use bilibi_tenancy::{Authentication, Scope};
use bilibi_ui::{
    download::{DownloadRequest, DownloadService},
    page::{LiveInput, LiveRequest, LiveState, PageRequest, PageService},
};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc, time::Duration};

fn prove(label: &str, condition: bool) -> Result<()> {
    if !condition {
        eprintln!("PV2 failed: {label}");
        return Err(Error::Internal);
    }
    println!("PV2 proved: {label}");
    Ok(())
}

async fn update(
    page: &mut dyn LiveState<Scope, Error>,
    scope: &Scope,
    event: Value,
) -> Result<String> {
    Ok(page
        .update(LiveRequest {
            scope,
            credential: "PV2 disposable proof",
            input: LiveInput::Event(&event),
        })
        .await?
        .html)
}

#[tokio::main]
async fn main() -> Result<()> {
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = pool(&std::env::var("DATABASE_URL").map_err(|_| Error::Invalid)?)?;
    let modules = bilibi_module_registry::discover()?;
    let catalog = Catalog::discover(&modules)?;
    let mut services = Services::default();
    services.provide("bootstrap.database", Arc::new(db.clone()))?;
    services.provide("bootstrap.composition", Arc::new(bilibi_web::composition()))?;
    services.install(&modules)?;
    services
        .get::<bilibi_base::step_up::StepUp>(bilibi_base::step_up::SERVICE)?
        .set_enforced(false);
    let client = db.get().await?;
    prove(
        "only an installed disposable walkthrough is used",
        client
            .query_opt(
                "SELECT id FROM users WHERE tenant_id=1 AND id=1 AND email='manager@example.test'",
                &[],
            )
            .await?
            .is_some(),
    )?;
    let auth = services.get::<Authentication>("base/tenancy.authentication")?;
    let admin = auth.authenticate(&**client, 1, 4, "PV2".into()).await?;
    let manager = auth.authenticate(&**client, 1, 1, "PV2".into()).await?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    for key in grants.vocabulary().capabilities.keys() {
        if key.starts_with("people.") {
            grants.set_direct(&admin, 1, key, Some(true)).await?;
        }
    }
    let profile: i64 = client
        .query_one("SELECT id FROM people_skill_profiles WHERE tenant_id=1 AND company_id=1 ORDER BY id LIMIT 1", &[])
        .await?
        .get(0);
    let query = HashMap::from([("company_id".into(), "1".into())]);
    let mut worker_jobs = Vec::new();
    for entry in catalog.entries::<Route>(ROUTES)? {
        let route = entry.get::<Route>()?;
        if !(route.path.starts_with("/people/training/")
            || route.path.starts_with("/people/skills")
            || route.path.starts_with("/people/payroll/")
            || route.path.starts_with("/people/performance")
            || route.path.starts_with("/people/progression")
            || matches!(
                route.path,
                "/people/attendance/my"
                    | "/people/leave/my"
                    | "/people/leave/policies"
                    | "/people/claims"
            ))
        {
            continue;
        }
        let path = HashMap::from([("id".into(), profile.to_string())]);
        let pages = services.get::<PageService<Scope, Error>>(route.service)?;
        let mut page = pages
            .0
            .mount(PageRequest {
                operation: route.operation,
                scope: Some(&manager),
                path: &path,
                query: &query,
                body: &[],
                framed: false,
            })
            .await?
            .ok_or(Error::Internal)?;
        let html = page
            .update(LiveRequest {
                scope: &manager,
                credential: "PV2",
                input: LiveInput::Refresh,
            })
            .await?
            .html;
        prove(
            &format!("{} mounts with real records", route.path),
            !html.is_empty(),
        )?;
        let command = match route.path {
            "/people/leave/policies" => Some((
                "carry_forward",
                "confirm_action",
                "people-leave/carry-forward",
            )),
            "/people/skills/policy" => {
                Some(("run_reminders", "confirm", "people-skills/reminders"))
            }
            _ => None,
        };
        if let Some((event, confirm, worker)) = command {
            let year = jiff::Timestamp::now()
                .to_zoned(jiff::tz::TimeZone::UTC)
                .year()
                - 1;
            let before: i64 = client
                .query_one(
                    "SELECT coalesce(max(id),0) FROM base_queue_jobs WHERE tenant_id=1",
                    &[],
                )
                .await?
                .get(0);
            update(
                &mut *page,
                &manager,
                json!({"type":event,"from_year":year.to_string()}),
            )
            .await?;
            let html = update(&mut *page, &manager, json!({"type":confirm})).await?;
            prove(
                &format!("{worker} page reports queued"),
                html.contains("queued"),
            )?;
            let job = client.query_one("SELECT id,actor_id,worker_id FROM base_queue_jobs WHERE tenant_id=1 AND id>$1 ORDER BY id DESC LIMIT 1", &[&before]).await?;
            prove(
                &format!("{worker} retains the enqueuing account"),
                job.get::<_, i64>(1) == 1 && job.get::<_, String>(2) == worker,
            )?;
            worker_jobs.push(job.get::<_, i64>(0));
        }
    }
    prove(
        "both workers were queued through their pages",
        worker_jobs.len() == 2,
    )?;

    // Export through the registered transport, then revoke access with the
    // same old scope. Download links never preserve an obsolete grant.
    let batch: i64 = client.query_one("SELECT id FROM people_claim_handoff_batches WHERE tenant_id=1 AND company_id=1 ORDER BY id LIMIT 1", &[]).await?.get(0);
    let files = services.get::<DownloadService<Scope, Error>>("people/claims.downloads")?;
    let path = HashMap::from([
        ("company_id".into(), "1".into()),
        ("id".into(), batch.to_string()),
    ]);
    let download = || DownloadRequest {
        operation: "handoff_export",
        scope: &manager,
        path: &path,
        query: &query,
    };
    let csv = files.0.serve(download()).await?;
    prove(
        "CSV transport uses private attachment headers",
        csv.headers()
            .contains(&("cache-control", "private, no-store".into()))
            && csv
                .headers()
                .contains(&("x-content-type-options", "nosniff".into()))
            && !csv.bytes.is_empty(),
    )?;
    grants
        .set_direct(&admin, 1, bilibi_people_claims::REIMBURSE, Some(false))
        .await?;
    let refused = files.0.serve(download()).await;
    grants
        .set_direct(&admin, 1, bilibi_people_claims::REIMBURSE, Some(true))
        .await?;
    prove(
        "CSV refuses a grant revoked after its link was rendered",
        matches!(refused, Err(Error::Forbidden)),
    )?;

    let finance = modules.iter().any(|m| m.id == "finance/payroll");
    if finance {
        grants
            .set_system(
                &admin,
                "domain-events",
                "events.delivery.deliver",
                Some(true),
            )
            .await?;
    }
    drop(client);
    let executor = Executor::from_services(
        &services,
        Config {
            concurrency: 2,
            poll_interval: Duration::from_millis(100),
            lease_seconds: 30,
            max_runtime: Duration::from_secs(60),
        },
    )?;
    let (stop, shutdown) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(executor.run(shutdown));
    let completed = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let c = db.get().await?;
            let jobs: i64 = c.query_one("SELECT count(*) FROM base_queue_jobs WHERE tenant_id=1 AND id=ANY($1) AND state='completed'", &[&worker_jobs]).await?.get(0);
            let pending: i64 = if finance {
                c.query_one("SELECT count(*) FROM base_events_deliveries d JOIN base_events e ON e.id=d.event_id WHERE e.tenant_id=1 AND d.subscriber='finance/payroll.people' AND d.delivered_at IS NULL", &[]).await?.get(0)
            } else { 0 };
            if jobs == 2 && pending == 0 { return Ok::<_,Error>(()); }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }).await;
    let _ = stop.send(true);
    task.await.map_err(|_| Error::Internal)??;
    completed.map_err(|_| Error::Internal)??;
    prove("both page-queued workers completed as their operator", true)?;
    if finance {
        let c = db.get().await?;
        for name in [
            "people/payroll.run_approved",
            "people/claims.request_approved",
            "people/claims.request_reimbursed",
        ] {
            let delivered: i64 = c.query_one("SELECT count(*) FROM base_events e JOIN base_events_deliveries d ON d.event_id=e.id WHERE e.tenant_id=1 AND e.name=$1 AND e.version=1 AND d.subscriber='finance/payroll.people' AND d.delivered_at IS NOT NULL", &[&name]).await?.get(0);
            prove(
                &format!("{name} outbox committed and delivered to Finance"),
                delivered > 0,
            )?;
        }
    } else {
        println!("PV2: Finance is absent; mount it and repeat for delivery proof.");
    }
    Ok(())
}
