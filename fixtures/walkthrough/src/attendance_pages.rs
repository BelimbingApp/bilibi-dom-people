//! PA5: disposable records for roster publication and adjustment decisions.
use super::*;
use bilibi_people_attendance::{
    Attendance, EventType, NewAdjustment, NewShiftTemplate, RosterValue, RuleChanges,
    ADJUSTMENTS_APPROVE, ROSTER_MANAGE, RULES_MANAGE, SELF_VIEW,
};
fn failed(error: impl Display) -> Error {
    eprintln!("PA5 fixture: {error}");
    Error::Internal
}
pub(super) async fn run(url: &str) -> Result<()> {
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = pool(url)?;
    let modules = bilibi_module_registry::discover()?;
    bilibi_database::migrate(&db, &modules).await?;
    bilibi_walkthrough_fixture::install(&db).await?;
    let mut services = Services::default();
    services.provide("bootstrap.database", Arc::new(db.clone()))?;
    services.provide("bootstrap.composition", Arc::new(bilibi_web::composition()))?;
    services.install(&modules)?;
    // Use the installed services once: agent bindings refuse a second bootstrap.
    // Production seeds store the Dev role needed for this HTTP-only proof.
    bilibi_database::seeds::run(&db, &modules, services.catalog()?, &services).await?;
    services
        .get::<bilibi_base::step_up::StepUp>(bilibi_base::step_up::SERVICE)?
        .set_enforced(false);
    let grants = services.get::<Grants>("base/authz.grants")?;
    grants.reconcile().await?;
    let auth = services.get::<Authentication>("base/tenancy.authentication")?;
    let c = db.get().await?;
    let admin = auth
        .authenticate(&**c, TENANT, ADMIN, "walkthrough".into())
        .await?;
    for key in [ROSTER_MANAGE, ADJUSTMENTS_APPROVE, RULES_MANAGE, SELF_VIEW] {
        grants.set_direct(&admin, MANAGER, key, Some(true)).await?;
    }
    grants
        .set_direct(&admin, STAFF, SELF_VIEW, Some(true))
        .await?;
    let workforce = services.get::<Workforce>(SERVICE)?;
    let manager_employee = workforce
        .create_employment(
            &admin,
            COMPANY,
            &NewEmployment::new(
                MANAGER_PERSON,
                MANAGER_AFFILIATION,
                "PA5-001",
                "employment",
                "active",
            ),
        )
        .await
        .map_err(failed)?;
    workforce
        .create_employment(
            &admin,
            OTHER_COMPANY,
            &NewEmployment::new(
                CONTRACTOR_PERSON,
                CONTRACTOR_AFFILIATION,
                "PA5-003",
                "employment",
                "active",
            ),
        )
        .await
        .map_err(failed)?;
    let person = services
        .get::<PersonService>(bilibi_person::SERVICE)?
        .create(
            &admin,
            &PersonInput {
                name: "PA5 Staff".into(),
                kind: PersonKind::Human,
                email: None,
                phone: None,
            },
        )
        .await
        .map_err(failed)?;
    let affiliation = services
        .get::<AffiliationService>(bilibi_affiliation::SERVICE)?
        .create(
            &admin,
            &AffiliationInput {
                person_id: person.id,
                organisation_id: OrganisationId::new(COMPANY).map_err(|_| Error::Invalid)?,
                title: None,
                work_email: None,
                work_phone: None,
                dates: EffectiveDates::new(None, None).map_err(|_| Error::Invalid)?,
            },
        )
        .await?;
    crate::give_staff_desk(
        &services.get::<UserService>("core/user.accounts")?,
        &admin,
        STAFF,
        affiliation.affiliation.person_id,
        affiliation.affiliation.organisation_id,
    )
    .await
    .map_err(failed)?;
    let staff_employee = workforce
        .create_employment(
            &admin,
            COMPANY,
            &NewEmployment::new(
                person.id.get(),
                affiliation.id.get(),
                "PA5-002",
                "employment",
                "active",
            ),
        )
        .await
        .map_err(failed)?;
    let manager = auth
        .authenticate(&**c, TENANT, MANAGER, "walkthrough".into())
        .await?;
    let staff = auth
        .authenticate(&**c, TENANT, STAFF, "walkthrough".into())
        .await?;
    let attendance = services.get::<Attendance>(bilibi_people_attendance::SERVICE)?;
    attendance
        .put_rules(
            &manager,
            COMPANY,
            &RuleChanges {
                timezone: Some("Etc/UTC".into()),
                adjustment_window_days: Some(7),
                ..Default::default()
            },
        )
        .await
        .map_err(failed)?;
    let shift = attendance
        .create_shift_template(
            &manager,
            COMPANY,
            &NewShiftTemplate {
                code: "PA5-DAY".into(),
                name: "PA5 day shift".into(),
                starts_at: jiff::civil::time(8, 0, 0, 0),
                ends_at: jiff::civil::time(17, 0, 0, 0),
                break_minutes: 60,
            },
        )
        .await
        .map_err(failed)?;
    let today = jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date();
    attendance
        .plan_roster_entry(
            &manager,
            COMPANY,
            manager_employee.id,
            today,
            RosterValue::Rest,
        )
        .await
        .map_err(failed)?;
    attendance
        .plan_roster_entry(
            &manager,
            COMPANY,
            staff_employee.id,
            today,
            RosterValue::Shift(shift.id),
        )
        .await
        .map_err(failed)?;
    let yesterday = today.yesterday().map_err(|_| Error::Invalid)?;
    for (scope, key, clock, event_type) in [
        (&manager, "pa5-manager", "08:00", EventType::In),
        (&staff, "pa5-staff-in", "08:00", EventType::In),
        (&staff, "pa5-staff-out", "17:00", EventType::Out),
    ] {
        attendance
            .submit_adjustment(
                scope,
                COMPANY,
                &NewAdjustment {
                    request_key: key.into(),
                    event_type,
                    local_at: format!("{yesterday}T{clock}"),
                    reason: format!("PA5 browser proof {key}"),
                },
            )
            .await
            .map_err(failed)?;
    }
    println!(
        "PA5 browser fixture ready: company 1, week from {today}; manager and staff have two drafts and three pending requests. Manager can plan and decide; admin can independently decide the manager request; viewer has neither permission."
    );
    Ok(())
}

walkthrough_command!("attendance-pages", |url, _argument| async move {
    run(url).await
});
