//! PE2 running-system proof: actual facades and the mounted live page over the
//! disposable database, including stale browser intents and revoked grants.
use super::*;
use bilibi_people_employee_workspace::{EmployeeWorkspace, WorkspaceError, MANAGE, REVIEW, VIEW};
use bilibi_ui::page::{LiveInput, LiveRequest, PageRequest, PageService};
use serde_json::json;
use std::collections::HashMap;

fn prove(value: bool, step: &str) -> Result<()> {
    if !value {
        eprintln!("PE2 failed: {step}");
        return Err(Error::Internal);
    }
    println!("PE2: {step}");
    Ok(())
}
fn saved<T>(result: std::result::Result<T, WorkspaceError>) -> Result<T> {
    result.map_err(|e| {
        eprintln!("PE2 unexpected refusal: {e}");
        Error::Internal
    })
}

pub(super) async fn run(
    services: &Services,
    client: &tokio_postgres::Client,
    admin: &Scope,
    manager: &Scope,
    viewer: &Scope,
    employee: i64,
) -> Result<()> {
    let workspace = services.get::<EmployeeWorkspace>(bilibi_people_employee_workspace::SERVICE)?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    for key in [
        VIEW,
        MANAGE,
        REVIEW,
        EMPLOYMENT_VIEW,
        EMPLOYMENT_MANAGE,
        "admin.person.view",
        "admin.affiliation.view",
        "admin.audit.log.list",
    ] {
        grants.set_direct(admin, MANAGER, key, Some(true)).await?;
    }
    // Two first edits to an absent profile must not both claim version zero.
    let mut absent = None;
    for row in saved(workspace.employees(manager, COMPANY).await)? {
        if saved(
            workspace
                .work_profile_record(manager, COMPANY, row.id)
                .await,
        )?
        .id
        .is_none()
        {
            absent = Some(row.id);
            break;
        }
    }
    let absent = absent.ok_or(Error::Internal)?;
    let (a, b) = tokio::join!(
        workspace.update_work_profile(
            manager,
            COMPANY,
            absent,
            0,
            "notes",
            "First concurrent edit"
        ),
        workspace.update_work_profile(
            manager,
            COMPANY,
            absent,
            0,
            "notes",
            "Second concurrent edit"
        ),
    );
    prove(
        matches!(
            (&a, &b),
            (Ok(_), Err(WorkspaceError::Stale)) | (Err(WorkspaceError::Stale), Ok(_))
        ),
        "concurrent first edits serialize and only one version-zero write succeeds",
    )?;
    let old = saved(
        workspace
            .work_profile_record(manager, COMPANY, employee)
            .await,
    )?;
    saved(
        workspace
            .update_work_profile(
                manager,
                COMPANY,
                employee,
                old.version,
                "work_location",
                "Detail proof location",
            )
            .await,
    )?;
    prove(
        matches!(
            workspace
                .update_work_profile(
                    manager,
                    COMPANY,
                    employee,
                    old.version,
                    "notes",
                    "stale edit"
                )
                .await,
            Err(WorkspaceError::Stale)
        ),
        "a stale profile version refuses without overwriting",
    )?;
    let current = saved(
        workspace
            .work_profile_record(manager, COMPANY, employee)
            .await,
    )?;
    prove(
        current.version == old.version + 1
            && current.value.work_location.as_deref() == Some("Detail proof location"),
        "profile changes advance the stored version",
    )?;
    saved(
        workspace
            .update_work_profile(
                manager,
                COMPANY,
                employee,
                current.version,
                "notes",
                "Detail proof note",
            )
            .await,
    )?;
    let profile = saved(
        workspace
            .work_profile_record(manager, COMPANY, employee)
            .await,
    )?;
    prove(
        profile.value.work_location == current.value.work_location,
        "changing one profile field preserves the other facts",
    )?;
    prove(
        matches!(
            workspace
                .update_work_profile(
                    manager,
                    COMPANY,
                    employee,
                    profile.version,
                    "work_location",
                    &"x".repeat(201)
                )
                .await,
            Err(WorkspaceError::Workforce(WorkforceError::Invalid(_)))
        ),
        "profile validation refuses oversized inline values",
    )?;
    let old_access = saved(workspace.access_record(manager, COMPANY, employee).await)?;
    saved(
        workspace
            .update_access(
                manager,
                COMPANY,
                employee,
                old_access.version,
                "portal_enabled",
                "true",
            )
            .await,
    )?;
    prove(
        matches!(
            workspace
                .update_access(
                    manager,
                    COMPANY,
                    employee,
                    old_access.version,
                    "reason",
                    "stale eligibility edit"
                )
                .await,
            Err(WorkspaceError::Stale)
        ),
        "portal eligibility has an independent version fence",
    )?;
    let access = saved(workspace.access_record(manager, COMPANY, employee).await)?;
    saved(
        workspace
            .update_access(
                manager,
                COMPANY,
                employee,
                access.version,
                "reason",
                "Detail proof eligibility",
            )
            .await,
    )?;
    prove(
        saved(workspace.access(manager, COMPANY, employee).await)?
            .is_some_and(|a| a.portal_enabled),
        "editing an access reason preserves eligibility",
    )?;
    prove(
        matches!(
            workspace
                .update_work_profile(
                    viewer,
                    COMPANY,
                    employee,
                    profile.version,
                    "notes",
                    "denied"
                )
                .await,
            Err(WorkspaceError::Workforce(WorkforceError::Unauthorized))
        ),
        "a reader cannot invoke inline write facades",
    )?;

    let page = services.get::<PageService<Scope, Error>>("people/employee_workspace.pages")?;
    let path = HashMap::from([("id".into(), employee.to_string())]);
    let query = HashMap::from([("company_id".into(), COMPANY.to_string())]);
    let mut state = page
        .0
        .mount(PageRequest {
            operation: "employee",
            scope: Some(manager),
            path: &path,
            query: &query,
            body: &[],
            framed: false,
        })
        .await?
        .ok_or(Error::Internal)?;
    let body = state
        .update(LiveRequest {
            scope: manager,
            credential: "",
            input: LiveInput::Refresh,
        })
        .await?;
    prove(
        body.html.contains("employee-employment-facts")
            && body.html.contains("work-profile-facts")
            && body.html.contains("portal-access-facts"),
        "mounted detail renders employment, work profile and portal access",
    )?;
    prove(
        body.html
            .contains(&format!("data-record-id=\"{employee}\"")),
        "workspace follow announces the employee id rather than the page id",
    )?;
    prove(
        state.panels().len() == 3,
        "all three fact sections use shared audit history",
    )?;

    let commit = json!({"type":"save_field","id":employee.to_string(),"field":"notes","value":"not saved","version":old.version.to_string()});
    let body = state
        .update(LiveRequest {
            scope: manager,
            credential: "",
            input: LiveInput::Event(&commit),
        })
        .await?;
    prove(
        body.html.contains("data-status=\"error\"")
            && body.html.contains("This record has changed"),
        "a stale live commit renders a recoverable field refusal",
    )?;

    let submit = json!({"type":"request_change","request[field]":"full_name","request[proposed_value]":"PE2 approved proposal","request[reason]":"Detail proof"});
    state
        .update(LiveRequest {
            scope: manager,
            credential: "",
            input: LiveInput::Event(&submit),
        })
        .await?;
    let request = saved(workspace.change_requests(manager, COMPANY, employee).await)?
        .into_iter()
        .find(|r| r.proposed_value == "PE2 approved proposal")
        .ok_or(Error::Internal)?;
    let open = json!({"type":"approve_change","value":request.id.to_string()});
    let body = state
        .update(LiveRequest {
            scope: manager,
            credential: "",
            input: LiveInput::Event(&open),
        })
        .await?;
    prove(
        body.html.contains("Approve profile change request?")
            && saved(workspace.change_requests(manager, COMPANY, employee).await)?
                .iter()
                .any(|r| r.id == request.id && r.status == "pending"),
        "review selection opens confirmation without writing",
    )?;
    let cancel = json!({"type":"cancel_review"});
    state
        .update(LiveRequest {
            scope: manager,
            credential: "",
            input: LiveInput::Event(&cancel),
        })
        .await?;
    let confirm = json!({"type":"confirm_review","id":"999999","decision":"rejected"});
    state
        .update(LiveRequest {
            scope: manager,
            credential: "",
            input: LiveInput::Event(&confirm),
        })
        .await?;
    prove(
        saved(workspace.change_requests(manager, COMPANY, employee).await)?
            .iter()
            .any(|r| r.id == request.id && r.status == "pending"),
        "confirming after cancel cannot review a posted id",
    )?;
    state
        .update(LiveRequest {
            scope: manager,
            credential: "",
            input: LiveInput::Event(&open),
        })
        .await?;
    state
        .update(LiveRequest {
            scope: manager,
            credential: "",
            input: LiveInput::Event(&confirm),
        })
        .await?;
    prove(
        saved(workspace.change_requests(manager, COMPANY, employee).await)?
            .iter()
            .any(|r| r.id == request.id && r.status == "approved"),
        "confirmation uses the server-held id and decision",
    )?;
    let again = state
        .update(LiveRequest {
            scope: manager,
            credential: "",
            input: LiveInput::Event(&open),
        })
        .await?;
    prove(
        again.html.contains("Request cannot be reviewed."),
        "a reviewed request cannot be selected again",
    )?;

    let access = saved(workspace.access_record(manager, COMPANY, employee).await)?;
    let commit = json!({"type":"save_field","id":employee.to_string(),"field":"reason","value":"forbidden","version":access.version.to_string()});
    grants
        .set_direct(admin, MANAGER, MANAGE, Some(false))
        .await?;
    let body = state
        .update(LiveRequest {
            scope: manager,
            credential: "",
            input: LiveInput::Event(&commit),
        })
        .await?;
    prove(
        body.html.contains("You no longer have permission")
            && body.html.contains("data-editable=\"false\""),
        "revoking manage mid-page refuses the next edit and renders facts read-only",
    )?;
    prove(
        saved(workspace.access_record(manager, COMPANY, employee).await)?.version == access.version,
        "a forbidden live commit makes no write",
    )?;
    grants
        .set_direct(admin, MANAGER, MANAGE, Some(true))
        .await?;
    grants.set_direct(admin, MANAGER, VIEW, Some(false)).await?;
    let body = state
        .update(LiveRequest {
            scope: manager,
            credential: "",
            input: LiveInput::Refresh,
        })
        .await?;
    prove(
        body.html.contains("This employee is unavailable")
            && !body.html.contains("Detail proof location")
            && state.panels().is_empty(),
        "revoking view clears facts and history from the mounted state",
    )?;
    grants.set_direct(admin, MANAGER, VIEW, Some(true)).await?;
    let audit: i64 = client.query_one("SELECT count(*) FROM base_audit_mutations WHERE tenant_id=$1 AND auditable_type IN ('people_employee_workspace_profile','people_employee_access','people_employee_change_request')", &[&manager.tenant()]).await?.get(0);
    prove(
        audit >= 6,
        "inline fact and review changes produce audit mutations",
    )?;
    println!("PE2 browser record: /people/employees/{employee}?company_id={COMPANY}");
    Ok(())
}
