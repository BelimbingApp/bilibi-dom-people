use super::*;

// ---------------------------------------------------------------------------
// PW1: employment records and self-service. Reads the private employment
// facts and the summary, changes and ends employments through the
// version-fenced `update_employment` (type, status, dates, supervisor,
// department) and shows its refusals (a stale version, a reporting cycle, a
// supervisor who is not working, an unknown department, another company's
// employee, an actor without the grant), walks the supervisor chain and the
// reach, resolves each account's own employee and proves it under the
// employment lock, shows the refusals the slice names (an affiliation of
// another company, a duplicate number, an unlinked account, a sibling
// company without reach), and asks for positions while Organisation is not
// mounted.
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
async fn pw1(
    services: &Services,
    client: &tokio_postgres::Client,
    admin: &Scope,
    manager: &Scope,
    viewer: &Scope,
    contractor: &Scope,
    manager_employment: &Employment,
    staff_employment: &Employment,
    contractor_employment: &Employment,
) -> Result<()> {
    use bilibi_company::CompanyService;
    use bilibi_people_workforce::{EmploymentChange, PositionScan};

    let workforce = services.get::<Workforce>(SERVICE)?;
    let persons = services.get::<PersonService>(bilibi_person::SERVICE)?;
    let affiliations = services.get::<AffiliationService>(bilibi_affiliation::SERVICE)?;
    let users = services.get::<UserService>("core/user.accounts")?;
    let companies = services.get::<CompanyService>(bilibi_company::SERVICE)?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    let authentication = services.get::<Authentication>("base/tenancy.authentication")?;
    let staff = authentication
        .authenticate(client, TENANT, STAFF, "walkthrough".into())
        .await?;
    println!("\n== PW1: employment records");

    // The private facts, under the view capability, at any status.
    if let Some(read) = show(
        &format!(
            "manager reads employee #{}'s employment",
            staff_employment.id
        ),
        workforce
            .employment(client, manager, COMPANY, staff_employment.id)
            .await,
    ) {
        if let Some(employment) = read.value {
            print_employment("read", &employment);
        }
    }
    show(
        &format!(
            "viewer reads employee #{}'s employment",
            staff_employment.id
        ),
        workforce
            .employment(client, viewer, COMPANY, staff_employment.id)
            .await,
    );
    show(
        &format!(
            "manager reads employee #{}'s employment in company {COMPANY}",
            contractor_employment.id
        ),
        workforce
            .employment(client, manager, COMPANY, contractor_employment.id)
            .await,
    );
    show(
        &format!("manager reads company {COMPANY}'s employment summary"),
        workforce.employment_summary(client, manager, COMPANY).await,
    );
    show(
        &format!("viewer reads company {COMPANY}'s employment summary"),
        workforce.employment_summary(client, viewer, COMPANY).await,
    );

    // A department of Company 001, through Core Company, for the change below.
    let department = match companies
        .create_department_type(
            admin,
            &bilibi_company::NewType {
                code: "PEOPLE-WALK".into(),
                name: "Walkthrough operations".into(),
                category: Some("operational".into()),
                description: None,
                is_active: None,
            },
        )
        .await?
    {
        bilibi_company::TypeOutcome::Saved(department_type) => {
            match companies
                .create_department(admin, COMPANY, department_type.id, "active")
                .await
            {
                Ok(bilibi_company::DepartmentOutcome::Saved(department)) => Some(department),
                Ok(bilibi_company::DepartmentOutcome::Refused(errors)) => {
                    println!(
                        "admin adds the Walkthrough operations department to Company 001: refused ({errors:?})"
                    );
                    None
                }
                Err(error) => {
                    println!(
                        "admin adds the Walkthrough operations department to Company 001: refused ({error})"
                    );
                    None
                }
            }
        }
        bilibi_company::TypeOutcome::Refused(errors) => {
            println!("admin creates the department type PEOPLE-WALK: refused ({errors:?})");
            None
        }
    };
    if let Some(department) = &department {
        println!(
            "admin adds the Walkthrough operations department to Company 001: department {}",
            department.id
        );
    }

    // Changes: type, status, dates, department and metadata, fenced on the
    // version the manager read.
    let changed = saved(show(
        &format!(
            "manager confirms employee #{} (version {}): permanent, active, department Walkthrough operations",
            staff_employment.id, staff_employment.version
        ),
        workforce
            .update_employment(
                manager,
                COMPANY,
                staff_employment.id,
                &EmploymentChange {
                    employment_type: Some("permanent".into()),
                    status: Some("active".into()),
                    employment_start: Some(Some(date(2026, 10, 1))),
                    department_id: Some(department.as_ref().map(|department| department.id)),
                    metadata: Some(Some(serde_json::json!({"confirmed": "2026-10-22"}))),
                    ..EmploymentChange::at(staff_employment.version)
                },
            )
            .await,
    ))?;
    print_employment("changed", &changed);
    show(
        &format!(
            "manager changes employee #{} again with the version already used ({})",
            staff_employment.id, staff_employment.version
        ),
        workforce
            .update_employment(
                manager,
                COMPANY,
                staff_employment.id,
                &EmploymentChange {
                    status: Some("inactive".into()),
                    ..EmploymentChange::at(staff_employment.version)
                },
            )
            .await,
    );
    show(
        &format!(
            "manager makes employee #{} report to their own report #{}",
            manager_employment.id, changed.id
        ),
        workforce
            .update_employment(
                manager,
                COMPANY,
                manager_employment.id,
                &EmploymentChange {
                    supervisor_employment_id: Some(Some(changed.id)),
                    ..EmploymentChange::at(manager_employment.version)
                },
            )
            .await,
    );
    show(
        &format!(
            "manager gives employee #{} the status 'fired' and an end before the start",
            changed.id
        ),
        workforce
            .update_employment(
                manager,
                COMPANY,
                changed.id,
                &EmploymentChange {
                    status: Some("fired".into()),
                    employment_end: Some(Some(date(2026, 9, 30))),
                    ..EmploymentChange::at(changed.version)
                },
            )
            .await,
    );
    show(
        &format!(
            "manager moves employee #{} to a department that does not exist",
            changed.id
        ),
        workforce
            .update_employment(
                manager,
                COMPANY,
                changed.id,
                &EmploymentChange {
                    department_id: Some(Some(i64::MAX)),
                    ..EmploymentChange::at(changed.version)
                },
            )
            .await,
    );
    show(
        &format!("viewer changes employee #{}", changed.id),
        workforce
            .update_employment(
                viewer,
                COMPANY,
                changed.id,
                &EmploymentChange {
                    status: Some("inactive".into()),
                    ..EmploymentChange::at(changed.version)
                },
            )
            .await,
    );
    show(
        &format!(
            "manager changes employee #{} in company {COMPANY}",
            contractor_employment.id
        ),
        workforce
            .update_employment(
                manager,
                COMPANY,
                contractor_employment.id,
                &EmploymentChange {
                    status: Some("inactive".into()),
                    ..EmploymentChange::at(contractor_employment.version)
                },
            )
            .await,
    );

    // A pending (not working) employee cannot supervise.
    let pending_person = persons
        .create(
            manager,
            &PersonInput {
                name: "Pat Pending".into(),
                kind: PersonKind::Human,
                email: None,
                phone: None,
            },
        )
        .await
        .map_err(|error| {
            println!("manager creates the Person Pat Pending: refused ({error})");
            Error::Internal
        })?;
    let pending_affiliation = affiliations
        .create(
            manager,
            &AffiliationInput {
                person_id: pending_person.id,
                organisation_id: OrganisationId::new(COMPANY).map_err(|_| Error::Invalid)?,
                title: Some("Trainee".into()),
                work_email: None,
                work_phone: None,
                dates: EffectiveDates::new(None, None).map_err(|_| Error::Invalid)?,
            },
        )
        .await?;
    let pending_employment = saved(show(
        "manager employs Pat Pending as pending, starting next year",
        workforce
            .create_employment(
                manager,
                COMPANY,
                &NewEmployment {
                    employment_start: Some(date(2027, 1, 4)),
                    supervisor_employment_id: Some(manager_employment.id),
                    ..NewEmployment::new(
                        pending_person.id.get(),
                        pending_affiliation.id.get(),
                        "E-003",
                        "full_time",
                        "pending",
                    )
                },
            )
            .await,
    ))?;
    print_employment("saved", &pending_employment);
    show(
        &format!(
            "manager makes employee #{} report to the pending employee #{}",
            changed.id, pending_employment.id
        ),
        workforce
            .update_employment(
                manager,
                COMPANY,
                changed.id,
                &EmploymentChange {
                    supervisor_employment_id: Some(Some(pending_employment.id)),
                    ..EmploymentChange::at(changed.version)
                },
            )
            .await,
    );

    // The supervisor chain and the reach.
    show(
        &format!("supervisor chain of employee #{}", changed.id),
        workforce
            .supervisor_chain(client, manager, COMPANY, changed.id)
            .await,
    );
    show(
        &format!("supervisor chain of employee #{}", manager_employment.id),
        workforce
            .supervisor_chain(client, manager, COMPANY, manager_employment.id)
            .await,
    );
    show(
        &format!(
            "supervisor chain of the pending employee #{}",
            pending_employment.id
        ),
        workforce
            .supervisor_chain(client, manager, COMPANY, pending_employment.id)
            .await,
    );
    show(
        &format!(
            "employee #{} is in employee #{}'s reach",
            changed.id, manager_employment.id
        ),
        workforce
            .in_reach(client, viewer, COMPANY, manager_employment.id, changed.id)
            .await,
    );
    show(
        &format!(
            "employee #{} is in employee #{}'s reach",
            manager_employment.id, changed.id
        ),
        workforce
            .in_reach(client, viewer, COMPANY, changed.id, manager_employment.id)
            .await,
    );
    show(
        &format!(
            "the pending employee #{} is in employee #{}'s reach",
            pending_employment.id, manager_employment.id
        ),
        workforce
            .in_reach(
                client,
                viewer,
                COMPANY,
                manager_employment.id,
                pending_employment.id,
            )
            .await,
    );

    // Self-service: each account's own employee, resolved now.
    println!("\n== PW1: self-service");
    show(
        &format!("manager's own employee in company {COMPANY}"),
        workforce.self_employee(client, manager, COMPANY).await,
    );
    show(
        &format!("staff's own employee in company {COMPANY}"),
        workforce.self_employee(client, &staff, COMPANY).await,
    );
    show(
        &format!("viewer's (unlinked account) own employee in company {COMPANY}"),
        workforce.self_employee(client, viewer, COMPANY).await,
    );
    show(
        &format!("contractor's own employee in company {COMPANY} (affiliated with Company 002)"),
        workforce.self_employee(client, contractor, COMPANY).await,
    );
    show(
        &format!("contractor's own employee in company {OTHER_COMPANY}"),
        workforce
            .self_employee(client, contractor, OTHER_COMPANY)
            .await,
    );
    show(
        &format!("staff authorizes employment.view for themselves in company {COMPANY}"),
        workforce
            .authorize_self(client, &staff, COMPANY, EMPLOYMENT_VIEW)
            .await,
    );
    show(
        &format!("manager authorizes employment.view for themselves in company {COMPANY}"),
        workforce
            .authorize_self(client, manager, COMPANY, EMPLOYMENT_VIEW)
            .await,
    );
    show(
        "manager runs a self-service write under their own employment lock",
        workforce
            .with_self_employee_lock(
                manager,
                COMPANY,
                EMPLOYMENT_VIEW,
                "people.walkthrough.self_service",
                |tx, me| {
                    Box::pin(async move {
                        // The row is locked for this transaction; a read on it
                        // shows which employee the write would act for.
                        let row = tx
                            .sql()
                            .query_one(
                                "SELECT employee_number FROM people_workforce_employments WHERE id = $1",
                                &[&me.employee_id],
                            )
                            .await
                            .map_err(Error::from)?;
                        Ok(format!(
                            "acting for employee #{} ({})",
                            me.employee_id,
                            row.get::<_, String>(0)
                        ))
                    })
                },
            )
            .await,
    );
    show(
        "viewer runs a self-service write",
        workforce
            .with_self_employee_lock(
                viewer,
                COMPANY,
                EMPLOYMENT_VIEW,
                "people.walkthrough.self_service",
                |_tx, me| {
                    let id = me.employee_id;
                    Box::pin(async move { Ok(id) })
                },
            )
            .await,
    );
    // Ending the contractor's employment: terminated, with the last day.
    let ended = saved(show(
        &format!(
            "manager ends the contractor's employment #{} at Company 002",
            contractor_employment.id
        ),
        workforce
            .update_employment(
                manager,
                OTHER_COMPANY,
                contractor_employment.id,
                &EmploymentChange {
                    status: Some("terminated".into()),
                    employment_end: Some(Some(date(2026, 10, 31))),
                    ..EmploymentChange::at(contractor_employment.version)
                },
            )
            .await,
    ))?;
    print_employment("ended", &ended);
    if let Some(read) = show(
        &format!("manager lists company {OTHER_COMPANY}'s employees"),
        workforce.employees(client, manager, OTHER_COMPANY).await,
    ) {
        print_employees(
            "working employees of Company 002 after the end",
            read.value.as_deref().unwrap_or(&[]),
        );
    }
    show(
        &format!("contractor's own employee in company {OTHER_COMPANY} after the end"),
        workforce
            .self_employee(client, contractor, OTHER_COMPANY)
            .await,
    );
    // The account unlinked while the employment stands: the next call refuses.
    match users.unlink(manager, STAFF).await {
        Ok(links) => println!(
            "manager unlinks staff@example.test from Sam Staff's affiliation: affiliation {:?}",
            links.person
        ),
        Err(error) => println!("manager unlinks staff@example.test: refused ({error})"),
    }
    show(
        &format!("staff's own employee in company {COMPANY} after the unlink"),
        workforce.self_employee(client, &staff, COMPANY).await,
    );
    show(
        &format!(
            "manager reads employee #{}'s employment after the unlink",
            changed.id
        ),
        workforce
            .employment(client, manager, COMPANY, changed.id)
            .await,
    );

    // A sibling company without reach: the contractor holds the employment
    // grants directly but no tenant-wide reach, so Company 002 is theirs to
    // write and Company 001 is not.
    println!("\n== PW1: reach over companies");
    for key in [EMPLOYMENT_MANAGE, bilibi_affiliation::contributions::PROVE] {
        match grants.set_direct(admin, CONTRACTOR, key, Some(true)).await {
            Ok(()) => println!("admin grants {key} to contractor@example.test directly"),
            Err(error) => {
                println!("admin grants {key} to contractor@example.test: refused ({error})")
            }
        }
    }
    let lead_person = persons
        .create(
            manager,
            &PersonInput {
                name: "Lee Lead".into(),
                kind: PersonKind::Human,
                email: None,
                phone: None,
            },
        )
        .await
        .map_err(|error| {
            println!("manager creates the Person Lee Lead: refused ({error})");
            Error::Internal
        })?;
    let lead_affiliation = affiliations
        .create(
            manager,
            &AffiliationInput {
                person_id: lead_person.id,
                organisation_id: OrganisationId::new(OTHER_COMPANY).map_err(|_| Error::Invalid)?,
                title: Some("Site Lead".into()),
                work_email: None,
                work_phone: None,
                dates: EffectiveDates::new(None, None).map_err(|_| Error::Invalid)?,
            },
        )
        .await?;
    show(
        "contractor employs Lee Lead at Company 001 (a sibling company without reach)",
        workforce
            .create_employment(
                contractor,
                COMPANY,
                &NewEmployment::new(
                    lead_person.id.get(),
                    lead_affiliation.id.get(),
                    "E-004",
                    "full_time",
                    "active",
                ),
            )
            .await,
    );
    show(
        "contractor employs Lee Lead at Company 002 with the contractor's own number",
        workforce
            .create_employment(
                contractor,
                OTHER_COMPANY,
                &NewEmployment::new(
                    lead_person.id.get(),
                    lead_affiliation.id.get(),
                    "C-001",
                    "full_time",
                    "active",
                ),
            )
            .await,
    );
    if let Some(lead) = show(
        "contractor employs Lee Lead at Company 002",
        workforce
            .create_employment(
                contractor,
                OTHER_COMPANY,
                &NewEmployment {
                    employment_start: Some(date(2026, 11, 2)),
                    ..NewEmployment::new(
                        lead_person.id.get(),
                        lead_affiliation.id.get(),
                        "C-002",
                        "full_time",
                        "active",
                    )
                },
            )
            .await,
    ) {
        print_employment("saved", &lead);
    }
    show(
        "manager employs Lee Lead's Company 002 affiliation at Company 001",
        workforce
            .create_employment(
                manager,
                COMPANY,
                &NewEmployment::new(
                    lead_person.id.get(),
                    lead_affiliation.id.get(),
                    "E-005",
                    "full_time",
                    "active",
                ),
            )
            .await,
    );
    show(
        &format!("manager reads company {OTHER_COMPANY}'s employment summary"),
        workforce
            .employment_summary(client, manager, OTHER_COMPANY)
            .await,
    );

    // Positions come from Organisation, which is not mounted here.
    println!("\n== PW1: positions");
    println!(
        "a mounted module answers the positions port: {}",
        workforce.positions_available()
    );
    show(
        &format!("manager scans company {COMPANY}'s positions as of 2026-10-22"),
        workforce
            .positions(
                client,
                manager,
                COMPANY,
                &PositionScan::from_start(date(2026, 10, 22)),
            )
            .await,
    );
    show(
        "manager scans positions with a page size of 0",
        workforce
            .positions(
                client,
                manager,
                COMPANY,
                &PositionScan {
                    page_size: 0,
                    ..PositionScan::from_start(date(2026, 10, 22))
                },
            )
            .await,
    );
    Ok(())
}

walkthrough_step!(
    "workforce_records",
    after = ["leave_requests"],
    |ctx| async move {
        let client = ctx.db.get().await?;
        // PW1: employment records and self-service.
        pw1(
            &ctx.services,
            &client,
            &ctx.admin,
            &ctx.manager,
            &ctx.viewer,
            &ctx.contractor,
            &ctx.manager_employment,
            &ctx.staff_employment,
            &ctx.contractor_employment,
        )
        .await?;

        Ok(())
    }
);
