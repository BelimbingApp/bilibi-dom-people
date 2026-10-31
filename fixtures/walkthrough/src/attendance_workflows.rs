//! PA2 executable proof against the disposable People walkthrough database.
use super::*;
use bilibi_people_attendance::{
    AllowanceRule, NewAdjustment, NewAllowanceRule, RosterValue, ADJUSTMENTS_APPROVE,
    ALLOWANCES_MANAGE, ROSTER_MANAGE,
};
fn must<T: std::fmt::Debug>(step: &str, result: bilibi_people_attendance::Result<T>) -> Result<T> {
    match result {
        Ok(value) => {
            println!("{step}: {value:?}");
            Ok(value)
        }
        Err(e) => {
            eprintln!("{step}: {e:?}");
            Err(Error::Internal)
        }
    }
}
fn check(step: &str, condition: bool) -> Result<()> {
    if condition {
        println!("{step}: proved");
        Ok(())
    } else {
        eprintln!("{step}: FAILED");
        Err(Error::Internal)
    }
}
fn refused_as<T>(
    step: &str,
    result: bilibi_people_attendance::Result<T>,
    expected: fn(&AttendanceError) -> bool,
) -> Result<()> {
    match result {
        Err(e) if expected(&e) => {
            println!("{step}: refused ({e})");
            Ok(())
        }
        Err(e) => {
            eprintln!("{step}: unexpected {e:?}");
            Err(Error::Internal)
        }
        Ok(_) => {
            eprintln!("{step}: unexpectedly accepted");
            Err(Error::Internal)
        }
    }
}
pub async fn run(url: &str) -> Result<()> {
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
    let auth = services.get::<Authentication>("base/tenancy.authentication")?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    let workforce = services.get::<Workforce>(SERVICE)?;
    let attendance = services.get::<Attendance>(bilibi_people_attendance::SERVICE)?;
    let c = db.get().await?;
    let admin = auth
        .authenticate(&**c, TENANT, ADMIN, "walkthrough".into())
        .await?;
    for key in [
        RULES_MANAGE,
        ROSTER_MANAGE,
        ALLOWANCES_MANAGE,
        ADJUSTMENTS_APPROVE,
        SELF_VIEW,
    ] {
        grants.set_direct(&admin, MANAGER, key, Some(true)).await?;
    }
    for key in [SELF_VIEW, ADJUSTMENTS_APPROVE] {
        grants.set_direct(&admin, STAFF, key, Some(true)).await?;
    }
    let manager = auth
        .authenticate(&**c, TENANT, MANAGER, "walkthrough".into())
        .await?;
    let employees = workforce
        .employees(&**c, &manager, COMPANY)
        .await
        .map_err(|_| Error::Internal)?
        .require_current()
        .map_err(|_| Error::Internal)?;
    let employee = employees
        .iter()
        .find(|e| e.employee_number == "E-002")
        .and_then(|e| e.reference.stable_id.parse::<i64>().ok())
        .ok_or(Error::NotFound)?;
    let employment = workforce
        .employment(&**c, &manager, COMPANY, employee)
        .await
        .map_err(|_| Error::Internal)?
        .require_current()
        .map_err(|_| Error::Internal)?;
    let users = services.get::<UserService>("core/user.accounts")?;
    users
        .link_person(
            &admin,
            STAFF,
            PersonId::new(employment.person_id).map_err(|_| Error::Invalid)?,
        )
        .await
        .map_err(|_| Error::Internal)?;
    users
        .link_person(
            &admin,
            STAFF,
            PersonId::new(employment.person_id).map_err(|_| Error::Invalid)?,
        )
        .await
        .map_err(|_| Error::Internal)?;
    crate::give_staff_desk(
        &users,
        &admin,
        STAFF,
        bilibi_party_ids::PersonId::new(employment.person_id).map_err(|_| Error::Invalid)?,
        bilibi_party_ids::OrganisationId::new(employment.platform_company_id)
            .map_err(|_| Error::Invalid)?,
    )
    .await
    .map_err(|_| Error::Internal)?;
    let staff = auth
        .authenticate(&**c, TENANT, STAFF, "walkthrough".into())
        .await?;
    let viewer = auth
        .authenticate(&**c, TENANT, VIEWER, "walkthrough".into())
        .await?;
    drop(c);
    must(
        "PA2 rules",
        attendance
            .put_rules(
                &manager,
                COMPANY,
                &RuleChanges {
                    timezone: Some("Etc/UTC".into()),
                    self_clock_enabled: Some(true),
                    location_required: Some(false),
                    adjustment_window_days: Some(7),
                    ..Default::default()
                },
            )
            .await,
    )?;
    let today = Timestamp::now().to_zoned(jiff::tz::TimeZone::UTC).date();
    let shift = must(
        "create PA2 shift",
        attendance
            .create_shift_template(
                &manager,
                COMPANY,
                &NewShiftTemplate {
                    code: "PA2-DAY".into(),
                    name: "Walkthrough day".into(),
                    starts_at: "08:00".parse().map_err(|_| Error::Invalid)?,
                    ends_at: "17:00".parse().map_err(|_| Error::Invalid)?,
                    break_minutes: 60,
                },
            )
            .await,
    )?;
    must(
        "plan draft",
        attendance
            .plan_roster_entry(
                &manager,
                COMPANY,
                employee,
                today,
                RosterValue::Shift(shift.id),
            )
            .await,
    )?;
    check(
        "draft hidden from employee",
        must(
            "self roster before publication",
            attendance.self_roster(&staff, COMPANY, today, 14).await,
        )?
        .is_empty(),
    )?;
    let grid = must(
        "search roster",
        attendance
            .roster(&manager, COMPANY, today, 14, Some("E-002"))
            .await,
    )?;
    check(
        "matching employee and pending count",
        grid.employees.len() == 1 && grid.pending == 1 && grid.entries[0].pending(),
    )?;
    check(
        "publish one",
        must(
            "publish draft",
            attendance
                .publish_roster(&manager, COMPANY, today, today)
                .await,
        )? == 1,
    )?;
    let published = must(
        "published self roster",
        attendance.self_roster(&staff, COMPANY, today, 14).await,
    )?;
    check(
        "published shift joins template",
        published.len() == 1 && published[0].shift_code.as_deref() == Some("PA2-DAY"),
    )?;
    must(
        "plan published removal",
        attendance
            .plan_roster_entry(&manager, COMPANY, employee, today, RosterValue::None)
            .await,
    )?;
    check(
        "pending removal retains publication",
        must(
            "self roster with removal pending",
            attendance.self_roster(&staff, COMPANY, today, 14).await,
        )?
        .len()
            == 1,
    )?;
    must(
        "publish removal",
        attendance
            .publish_roster(&manager, COMPANY, today, today)
            .await,
    )?;
    check(
        "published removal gone",
        must(
            "self roster after removal",
            attendance.self_roster(&staff, COMPANY, today, 14).await,
        )?
        .is_empty(),
    )?;
    must(
        "plan rest",
        attendance
            .plan_roster_entry(&manager, COMPANY, employee, today, RosterValue::Rest)
            .await,
    )?;
    check(
        "clear unpublished rest deletes row",
        must(
            "clear rest",
            attendance
                .plan_roster_entry(&manager, COMPANY, employee, today, RosterValue::None)
                .await,
        )?
        .is_none(),
    )?;
    refused_as(
        "period over 31 days",
        attendance.roster(&manager, COMPANY, today, 32, None).await,
        |e| matches!(e, AttendanceError::InvalidPeriod),
    )?;
    refused_as(
        "roster viewer without grant",
        attendance.roster(&viewer, COMPANY, today, 1, None).await,
        |e| matches!(e, AttendanceError::Unauthorized),
    )?;
    must(
        "retire shift",
        attendance
            .set_shift_template_status(&manager, COMPANY, shift.id, "retired")
            .await,
    )?;
    refused_as(
        "retired shift refused",
        attendance
            .plan_roster_entry(
                &manager,
                COMPANY,
                employee,
                today,
                RosterValue::Shift(shift.id),
            )
            .await,
        |e| matches!(e, AttendanceError::ShiftUnavailable),
    )?;
    let at =
        Timestamp::from_second(Timestamp::now().as_second() - 3600).map_err(|_| Error::Invalid)?;
    let local = at.to_zoned(jiff::tz::TimeZone::UTC).datetime().to_string();
    let request = NewAdjustment {
        request_key: "pa2-punch".into(),
        event_type: EventType::In,
        local_at: local,
        reason: "Missing clock-in".into(),
    };
    let submitted = must(
        "submit missing punch",
        attendance
            .submit_adjustment(&staff, COMPANY, &request)
            .await,
    )?;
    check(
        "same key returns same request",
        must(
            "replay request",
            attendance
                .submit_adjustment(&staff, COMPANY, &request)
                .await,
        )?
        .id == submitted.id,
    )?;
    refused_as(
        "changed key facts",
        attendance
            .submit_adjustment(
                &staff,
                COMPANY,
                &NewAdjustment {
                    event_type: EventType::Out,
                    ..request.clone()
                },
            )
            .await,
        |e| matches!(e, AttendanceError::RequestKeyConflict),
    )?;
    refused_as(
        "duplicate with different key",
        attendance
            .submit_adjustment(
                &staff,
                COMPANY,
                &NewAdjustment {
                    request_key: "pa2-duplicate".into(),
                    ..request.clone()
                },
            )
            .await,
        |e| matches!(e, AttendanceError::DuplicateRequest),
    )?;
    refused_as(
        "future clock",
        attendance
            .submit_adjustment(
                &staff,
                COMPANY,
                &NewAdjustment {
                    request_key: "pa2-future".into(),
                    local_at: Timestamp::from_second(at.as_second() + 7200)
                        .map_err(|_| Error::Invalid)?
                        .to_zoned(jiff::tz::TimeZone::UTC)
                        .datetime()
                        .to_string(),
                    ..request.clone()
                },
            )
            .await,
        |e| matches!(e, AttendanceError::FutureTime),
    )?;
    refused_as(
        "outside window",
        attendance
            .submit_adjustment(
                &staff,
                COMPANY,
                &NewAdjustment {
                    request_key: "pa2-old".into(),
                    local_at: Timestamp::from_second(at.as_second() - 8 * 86400)
                        .map_err(|_| Error::Invalid)?
                        .to_zoned(jiff::tz::TimeZone::UTC)
                        .datetime()
                        .to_string(),
                    ..request.clone()
                },
            )
            .await,
        |e| matches!(e, AttendanceError::OutsideWindow),
    )?;
    refused_as(
        "requester cannot approve",
        attendance
            .decide_adjustment(&staff, COMPANY, submitted.id, "approve", None)
            .await,
        |e| matches!(e, AttendanceError::SelfApproval),
    )?;
    refused_as(
        "reject needs note",
        attendance
            .decide_adjustment(&manager, COMPANY, submitted.id, "reject", Some(" "))
            .await,
        |e| matches!(e, AttendanceError::NoteRequired),
    )?;
    let queue = must(
        "approver queue",
        attendance.pending_adjustments(&manager, COMPANY).await,
    )?;
    check(
        "queue resolves employee name",
        queue
            .iter()
            .any(|p| p.request.id == submitted.id && p.employee_name.is_some()),
    )?;
    must(
        "require location",
        attendance
            .put_rules(
                &manager,
                COMPANY,
                &RuleChanges {
                    location_required: Some(true),
                    ..Default::default()
                },
            )
            .await,
    )?;
    refused_as(
        "self clock needs coordinates",
        attendance
            .self_clock(
                &staff,
                COMPANY,
                EventType::In,
                "pa2-web-refused",
                None,
                None,
            )
            .await,
        |e| matches!(e, AttendanceError::LocationRequired),
    )?;
    let approved = must(
        "approve adjustment without geofence",
        attendance
            .decide_adjustment(
                &manager,
                COMPANY,
                submitted.id,
                "approve",
                Some("Confirmed"),
            )
            .await,
    )?;
    check(
        "approval ingests clock",
        approved.status == "approved" && approved.applied_clock_event_id.is_some(),
    )?;
    refused_as(
        "cannot decide twice",
        attendance
            .decide_adjustment(&manager, COMPANY, submitted.id, "reject", Some("No"))
            .await,
        |e| matches!(e, AttendanceError::NotPending),
    )?;
    let cancelled = must(
        "another request",
        attendance
            .submit_adjustment(
                &staff,
                COMPANY,
                &NewAdjustment {
                    request_key: "pa2-cancel".into(),
                    event_type: EventType::Out,
                    ..request.clone()
                },
            )
            .await,
    )?;
    must(
        "cancel own pending",
        attendance
            .cancel_adjustment(&staff, COMPANY, cancelled.id)
            .await,
    )?;
    refused_as(
        "cannot cancel twice",
        attendance
            .cancel_adjustment(&staff, COMPANY, cancelled.id)
            .await,
        |e| matches!(e, AttendanceError::NotPending),
    )?;
    let rejected = must(
        "resubmit cancelled punch",
        attendance
            .submit_adjustment(
                &staff,
                COMPANY,
                &NewAdjustment {
                    request_key: "pa2-reject".into(),
                    event_type: EventType::Out,
                    ..request.clone()
                },
            )
            .await,
    )?;
    must(
        "reject with note",
        attendance
            .decide_adjustment(
                &manager,
                COMPANY,
                rejected.id,
                "reject",
                Some("Time not supported"),
            )
            .await,
    )?;
    must(
        "own adjustment history",
        attendance.self_adjustments(&staff, COMPANY).await,
    )?;
    must(
        "disable location",
        attendance
            .put_rules(
                &manager,
                COMPANY,
                &RuleChanges {
                    location_required: Some(false),
                    ..Default::default()
                },
            )
            .await,
    )?;
    must(
        "self browser clock",
        attendance
            .self_clock(&staff, COMPANY, EventType::Out, "pa2-web", None, None)
            .await,
    )?;
    must(
        "disable clock",
        attendance
            .put_rules(
                &manager,
                COMPANY,
                &RuleChanges {
                    self_clock_enabled: Some(false),
                    ..Default::default()
                },
            )
            .await,
    )?;
    refused_as(
        "disabled self clock",
        attendance
            .self_clock(&staff, COMPANY, EventType::In, "pa2-disabled", None, None)
            .await,
        |e| matches!(e, AttendanceError::SelfClockUnavailable),
    )?;
    let base: NewAllowanceRule = NewAllowanceRule {
        code: "PA2-ALLOWANCE".into(),
        name: "Walkthrough allowance".into(),
        unit: "shift".into(),
        value: "12.5000".parse().map_err(|_| Error::Invalid)?,
        currency: "ABC".into(),
        effective_from: today,
        effective_until: None,
    };
    let first: AllowanceRule = must(
        "first allowance version",
        attendance
            .create_allowance_rule(&manager, COMPANY, &base)
            .await,
    )?;
    let later = today
        .checked_add(jiff::Span::new().days(10))
        .map_err(|_| Error::Invalid)?;
    let second = must(
        "next allowance version closes first",
        attendance
            .create_allowance_rule(
                &manager,
                COMPANY,
                &NewAllowanceRule {
                    effective_from: later,
                    ..base.clone()
                },
            )
            .await,
    )?;
    let first_now = must(
        "read ended version",
        attendance
            .get_allowance_rule(&manager, COMPANY, first.id)
            .await,
    )?;
    check(
        "inclusive automatic end",
        first_now.effective_until
            == Some(
                later
                    .checked_sub(jiff::Span::new().days(1))
                    .map_err(|_| Error::Invalid)?
                    .to_string(),
            ),
    )?;
    refused_as(
        "overlap refused",
        attendance
            .create_allowance_rule(
                &manager,
                COMPANY,
                &NewAllowanceRule {
                    effective_from: today
                        .checked_add(jiff::Span::new().days(5))
                        .map_err(|_| Error::Invalid)?,
                    ..base.clone()
                },
            )
            .await,
        |e| matches!(e, AttendanceError::EffectivePeriodOverlap),
    )?;
    let ended = must(
        "end active version earlier",
        attendance
            .end_allowance_rule(&manager, COMPANY, second.id, later)
            .await,
    )?;
    refused_as(
        "end cannot be extended",
        attendance
            .end_allowance_rule(
                &manager,
                COMPANY,
                second.id,
                later
                    .checked_add(jiff::Span::new().days(1))
                    .map_err(|_| Error::Invalid)?,
            )
            .await,
        |e| matches!(e, AttendanceError::InvalidEndDate),
    )?;
    refused_as(
        "end before start",
        attendance
            .end_allowance_rule(&manager, COMPANY, second.id, today)
            .await,
        |e| matches!(e, AttendanceError::Invalid(_)),
    )?;
    must(
        "retire version",
        attendance
            .retire_allowance_rule(&manager, COMPANY, second.id)
            .await,
    )?;
    check("ended value preserved", ended.value == base.value)?;
    refused_as(
        "same company code date stays unique after retirement",
        attendance
            .create_allowance_rule(
                &manager,
                COMPANY,
                &NewAllowanceRule {
                    effective_from: later,
                    ..base.clone()
                },
            )
            .await,
        |e| matches!(e, AttendanceError::Invalid(_)),
    )?;
    must(
        "payroll allowance read",
        attendance.list_allowance_rules(&manager, COMPANY).await,
    )?;
    // Revocation and account relinking must take effect on the next call with an earlier Scope.
    grants
        .set_direct(&admin, MANAGER, ROSTER_MANAGE, Some(false))
        .await?;
    refused_as(
        "revoked roster grant",
        attendance
            .plan_roster_entry(&manager, COMPANY, employee, today, RosterValue::Rest)
            .await,
        |e| matches!(e, AttendanceError::Unauthorized),
    )?;
    grants
        .set_direct(&admin, MANAGER, ROSTER_MANAGE, Some(true))
        .await?;
    users
        .unlink(&admin, STAFF)
        .await
        .map_err(|_| Error::Internal)?;
    refused_as(
        "old scope cannot submit after unlink",
        attendance
            .submit_adjustment(
                &staff,
                COMPANY,
                &NewAdjustment {
                    request_key: "pa2-unlinked".into(),
                    ..request
                },
            )
            .await,
        |e| matches!(e, AttendanceError::NotLinked),
    )?;
    users
        .link_person(
            &admin,
            STAFF,
            PersonId::new(employment.person_id).map_err(|_| Error::Invalid)?,
        )
        .await
        .map_err(|_| Error::Internal)?;
    crate::give_staff_desk(
        &users,
        &admin,
        STAFF,
        bilibi_party_ids::PersonId::new(employment.person_id).map_err(|_| Error::Invalid)?,
        bilibi_party_ids::OrganisationId::new(employment.platform_company_id)
            .map_err(|_| Error::Invalid)?,
    )
    .await
    .map_err(|_| Error::Internal)?;
    let c = db.get().await?;
    let source:String=c.query_one("SELECT source FROM people_attendance_clock_facts WHERE tenant_id=$1 AND company_id=$2 AND id=$3",
&[&TENANT,
&COMPANY,
&approved.applied_clock_event_id]).await?.get(0);
    check(
        "approved clock source is adjustment",
        source == "adjustment",
    )?;
    let count:i64=c.query_one("SELECT count(DISTINCT event) FROM base_audit_actions WHERE tenant_id=$1 AND event IN
('people.attendance.roster_published','people.attendance.adjustment_approved','people.attendance.adjustment_rejected','people.attendance.allowance_rule_ended')",
&[&TENANT]).await?.get(0);
    check("four audit action kinds recorded", count == 4)?;
    // Direct invalid SQL is refused on disposable records; each failure leaves the database usable.
    for (name,
sql) in [
 ("roster kind check",
"INSERT INTO people_attendance_roster_entries(tenant_id,company_id,employee_id,on_date,kind)
VALUES(1,1,1,current_date,'shift')"),
 ("correction decision check",
"UPDATE people_attendance_corrections SET status='approved' WHERE request_key='pa2-cancel'"),
 ("allowance positive check",
"UPDATE people_attendance_allowance_policies SET value=0 WHERE code='PA2-ALLOWANCE'"),
 ("allowance date order check",
"UPDATE people_attendance_allowance_policies SET effective_until=effective_from-1 WHERE code='PA2-ALLOWANCE'")]{
  let result=c.execute(sql,
&[]).await;
check(name,
result.as_ref().err().is_some_and(|e|e.code()==Some(&tokio_postgres::error::SqlState::CHECK_VIOLATION)))?;
 }
    println!("PA2 roster, adjustment, self-clock and allowance workflows proved.");
    Ok(())
}

walkthrough_command!("attendance-workflows", |url, _argument| async move {
    run(url).await
});
