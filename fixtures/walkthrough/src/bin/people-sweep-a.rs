//! PV1: run after the default People walkthrough on a disposable database.
//! Uses mounted pages, current grants and PostgreSQL guards, never product seeds.
use bilibi_authz::grants::Grants;
use bilibi_database::{pool, Error, Result};
use bilibi_module_registry::Services;
use bilibi_tenancy::{Authentication, Scope};
use bilibi_ui::page::{LiveInput, LiveRequest, PageRequest, PageService};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc};

fn prove(ok: bool, label: &str) -> Result<()> {
    if !ok {
        eprintln!("PV1 FAILED: {label}");
        return Err(Error::Internal);
    }
    println!("PV1 proved: {label}");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = pool(&std::env::var("DATABASE_URL").map_err(|_| Error::Invalid)?)?;
    let mut client = db.get().await?;
    prove(
        client
            .query_opt(
                "SELECT 1 FROM users WHERE tenant_id=1 AND id=1 AND email='manager@example.test'",
                &[],
            )
            .await?
            .is_some(),
        "disposable walkthrough is installed",
    )?;
    let modules = bilibi_module_registry::discover()?;
    let mut services = Services::default();
    services.provide("bootstrap.database", Arc::new(db.clone()))?;
    services.provide("bootstrap.composition", Arc::new(bilibi_web::composition()))?;
    services.install(&modules)?;
    services
        .get::<bilibi_base::step_up::StepUp>(bilibi_base::step_up::SERVICE)?
        .set_enforced(false);
    let auth = services.get::<Authentication>("base/tenancy.authentication")?;
    let admin = auth
        .authenticate(&**client, 1, 4, "PV1 proof".into())
        .await?;
    let manager = auth
        .authenticate(&**client, 1, 1, "PV1 proof".into())
        .await?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    let cases: &[(&str, &str, &str, Value)] = &[
        (
            "people/employee_workspace.pages",
            "employees",
            "people.workforce.employment.manage",
            json!({"type":"create_employment"}),
        ),
        (
            "people/employee_workspace.pages",
            "employee",
            "people.employees.manage",
            json!({"type":"request_change"}),
        ),
        (
            "people/workforce.pages",
            "settings",
            "people.workforce.settings.manage",
            json!({"type":"save_statuses","statuses[active]":"true"}),
        ),
        (
            "people/reference_data.pages",
            "references",
            "people.references.manage",
            json!({"type":"create_entry"}),
        ),
        (
            "people/organisation.pages",
            "explorer",
            "people.organisation.manage",
            json!({"type":"assign","position_id":"1"}),
        ),
        (
            "people/attendance.settings_pages",
            "rules",
            "people.attendance.rules.manage",
            json!({"type":"save_rules"}),
        ),
        (
            "people/attendance.settings_pages",
            "shifts",
            "people.attendance.rules.manage",
            json!({"type":"create"}),
        ),
        (
            "people/attendance.settings_pages",
            "locations",
            "people.attendance.rules.manage",
            json!({"type":"create"}),
        ),
        (
            "people/attendance.settings_pages",
            "allowances",
            "people.attendance.allowances.manage",
            json!({"type":"create"}),
        ),
        (
            "people/attendance.pages",
            "rosters",
            "people.attendance.roster.manage",
            json!({"type":"publish"}),
        ),
        (
            "people/attendance.pages",
            "approvals",
            "people.attendance.adjustments.approve",
            json!({"type":"approve","request_id":"1"}),
        ),
        (
            "people/leave.pages",
            "requests",
            "people.leave.requests.approve",
            json!({"type":"approve","request_id":"1"}),
        ),
        (
            "people/leave.policy_pages",
            "policies",
            "people.leave.policies.manage",
            json!({"type":"create_type"}),
        ),
        (
            "people/claims.setup_pages",
            "setup",
            "people.claims.manage",
            json!({"type":"create_category"}),
        ),
        (
            "people/claims.pages",
            "list",
            "people.claims.approve",
            json!({"type":"approve","id":"1"}),
        ),
    ];
    for key in [
        bilibi_company::TENANT_WIDE,
        "people.organisation.view",
        "people.employees.view",
        "people.workforce.employment.view",
        "people.employees.manage",
        "people.employees.review",
    ] {
        grants.set_direct(&admin, 1, key, Some(true)).await?;
    }
    for (_, _, key, _) in cases {
        grants.set_direct(&admin, 1, key, Some(true)).await?;
    }
    if std::env::args().nth(1).as_deref() == Some("queued-grant") {
        drop(client);
        return revoke_queued_settings(&db, &services, &admin, &manager).await;
    }
    let query = HashMap::from([("company_id".into(), "1".into())]);
    let mut path = query.clone();
    path.insert("id".into(), "1".into());
    for (service, operation, capability, payload) in cases {
        let pages = services.get::<PageService<Scope, Error>>(service)?;
        let mut state = pages
            .0
            .mount(PageRequest {
                operation,
                scope: Some(&manager),
                path: &path,
                query: &query,
                body: &[],
                framed: false,
            })
            .await?
            .ok_or(Error::Internal)?;
        let initial = state
            .update(LiveRequest {
                scope: &manager,
                credential: "PV1 proof",
                input: LiveInput::Refresh,
            })
            .await?;
        prove(
            !initial.html.is_empty(),
            &format!("{operation} mounts with live records"),
        )?;
        if *operation == "employees" {
            let open = json!({"type":"open_add"});
            let body = state
                .update(LiveRequest {
                    scope: &manager,
                    credential: "PV1 proof",
                    input: LiveInput::Event(&open),
                })
                .await?;
            prove(
                body.html.contains("employment-person-picker"),
                "employment dialog opens before its grant is revoked",
            )?;
        }
        grants
            .set_direct(&admin, 1, capability, Some(false))
            .await?;
        let outcome = state
            .update(LiveRequest {
                scope: &manager,
                credential: "PV1 proof",
                input: LiveInput::Event(payload),
            })
            .await;
        // Restore even when a proof fails, leaving the browser records usable.
        grants.set_direct(&admin, 1, capability, Some(true)).await?;
        let body = outcome?;
        let refusal = match *operation {
            "employees" => "You cannot add an employment for this company.",
            "employee" => "You no longer have permission to change this employee",
            "settings" => "Working statuses could not be saved.",
            "references" => "You cannot change this company",
            "explorer" => "You no longer have permission to change this company",
            "rules" | "shifts" | "locations" | "allowances" | "policies" | "setup" => {
                "You no longer have permission to change this company"
            }
            "rosters" => "You no longer have permission to plan rosters for this company.",
            "approvals" => {
                "You no longer have permission to decide attendance requests for this company."
            }
            "requests" => {
                "You no longer have permission to decide leave requests for this company."
            }
            "operations" => "This claim is no longer awaiting a decision.",
            _ => return Err(Error::Invalid),
        };
        prove(
            body.html.contains(refusal),
            &format!("{operation} revoked mid-page refuses without a fatal live error"),
        )?;
        let body = state
            .update(LiveRequest {
                scope: &manager,
                credential: "PV1 proof",
                input: LiveInput::Refresh,
            })
            .await?;
        prove(
            !body.html.is_empty(),
            &format!("{operation} resumes after grant restoration"),
        )?;
    }
    // Roll back even if a guard is missing. Each probe requires an actual row
    // and a specific SQLSTATE, so missing fixtures cannot silently pass.
    for (label, table, predicate, sql, code) in [
        ("leave ledger update", "people_leave_ledger_entries", "tenant_id=1 AND company_id=1", "UPDATE people_leave_ledger_entries SET quantity=quantity WHERE id=$1", "P0001"),
        ("leave ledger delete", "people_leave_ledger_entries", "tenant_id=1 AND company_id=1", "DELETE FROM people_leave_ledger_entries WHERE id=$1", "P0001"),
        ("leave event update", "people_leave_request_events", "tenant_id=1 AND company_id=1", "UPDATE people_leave_request_events SET id=id WHERE id=$1", "P0001"),
        ("claim event delete", "people_claim_request_events", "tenant_id=1 AND company_id=1", "DELETE FROM people_claim_request_events WHERE id=$1", "P0001"),
        ("published scale update", "people_skill_scales", "company_id=1 AND status='published'", "UPDATE people_skill_scales SET name=name WHERE id=$1", "P0001"),
        ("published profile history update", "people_skill_profiles", "company_id=1 AND status IN ('published','retired')", "UPDATE people_skill_profiles SET name=name WHERE id=$1", "P0001"),
        ("overlapping substantive assignments", "people_position_placements", "kind='substantive' AND position_id IN (SELECT id FROM people_positions WHERE company_id=1)", "INSERT INTO people_position_placements(position_id,employee_id,kind,effective_from,effective_to) SELECT position_id,employee_id,kind,effective_from,effective_to FROM people_position_placements WHERE id=$1", "23P01"),
    ] {
        // Identifiers and predicates are compile-time fixture literals above.
        let id: i64 = client.query_one(&format!("SELECT id FROM {table} WHERE {predicate} ORDER BY id LIMIT 1"), &[]).await?.get(0);
        let tx = client.transaction().await?;
        let result = tx.execute(sql, &[&id]).await;
        tx.rollback().await?;
        prove(result.as_ref().err().and_then(|e| e.as_db_error()).is_some_and(|e| e.code().code()==code), &format!("database refuses {label} with {code}"))?;
    }
    let slot: i64 = client.query_one("SELECT id FROM people_leave_application_dates WHERE tenant_id=1 AND company_id=1 AND active ORDER BY id LIMIT 1", &[]).await?.get(0);
    let tx = client.transaction().await?;
    // A different request avoids mistaking the per-request date uniqueness
    // check for the employee's cross-request half-day reservation guard.
    let request: i64 = tx.query_one("INSERT INTO people_leave_applications(tenant_id,company_id,employee_id,leave_type_id,leave_year,starts_on,ends_on,day_part,quantity,unit,status,request_key,requested_by_user_id) SELECT tenant_id,company_id,employee_id,leave_type_id,leave_year,starts_on,ends_on,day_part,quantity,unit,'pending','pv1-slot-guard',requested_by_user_id FROM people_leave_applications WHERE id=(SELECT request_id FROM people_leave_application_dates WHERE id=$1) RETURNING id", &[&slot]).await?.get(0);
    let result = tx.execute("INSERT INTO people_leave_application_dates(tenant_id,company_id,employee_id,request_id,on_date,am,pm,quantity,active) SELECT tenant_id,company_id,employee_id,$2,on_date,am,pm,quantity,true FROM people_leave_application_dates WHERE id=$1", &[&slot, &request]).await;
    tx.rollback().await?;
    prove(
        result
            .as_ref()
            .err()
            .and_then(|e| e.as_db_error())
            .is_some_and(|e| {
                e.code().code() == "23505"
                    && matches!(
                        e.constraint(),
                        Some(
                            "people_leave_application_dates_am_unique"
                                | "people_leave_application_dates_pm_unique"
                        )
                    )
            }),
        "overlapping leave slots refuse on the employee half-day index",
    )?;
    drop(client);
    revoke_queued_settings(&db, &services, &admin, &manager).await?;
    queued_owner_writes(&db, &services, &admin, &manager).await?;
    archive_queued_write(&db, &services, &admin).await?;
    println!("PV1 sweep A complete; use the same database for Chrome captures.");
    Ok(())
}

async fn wait_for_blocked(db: &bilibi_database::Db, count: i64) -> Result<()> {
    let client = db.get().await?;
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let waiting: i64 = client.query_one("SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND cardinality(pg_blocking_pids(pid))>0", &[]).await?.get(0);
            if waiting >= count {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }).await.map_err(|_| Error::Internal)?
}

async fn revoke_queued_settings(
    db: &bilibi_database::Db,
    services: &Services,
    admin: &Scope,
    manager: &Scope,
) -> Result<()> {
    use bilibi_people_workforce::{Workforce, WorkforceError, SETTINGS_MANAGE};
    let workforce = services.get::<Workforce>(bilibi_people_workforce::SERVICE)?;
    let previous = workforce
        .working_statuses(admin, 1)
        .await
        .map_err(|_| Error::Internal)?
        .value
        .ok_or(Error::Internal)?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    let mutations = services.get::<bilibi_database::MutationBoundary>("base/database.mutations")?;
    let mut client = db.get().await?;
    let mut tx = mutations
        .begin(
            &mut client,
            admin.mutation_context("walkthrough.pv1.queued_settings"),
        )
        .await?;
    bilibi_company::require_writable(tx.sql(), admin, 1, true)
        .await
        .map_err(|_| Error::Internal)?;
    grants
        .set_direct_in(&mut tx, admin, 1, SETTINGS_MANAGE, Some(false))
        .await?;
    let writer = workforce.clone();
    let scope = manager.clone();
    let write = tokio::spawn(async move {
        writer
            .put_working_statuses(&scope, 1, &["terminated".into()])
            .await
    });
    if let Err(error) = wait_for_blocked(db, 1).await {
        write.abort();
        tx.rollback().await?;
        return Err(error);
    }
    tx.commit().await?;
    let outcome = write.await.map_err(|_| Error::Internal)?;
    // Restore before reporting a missing guard so Chrome still has its setup.
    grants
        .set_direct(admin, 1, SETTINGS_MANAGE, Some(true))
        .await?;
    workforce
        .put_working_statuses(admin, 1, &previous)
        .await
        .map_err(|_| Error::Internal)?;
    println!("PV1 queued settings outcome: {outcome:?}");
    prove(
        matches!(outcome, Err(WorkforceError::Unauthorized)),
        "settings write waiting on the company lock observes a committed grant revocation",
    )
}

async fn archive_queued_write(
    db: &bilibi_database::Db,
    services: &Services,
    admin: &Scope,
) -> Result<()> {
    // Company 062 belongs to the Platform's disposable fixture. Hold only a
    // read lock here; Core owns the actual archive and its audit mutation.
    let mut client = db.get().await?;
    let tx = client.transaction().await?;
    let row = tx
        .query_one(
            "SELECT status FROM companies WHERE tenant_id=$1 AND id=62 FOR UPDATE",
            &[&admin.tenant()],
        )
        .await?;
    prove(
        row.get::<_, String>(0) != "archived",
        "archive proof company starts writable",
    )?;
    let companies = services.get::<bilibi_company::CompanyService>(bilibi_company::SERVICE)?;
    let archive_scope = admin.clone();
    let archive = tokio::spawn(async move {
        companies
            .archive_company(&archive_scope, 62, Some("PV1 queued-write proof"))
            .await
    });
    if let Err(error) = wait_for_blocked(db, 1).await {
        archive.abort();
        tx.rollback().await?;
        return Err(error);
    }
    // PostgreSQL queues the company's archive before the People writer.
    let workforce =
        services.get::<bilibi_people_workforce::Workforce>(bilibi_people_workforce::SERVICE)?;
    let write_scope = admin.clone();
    let write = tokio::spawn(async move {
        workforce
            .put_working_statuses(&write_scope, 62, &["active".into()])
            .await
    });
    if let Err(error) = wait_for_blocked(db, 2).await {
        archive.abort();
        write.abort();
        tx.rollback().await?;
        return Err(error);
    }
    tx.commit().await?;
    let archived = archive.await.map_err(|_| Error::Internal)?;
    prove(
        archived.is_ok(),
        "Core archives the company ahead of the waiting write",
    )?;
    let outcome = write.await.map_err(|_| Error::Internal)?;
    prove(
        matches!(
            outcome,
            Err(bilibi_people_workforce::WorkforceError::CompanyArchived)
        ),
        "People write queued before archival refuses after obtaining the company lock",
    )
}

async fn queued_refusal(
    db: &bilibi_database::Db,
    services: &Services,
    admin: &Scope,
    capability: &'static str,
    write: impl std::future::Future<Output = bool> + Send + 'static,
) -> Result<()> {
    let grants = services.get::<Grants>("base/authz.grants")?;
    grants.set_direct(admin, 1, capability, Some(true)).await?;
    let mutations = services.get::<bilibi_database::MutationBoundary>("base/database.mutations")?;
    let mut client = db.get().await?;
    let mut tx = mutations
        .begin(
            &mut client,
            admin.mutation_context("walkthrough.pv1.queued_owner"),
        )
        .await?;
    bilibi_company::require_writable(tx.sql(), admin, 1, true)
        .await
        .map_err(|_| Error::Internal)?;
    grants
        .set_direct_in(&mut tx, admin, 1, capability, Some(false))
        .await?;
    let task = tokio::spawn(write);
    if let Err(error) = wait_for_blocked(db, 1).await {
        task.abort();
        tx.rollback().await?;
        return Err(error);
    }
    tx.commit().await?;
    let refused = task.await.map_err(|_| Error::Internal)?;
    grants.set_direct(admin, 1, capability, Some(true)).await?;
    prove(
        refused,
        &format!("queued {capability} write observes the committed revocation"),
    )
}

async fn queued_owner_writes(
    db: &bilibi_database::Db,
    services: &Services,
    admin: &Scope,
    manager: &Scope,
) -> Result<()> {
    let references = services.get::<bilibi_people_reference_data::ReferenceData>(
        bilibi_people_reference_data::SERVICE,
    )?;
    let scope = manager.clone();
    queued_refusal(
        db,
        services,
        admin,
        bilibi_people_reference_data::MANAGE,
        async move {
            matches!(
                references
                    .create_entry(
                        &scope,
                        1,
                        &bilibi_people_reference_data::NewEntry {
                            kind: "employment_type".into(),
                            code: "pv1-queued".into(),
                            label: "Queued reference must refuse".into(),
                            active: true,
                        }
                    )
                    .await,
                Err(bilibi_people_workforce::WorkforceError::Unauthorized)
            )
        },
    )
    .await?;
    let leave = services.get::<bilibi_people_leave::Leave>(bilibi_people_leave::SERVICE)?;
    let scope = manager.clone();
    queued_refusal(
        db,
        services,
        admin,
        bilibi_people_leave::POLICIES_MANAGE,
        async move {
            matches!(
                leave
                    .create_type(
                        &scope,
                        1,
                        &bilibi_people_leave::NewLeaveType::new(
                            "pv1-queued",
                            "Queued type must refuse",
                            "day"
                        )
                    )
                    .await,
                Err(bilibi_people_leave::LeaveError::Unauthorized)
            )
        },
    )
    .await?;
    let workspace = services.get::<bilibi_people_employee_workspace::EmployeeWorkspace>(
        bilibi_people_employee_workspace::SERVICE,
    )?;
    let profile = workspace
        .work_profile_record(admin, 1, 1)
        .await
        .map_err(|_| Error::Internal)?
        .value;
    let scope = manager.clone();
    queued_refusal(
        db,
        services,
        admin,
        bilibi_people_employee_workspace::MANAGE,
        async move {
            matches!(
                workspace.put_work_profile(&scope, 1, 1, &profile).await,
                Err(bilibi_people_employee_workspace::WorkspaceError::Workforce(
                    bilibi_people_workforce::WorkforceError::Unauthorized
                ))
            )
        },
    )
    .await
}
