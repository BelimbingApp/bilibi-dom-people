use super::*;

// Employee Workspace (PE1): owns its proof records and leaves a linked Person
// without employment for the Add employment dialog's browser interaction.
#[allow(clippy::too_many_arguments)]
async fn employee_workspace_proof(
    services: &Services,
    grants: &Grants,
    client: &tokio_postgres::Client,
    admin: &Scope,
    manager: &Scope,
    viewer: &Scope,
    manager_employee: i64,
    staff_employee: i64,
) -> Result<()> {
    use bilibi_people_employee_workspace::{
        Access, EmployeeWorkspace, NewChangeRequest, NewSavedView, WorkProfile, WorkspaceError,
        MANAGE, REVIEW, SERVICE as WORKSPACE, VIEW,
    };
    fn success<T>(value: std::result::Result<T, WorkspaceError>) -> Result<T> {
        value.map_err(|error| {
            eprintln!("PE1 unexpected refusal: {error}");
            Error::Internal
        })
    }
    fn prove(value: bool, step: &str) -> Result<()> {
        if !value {
            eprintln!("PE1 failed: {step}");
            return Err(Error::Internal);
        }
        println!("PE1: {step}");
        Ok(())
    }
    let workspace = services.get::<EmployeeWorkspace>(WORKSPACE)?;
    for key in [
        VIEW,
        MANAGE,
        REVIEW,
        "admin.person.list",
        "admin.affiliation.list",
    ] {
        grants.set_direct(admin, MANAGER, key, Some(true)).await?;
    }
    let rows = success(workspace.employees(manager, COMPANY).await)?;
    prove(
        rows.iter().any(|e| e.status == "pending"),
        "directory includes pending employment",
    )?;
    prove(
        !rows.is_empty(),
        "directory includes company employment facts",
    )?;
    prove(
        matches!(
            workspace.employees(viewer, COMPANY).await,
            Err(WorkspaceError::Workforce(WorkforceError::Unauthorized))
        ),
        "viewer cannot read employees",
    )?;
    prove(
        matches!(
            workspace.employees(manager, OTHER_TENANT_COMPANY).await,
            Err(WorkspaceError::Workforce(WorkforceError::NotFound))
                | Err(WorkspaceError::Workforce(WorkforceError::Unauthorized))
        ),
        "foreign company refused",
    )?;
    let profile = WorkProfile {
        work_location: Some("Work location".into()),
        work_arrangement: Some("Work arrangement".into()),
        notes: Some("PE1 work profile proof".into()),
    };
    let stored = success(
        workspace
            .put_work_profile(manager, COMPANY, manager_employee, &profile)
            .await,
    )?;
    prove(
        stored.work_location == profile.work_location,
        "profile created through facade",
    )?;
    let updated = WorkProfile {
        notes: Some("Updated work profile".into()),
        ..profile.clone()
    };
    success(
        workspace
            .put_work_profile(manager, COMPANY, manager_employee, &updated)
            .await,
    )?;
    prove(
        success(
            workspace
                .work_profile(manager, COMPANY, manager_employee)
                .await,
        )?
        .is_some_and(|p| p.notes == updated.notes),
        "profile update persisted",
    )?;
    prove(
        matches!(
            workspace
                .put_work_profile(
                    manager,
                    COMPANY,
                    manager_employee,
                    &WorkProfile {
                        work_location: Some("x".repeat(201)),
                        ..WorkProfile::default()
                    }
                )
                .await,
            Err(WorkspaceError::Workforce(WorkforceError::Invalid(_)))
        ),
        "overlong profile refused",
    )?;
    prove(
        matches!(
            workspace
                .put_work_profile(manager, COMPANY, i64::MAX, &profile)
                .await,
            Err(WorkspaceError::Workforce(WorkforceError::NotFound))
        ),
        "missing employee refused",
    )?;
    success(
        workspace
            .put_access(
                manager,
                COMPANY,
                staff_employee,
                &Access {
                    portal_enabled: true,
                    reason: Some("PE1 eligibility proof".into()),
                },
            )
            .await,
    )?;
    prove(
        success(workspace.access(manager, COMPANY, staff_employee).await)?
            .is_some_and(|a| a.portal_enabled),
        "portal eligibility saved",
    )?;
    let request = success(
        workspace
            .request_change(
                manager,
                COMPANY,
                staff_employee,
                &NewChangeRequest {
                    field: "full_name".into(),
                    proposed_value: "Proposed person name".into(),
                    reason: Some("PE1 review proof".into()),
                },
            )
            .await,
    )?;
    prove(
        request.requested_by_actor_id == manager.actor().user(),
        "requester derived from sealed scope",
    )?;
    let reviewed = success(
        workspace
            .review_change(manager, COMPANY, staff_employee, request.id, "approved")
            .await,
    )?;
    prove(
        reviewed.reviewed_by_actor_id == Some(manager.actor().user())
            && reviewed.reviewed_at.is_some(),
        "review records reviewer and time",
    )?;
    prove(
        matches!(
            workspace
                .review_change(manager, COMPANY, staff_employee, request.id, "rejected")
                .await,
            Err(WorkspaceError::AlreadyReviewed)
        ),
        "second review refused",
    )?;
    prove(
        matches!(
            workspace
                .review_change(manager, COMPANY, staff_employee, request.id, "pending")
                .await,
            Err(WorkspaceError::InvalidReview)
        ),
        "invalid decision refused",
    )?;
    prove(
        matches!(
            workspace
                .request_change(
                    manager,
                    COMPANY,
                    staff_employee,
                    &NewChangeRequest {
                        field: "status".into(),
                        proposed_value: "active".into(),
                        reason: None
                    }
                )
                .await,
            Err(WorkspaceError::Workforce(WorkforceError::Invalid(_)))
        ),
        "unsupported change field refused",
    )?;
    let view = NewSavedView {
        name: "PE1 active employees".into(),
        search: None,
        status: Some("active".into()),
    };
    let saved = success(workspace.save_view(manager, COMPANY, &view).await)?;
    prove(
        matches!(
            workspace.save_view(manager, COMPANY, &view).await,
            Err(WorkspaceError::Workforce(WorkforceError::Invalid(_)))
        ),
        "duplicate saved-view name refused",
    )?;
    prove(
        !success(workspace.saved_views(admin, COMPANY).await)?
            .iter()
            .any(|v| v.id == saved.id),
        "another account cannot list saved view",
    )?;
    prove(
        matches!(
            workspace.delete_view(admin, COMPANY, saved.id).await,
            Err(WorkspaceError::Workforce(WorkforceError::NotFound))
        ),
        "another account cannot delete saved view",
    )?;
    let disposable = success(
        workspace
            .save_view(
                manager,
                COMPANY,
                &NewSavedView {
                    name: "PE1 removable".into(),
                    ..NewSavedView::default()
                },
            )
            .await,
    )?;
    success(workspace.delete_view(manager, COMPANY, disposable.id).await)?;
    prove(
        !success(workspace.saved_views(manager, COMPANY).await)?
            .iter()
            .any(|v| v.id == disposable.id),
        "own view removed",
    )?;
    grants.set_direct(admin, MANAGER, VIEW, Some(false)).await?;
    prove(
        matches!(
            workspace
                .save_view(
                    manager,
                    COMPANY,
                    &NewSavedView {
                        name: "must refuse".into(),
                        ..NewSavedView::default()
                    }
                )
                .await,
            Err(WorkspaceError::Workforce(WorkforceError::Unauthorized))
        ),
        "grant revoked with old scope refuses next write",
    )?;
    grants.set_direct(admin, MANAGER, VIEW, Some(true)).await?;
    // A fixture-only lifecycle change on the disposable database proves the
    // same facade refuses a company archived after the actor was proved.
    client
        .execute(
            "UPDATE companies SET status='archived' WHERE tenant_id=$1 AND id=$2",
            &[&manager.tenant(), &OTHER_COMPANY],
        )
        .await?;
    prove(
        matches!(
            workspace.save_view(manager, OTHER_COMPANY, &view).await,
            Err(WorkspaceError::Workforce(WorkforceError::CompanyArchived))
        ),
        "archived company refuses saved view",
    )?;
    client
        .execute(
            "UPDATE companies SET status='active' WHERE tenant_id=$1 AND id=$2",
            &[&manager.tenant(), &OTHER_COMPANY],
        )
        .await?;
    let persons = services.get::<PersonService>(bilibi_person::SERVICE)?;
    let person = persons
        .create(
            admin,
            &PersonInput {
                name: "PE1 New Employee".into(),
                kind: PersonKind::Human,
                email: None,
                phone: None,
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    let affiliations = services.get::<AffiliationService>(bilibi_affiliation::SERVICE)?;
    affiliations
        .create(
            admin,
            &AffiliationInput {
                person_id: person.id,
                organisation_id: OrganisationId::new(COMPANY).map_err(|_| Error::Internal)?,
                title: Some("Employment dialog proof".into()),
                work_email: None,
                work_phone: None,
                dates: EffectiveDates::new(None, None).map_err(|_| Error::Internal)?,
            },
        )
        .await?;
    // The request is a proposal: the employee name is still the Core-owned name.
    prove(
        success(workspace.employee(manager, COMPANY, staff_employee).await)?
            .employee
            .display_name
            != "Proposed person name",
        "review leaves Core identity unchanged",
    )?;
    let audit_count:i64=client.query_one("SELECT count(*) FROM base_audit_mutations WHERE tenant_id=$1 AND auditable_type='people_employee_change_request'", &[&manager.tenant()]).await?.get(0);
    println!("PE1 change-request audit rows: {audit_count}");
    // Leave a reader-only account for the running app after proving the
    // ungranted reader refusal above; personal views still need only VIEW.
    grants.set_direct(admin, VIEWER, VIEW, Some(true)).await?;
    // The fixture viewer has no affiliation (hence no own-company axis).
    // Give explicit company reach while keeping every People write denied.
    grants
        .set_direct(
            admin,
            VIEWER,
            "admin.company.tenant-wide.manage",
            Some(true),
        )
        .await?;
    prove(
        !success(workspace.employees(viewer, COMPANY).await)?.is_empty(),
        "reader can list employees after a view grant",
    )?;
    prove(
        matches!(
            workspace
                .put_access(viewer, COMPANY, staff_employee, &Access::default())
                .await,
            Err(WorkspaceError::Workforce(WorkforceError::Unauthorized))
        ),
        "reader cannot change portal eligibility",
    )?;
    let reader_view = success(
        workspace
            .save_view(
                viewer,
                COMPANY,
                &NewSavedView {
                    name: "PE1 reader view".into(),
                    ..NewSavedView::default()
                },
            )
            .await,
    )?;
    prove(
        !success(workspace.saved_views(manager, COMPANY).await)?
            .iter()
            .any(|v| v.id == reader_view.id),
        "reader personal view is isolated from manager",
    )?;
    println!("PE1 browser: manager@example.test can add PE1 New Employee; viewer@example.test can read employees and manage only their own views.");
    Ok(())
}

walkthrough_step!(
    "employee_workspace",
    after = ["assessments"],
    |ctx| async move {
        let client = ctx.db.get().await?;
        // Employee Workspace (PE1): independent proof block below.
        employee_workspace_proof(
            &ctx.services,
            ctx.services.get::<Grants>("base/authz.grants")?.as_ref(),
            &client,
            &ctx.admin,
            &ctx.manager,
            &ctx.viewer,
            ctx.manager_employment.id,
            ctx.staff_employment.id,
        )
        .await?;

        Ok(())
    }
);
