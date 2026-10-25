use super::*;
use crate::attendance::decimal;

// PA4: browser proof records for My attendance, after the main walkthrough.
/// Creates self-service attendance records through the facades on the disposable fixture database.
/// `mode` also changes the policy while a browser page stays open, to prove current authority.
async fn run(url: &str, mode: &str) -> Result<()> {
    use bilibi_people_attendance::{NewAdjustment, RosterValue, ROSTER_MANAGE};
    let db = pool(url)?;
    let modules = bilibi_module_registry::discover()?;
    let mut services = Services::default();
    services.provide("bootstrap.database", Arc::new(db.clone()))?;
    services.provide("bootstrap.composition", Arc::new(bilibi_web::composition()))?;
    services.install(&modules)?;
    if std::env::var("BILIBI_ENV").as_deref() == Ok("development") {
        services
            .get::<bilibi_base::step_up::StepUp>(bilibi_base::step_up::SERVICE)?
            .set_enforced(false);
    }
    let authentication = services.get::<Authentication>("base/tenancy.authentication")?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    let workforce = services.get::<Workforce>(SERVICE)?;
    let attendance = services.get::<Attendance>(bilibi_people_attendance::SERVICE)?;
    let c = db.get().await?;
    let admin = authentication
        .authenticate(&**c, TENANT, ADMIN, "walkthrough".into())
        .await?;
    if mode == "unlink" {
        services
            .get::<UserService>("core/user.accounts")?
            .unlink(&admin, MANAGER)
            .await
            .map_err(|_| Error::Internal)?;
        return Ok(());
    }
    if mode == "link" || mode == "link-staff" {
        let user = if mode == "link" { MANAGER } else { STAFF };
        let number = if mode == "link" { "E-001" } else { "E-002" };
        let employees = workforce
            .employees(&admin, COMPANY)
            .await
            .map_err(|_| Error::Internal)?
            .require_current()
            .map_err(|_| Error::Internal)?;
        let id = employees
            .iter()
            .find(|e| e.employee_number == number)
            .ok_or(Error::NotFound)?
            .reference
            .stable_id
            .parse()
            .map_err(|_| Error::Invalid)?;
        let employment = workforce
            .employment(&admin, COMPANY, id)
            .await
            .map_err(|_| Error::Internal)?
            .require_current()
            .map_err(|_| Error::Internal)?;
        let users = services.get::<UserService>("core/user.accounts")?;
        users
            .link_person(
                &admin,
                user,
                PersonId::new(employment.person_id).map_err(|_| Error::Invalid)?,
            )
            .await
            .map_err(|_| Error::Internal)?;
        users
            .link_affiliation(
                &admin,
                user,
                bilibi_affiliation::AffiliationId::new(employment.affiliation_id)
                    .map_err(|_| Error::Invalid)?,
            )
            .await
            .map_err(|_| Error::Internal)?;
        return Ok(());
    }
    if mode == "revoke" || mode == "restore" {
        grants
            .set_direct(&admin, MANAGER, SELF_VIEW, Some(mode == "restore"))
            .await?;
        return Ok(());
    }
    for (user, key) in [
        (MANAGER, SELF_VIEW),
        (STAFF, SELF_VIEW),
        (VIEWER, SELF_VIEW),
        (MANAGER, RULES_MANAGE),
        (MANAGER, ROSTER_MANAGE),
    ] {
        grants.set_direct(&admin, user, key, Some(true)).await?;
    }
    let manager = authentication
        .authenticate(&**c, TENANT, MANAGER, "walkthrough".into())
        .await?;
    let required = mode == "location";
    attendance
        .put_rules(
            &manager,
            COMPANY,
            &RuleChanges {
                timezone: Some("Etc/UTC".into()),
                self_clock_enabled: Some(mode != "off"),
                location_required: Some(required),
                ..Default::default()
            },
        )
        .await
        .map_err(|e| {
            eprintln!("PA4 rules: {e}");
            Error::Internal
        })?;
    if mode != "seed" {
        return Ok(());
    }
    let employee = workforce
        .authorize_self(&manager, COMPANY, SELF_VIEW)
        .await
        .map_err(|_| Error::Internal)?
        .employee_id;
    let today = Timestamp::now().to_zoned(jiff::tz::TimeZone::UTC).date();
    let shift = attendance
        .create_shift_template(
            &manager,
            COMPANY,
            &NewShiftTemplate {
                code: "PA4-SHIFT".into(),
                name: "Walkthrough shift".into(),
                starts_at: time(8, 0, 0, 0),
                ends_at: time(17, 0, 0, 0),
                break_minutes: 60,
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    let tomorrow = today
        .checked_add(jiff::Span::new().days(1))
        .map_err(|_| Error::Invalid)?;
    attendance
        .plan_roster_entry(
            &manager,
            COMPANY,
            employee,
            today,
            RosterValue::Shift(shift.id),
        )
        .await
        .map_err(|_| Error::Internal)?;
    attendance
        .plan_roster_entry(&manager, COMPANY, employee, tomorrow, RosterValue::Rest)
        .await
        .map_err(|_| Error::Internal)?;
    attendance
        .publish_roster(&manager, COMPANY, today, tomorrow)
        .await
        .map_err(|_| Error::Internal)?;
    // This replacement remains a draft, so the browser must still show the published rest day.
    attendance
        .plan_roster_entry(
            &manager,
            COMPANY,
            employee,
            tomorrow,
            RosterValue::Shift(shift.id),
        )
        .await
        .map_err(|_| Error::Internal)?;
    let far = today
        .checked_add(jiff::Span::new().days(14))
        .map_err(|_| Error::Invalid)?;
    attendance
        .plan_roster_entry(
            &manager,
            COMPANY,
            employee,
            far,
            RosterValue::Shift(shift.id),
        )
        .await
        .map_err(|_| Error::Internal)?;
    attendance
        .publish_roster(&manager, COMPANY, far, far)
        .await
        .map_err(|_| Error::Internal)?;
    // Timestamp arithmetic uses duration units; calendar days belong to Date above.
    for days in 1..=32 {
        let at = Timestamp::now()
            .checked_sub(jiff::Span::new().hours(days * 24))
            .map_err(|_| Error::Invalid)?;
        attendance
            .record_clock(
                &manager,
                COMPANY,
                employee,
                &ClockInput::new(format!("pa4-day-{days}"), EventType::In, "walkthrough", at),
            )
            .await
            .map_err(|_| Error::Internal)?;
    }
    let at = Timestamp::now()
        .checked_sub(jiff::Span::new().hours(1))
        .map_err(|_| Error::Invalid)?;
    attendance
        .submit_adjustment(
            &manager,
            COMPANY,
            &NewAdjustment {
                request_key: "pa4-browser-initial".into(),
                event_type: EventType::Out,
                local_at: at
                    .to_zoned(jiff::tz::TimeZone::UTC)
                    .strftime("%Y-%m-%dT%H:%M")
                    .to_string(),
                reason: "Walkthrough missed clock event".into(),
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    attendance
        .create_clocking_location(
            &manager,
            COMPANY,
            &NewClockingLocation {
                code: "PA4-LOCATION".into(),
                name: "Walkthrough location".into(),
                latitude: decimal("1"),
                longitude: decimal("1"),
                radius_meters: 100,
            },
        )
        .await
        .map_err(|_| Error::Internal)?;
    println!("PA4 browser fixture: 32 days (31 visible), two published shifts (draft replacement hidden), day 15 outside the window, and one pending request.");
    Ok(())
}

walkthrough_command!("my-attendance", |url, argument| async move {
    run(url, argument.unwrap_or("seed")).await
});
