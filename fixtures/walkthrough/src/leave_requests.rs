//! PL2: prove the real Leave facade and durable worker on a disposable database.
use bilibi_authz::grants::Grants;
use bilibi_database::{Db, Error, Result};
use bilibi_module_registry::Services;
use bilibi_people_leave::{
    Decision, Leave, LeaveError, NewEntry, NewLeaveType, NewPolicy, NewRequest, RequestRules,
    POLICIES_MANAGE, REQUESTS_APPROVE, SELF_VIEW,
};
use bilibi_people_reference_data::{NewCalendarException, ReferenceData};
use bilibi_tenancy::Scope;
use jiff::{civil::Date, ToSpan};
use rust_decimal::Decimal;

fn require(ok: bool, description: &str) -> Result<()> {
    if !ok {
        eprintln!("PL2 proof failed: {description}");
        return Err(Error::Internal);
    }
    println!("PL2: {description}");
    Ok(())
}
fn dec(value: i64) -> Decimal {
    Decimal::from(value)
}
fn leave_error(error: LeaveError) -> Error {
    eprintln!("PL2 unexpected refusal: {error:?}");
    Error::Internal
}
#[allow(clippy::too_many_arguments)]
async fn opening(
    leave: &Leave,
    scope: &Scope,
    company: i64,
    employee: i64,
    kind: i64,
    on: Date,
    key: &str,
    quantity: i64,
) -> Result<()> {
    leave
        .record_entry(
            scope,
            company,
            employee,
            &NewEntry {
                leave_type_id: kind,
                entry_type: "opening".into(),
                quantity: dec(quantity),
                occurred_on: on,
                source: "walkthrough-pl2".into(),
                entry_key: key.into(),
                note: None,
            },
        )
        .await
        .map_err(leave_error)?;
    Ok(())
}
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run(
    services: &Services,
    db: &Db,
    admin: &Scope,
    manager: &Scope,
    viewer: &Scope,
    company: i64,
    employee: i64,
) -> Result<()> {
    println!("\n== Leave requests and carry-forward (PL2)");
    let leave = services.get::<Leave>(bilibi_people_leave::SERVICE)?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    for key in [SELF_VIEW, REQUESTS_APPROVE, POLICIES_MANAGE] {
        grants
            .set_direct(admin, manager.user(), key, Some(true))
            .await?;
    }
    let today = leave.today(manager, company).await.map_err(leave_error)?;
    let rules = leave.rules(manager, company).await.map_err(leave_error)?;
    let current_year = rules.leave_year(today);
    leave
        .put_request_rules(
            manager,
            company,
            &RequestRules {
                working_weekdays: vec![1, 2, 3, 4, 5, 6, 7],
                backdate_days: 366,
            },
        )
        .await
        .map_err(leave_error)?;
    let future = today.checked_add(20.days()).map_err(|_| Error::Internal)?;
    let mut new_type = NewLeaveType::new("pl2-day", "Request walkthrough", "day");
    let day = leave
        .create_type(manager, company, &new_type)
        .await
        .map_err(leave_error)?;
    opening(
        &leave, manager, company, employee, day.id, future, "current", 10,
    )
    .await?;
    let mut am = NewRequest::new(day.id, future, "pl2-am");
    am.day_part = "am".into();
    require(
        matches!(
            leave.submit_request(viewer, company, &am).await,
            Err(LeaveError::Unauthorized)
        ),
        "self-service refuses an account without its grant",
    )?;
    let saved = leave
        .submit_request(manager, company, &am)
        .await
        .map_err(leave_error)?;
    require(
        saved.quantity == Decimal::new(50, 2),
        "morning request reserves 0.50",
    )?;
    let client = db.get().await?;
    let audit_before: i64 = client
        .query_one(
            "SELECT count(*) FROM base_audit_mutations WHERE tenant_id=$1",
            &[&manager.tenant()],
        )
        .await?
        .get(0);
    let replay = leave
        .submit_request(manager, company, &am)
        .await
        .map_err(leave_error)?;
    let audit_after: i64 = client
        .query_one(
            "SELECT count(*) FROM base_audit_mutations WHERE tenant_id=$1",
            &[&manager.tenant()],
        )
        .await?
        .get(0);
    require(
        replay.id == saved.id && audit_before == audit_after,
        "identical request-key replay adds no mutation",
    )?;
    let mut conflicting = am.clone();
    conflicting.reason = Some("different".into());
    require(
        matches!(
            leave.submit_request(manager, company, &conflicting).await,
            Err(LeaveError::RequestKeyConflict)
        ),
        "changed replay is refused",
    )?;
    let full = NewRequest::new(day.id, future, "pl2-overlap");
    require(
        matches!(
            leave.submit_request(manager, company, &full).await,
            Err(LeaveError::OverlappingRequest)
        ),
        "full date cannot overlap the reserved morning",
    )?;
    let mut pm = am.clone();
    pm.day_part = "pm".into();
    pm.request_key = "pl2-pm".into();
    let afternoon = leave
        .submit_request(manager, company, &pm)
        .await
        .map_err(leave_error)?;
    let balances = leave
        .balances(manager, company, employee, rules.leave_year(future))
        .await
        .map_err(leave_error)?;
    require(
        balances
            .iter()
            .any(|b| b.leave_type.id == day.id && b.pending == dec(1) && b.available == dec(9)),
        "two half days reserve one unit of balance",
    )?;
    require(
        matches!(
            leave
                .decide_request(manager, company, saved.id, Decision::Approve, None)
                .await,
            Err(LeaveError::SelfApproval)
        ),
        "requester cannot approve their own request",
    )?;
    leave
        .decide_request(
            admin,
            company,
            saved.id,
            Decision::Approve,
            Some("Approved"),
        )
        .await
        .map_err(leave_error)?;
    require(
        matches!(
            leave
                .decide_request(admin, company, saved.id, Decision::Approve, None)
                .await,
            Err(LeaveError::NotPending)
        ),
        "a decided request is no longer pending",
    )?;
    require(
        matches!(
            leave
                .decide_request(admin, company, afternoon.id, Decision::Reject, None)
                .await,
            Err(LeaveError::NoteRequired)
        ),
        "rejection requires a note",
    )?;
    leave
        .decide_request(
            admin,
            company,
            afternoon.id,
            Decision::Reject,
            Some("Please choose another date."),
        )
        .await
        .map_err(leave_error)?;
    leave
        .cancel_request(manager, company, saved.id)
        .await
        .map_err(leave_error)?;
    let ledger = leave
        .entries(manager, company, employee, rules.leave_year(future))
        .await
        .map_err(leave_error)?;
    require(
        ledger.iter().any(|e| {
            e.entry_key == format!("taken:{}", saved.id) && e.quantity == Decimal::new(-50, 2)
        }) && ledger.iter().any(|e| {
            e.entry_key == format!("cancelled:{}", saved.id) && e.quantity == Decimal::new(50, 2)
        }),
        "approval debits and pre-start cancellation restores the quantity",
    )?;
    let again = leave
        .submit_request(manager, company, &full)
        .await
        .map_err(leave_error)?;
    leave
        .cancel_request(manager, company, again.id)
        .await
        .map_err(leave_error)?;
    require(
        matches!(
            leave.cancel_request(manager, company, again.id).await,
            Err(LeaveError::NotCancellable)
        ),
        "a cancelled request cannot be cancelled again",
    )?;
    require(
        leave
            .requested_type_ids(manager, company, future, future)
            .await
            .map_err(leave_error)?
            .is_empty(),
        "Payroll excludes cancelled and rejected requests",
    )?;

    let references = services.get::<ReferenceData>(bilibi_people_reference_data::SERVICE)?;
    let closed_date = future.checked_add(1.day()).map_err(|_| Error::Internal)?;
    references
        .create_calendar_exception(
            admin,
            company,
            &NewCalendarException::new(closed_date, "Walkthrough closure"),
        )
        .await
        .map_err(|_| Error::Internal)?;
    let exception = NewRequest::new(day.id, closed_date, "pl2-exception");
    require(
        matches!(
            leave.submit_request(manager, company, &exception).await,
            Err(LeaveError::NoWorkingDays)
        ),
        "calendar exceptions contain no counted working date",
    )?;
    let mut range = NewRequest::new(day.id, future, "pl2-range");
    range.ends_on = future.checked_add(2.days()).map_err(|_| Error::Internal)?;
    let range_saved = leave
        .submit_request(manager, company, &range)
        .await
        .map_err(leave_error)?;
    require(
        range_saved.quantity == dec(2),
        "a date range omits its calendar exception",
    )?;
    leave
        .cancel_request(manager, company, range_saved.id)
        .await
        .map_err(leave_error)?;
    leave
        .put_request_rules(
            manager,
            company,
            &RequestRules {
                working_weekdays: vec![1, 2, 3, 4, 5],
                backdate_days: 366,
            },
        )
        .await
        .map_err(leave_error)?;
    let mut weekend = future;
    while weekend.weekday().to_monday_one_offset() != 7 {
        weekend = weekend.tomorrow().map_err(|_| Error::Internal)?;
    }
    require(
        matches!(
            leave
                .submit_request(
                    manager,
                    company,
                    &NewRequest::new(day.id, weekend, "pl2-weekend")
                )
                .await,
            Err(LeaveError::NoWorkingDays)
        ),
        "configured non-working weekdays are excluded",
    )?;
    leave
        .put_request_rules(
            manager,
            company,
            &RequestRules {
                working_weekdays: vec![1, 2, 3, 4, 5, 6, 7],
                backdate_days: 366,
            },
        )
        .await
        .map_err(leave_error)?;
    let mut old = NewRequest::new(
        day.id,
        today.checked_sub(367.days()).map_err(|_| Error::Internal)?,
        "pl2-old",
    );
    require(
        matches!(
            leave.submit_request(manager, company, &old).await,
            Err(LeaveError::TooFarBack)
        ),
        "backdating beyond the configured limit is refused",
    )?;
    let (_, year_end) = rules.year_range(current_year).map_err(leave_error)?;
    old.starts_on = year_end;
    old.ends_on = year_end.tomorrow().map_err(|_| Error::Internal)?;
    require(
        matches!(
            leave.submit_request(manager, company, &old).await,
            Err(LeaveError::SpansLeaveYears)
        ),
        "requests cannot cross a leave-year boundary",
    )?;
    new_type.code = "pl2-hour".into();
    new_type.name = "Hours walkthrough".into();
    new_type.unit = "hour".into();
    new_type.balance_required = false;
    let hours = leave
        .create_type(manager, company, &new_type)
        .await
        .map_err(leave_error)?;
    let hour_date = future.checked_add(4.days()).map_err(|_| Error::Internal)?;
    let mut hour_request = NewRequest::new(hours.id, hour_date, "pl2-hours");
    hour_request.day_part = "hours".into();
    hour_request.hours = Some(Decimal::new(125, 2));
    let h = leave
        .submit_request(manager, company, &hour_request)
        .await
        .map_err(leave_error)?;
    require(
        h.quantity == Decimal::new(125, 2),
        "hour-unit requests preserve exact hundredths without balance cover",
    )?;
    require(
        matches!(
            leave
                .submit_request(
                    manager,
                    company,
                    &NewRequest::new(day.id, hour_date, "pl2-cross-unit")
                )
                .await,
            Err(LeaveError::OverlappingRequest)
        ),
        "hour requests reserve both half-day slots across leave types",
    )?;
    require(
        leave
            .requested_type_ids(manager, company, hour_date, hour_date)
            .await
            .map_err(leave_error)?
            == vec![hours.id],
        "Payroll sees pending overlapping type IDs",
    )?;
    leave
        .cancel_request(manager, company, h.id)
        .await
        .map_err(leave_error)?;
    hour_request.hours = Some(Decimal::new(241, 1));
    hour_request.request_key = "pl2-too-many-hours".into();
    require(
        matches!(
            leave.submit_request(manager, company, &hour_request).await,
            Err(LeaveError::InvalidRequest)
        ),
        "more than 24 hours is refused",
    )?;
    let mut too_much = NewRequest::new(day.id, hour_date, "pl2-no-cover");
    too_much.ends_on = hour_date
        .checked_add(15.days())
        .map_err(|_| Error::Internal)?;
    require(
        matches!(
            leave.submit_request(manager, company, &too_much).await,
            Err(LeaveError::InsufficientBalance)
        ),
        "pending reservations cannot overspend the available balance",
    )?;
    let a = NewRequest::new(day.id, hour_date, "pl2-race-a");
    let b = NewRequest::new(day.id, hour_date, "pl2-race-b");
    let (left, right) = tokio::join!(
        leave.submit_request(manager, company, &a),
        leave.submit_request(manager, company, &b)
    );
    require(
        matches!(
            (&left, &right),
            (Ok(_), Err(LeaveError::OverlappingRequest))
                | (Err(LeaveError::OverlappingRequest), Ok(_))
        ),
        "concurrent submissions serialize the same half-day slots",
    )?;
    let race = left.or(right).map_err(leave_error)?;
    leave
        .cancel_request(manager, company, race.id)
        .await
        .map_err(leave_error)?;
    let today_request = leave
        .submit_request(
            manager,
            company,
            &NewRequest::new(day.id, today, "pl2-started"),
        )
        .await
        .map_err(leave_error)?;
    leave
        .decide_request(admin, company, today_request.id, Decision::Approve, None)
        .await
        .map_err(leave_error)?;
    require(
        matches!(
            leave
                .cancel_request(manager, company, today_request.id)
                .await,
            Err(LeaveError::NotCancellable)
        ),
        "approved leave cannot be cancelled on its start date",
    )?;
    require(
        leave
            .self_summary(manager, company, None)
            .await
            .map_err(leave_error)?
            .leave_year
            == current_year
            && !leave
                .self_requests(manager, company)
                .await
                .map_err(leave_error)?
                .is_empty(),
        "self reads resolve current employment and leave year",
    )?;
    require(
        leave
            .pending_requests(admin, company)
            .await
            .map_err(leave_error)?
            .len()
            <= 200,
        "approval queue is bounded",
    )?;

    let carry = leave
        .create_type(
            manager,
            company,
            &NewLeaveType::new("pl2-carry", "Carry walkthrough", "day"),
        )
        .await
        .map_err(leave_error)?;
    let (earlier, _) = rules.year_range(current_year - 2).map_err(leave_error)?;
    let (_, last_ended) = rules.year_range(current_year - 1).map_err(leave_error)?;
    leave
        .add_policy(
            manager,
            company,
            carry.id,
            &NewPolicy {
                effective_from: earlier,
                entitlement: dec(10),
                carry_forward_cap: Some(dec(3)),
            },
        )
        .await
        .map_err(leave_error)?;
    opening(
        &leave, manager, company, employee, carry.id, earlier, "earlier", 8,
    )
    .await?;
    opening(
        &leave, manager, company, employee, carry.id, last_ended, "ended", 5,
    )
    .await?;
    require(
        matches!(
            leave.carry_forward(manager, company, current_year).await,
            Err(LeaveError::YearNotEnded)
        ),
        "carry-forward refuses a year that has not ended",
    )?;
    let skipped = leave
        .carry_forward(manager, company, current_year - 1)
        .await
        .map_err(leave_error)?;
    require(
        skipped.previous_year_open > 0,
        "an earlier capped ledger year blocks a later close",
    )?;
    let report = leave
        .carry_forward_skipped(manager, company)
        .await
        .map_err(leave_error)?;
    require(
        report.skips.iter().any(|s| {
            s.employee_id == employee
                && s.leave_type_id == carry.id
                && s.blocking_year == current_year - 2
        }),
        "skip report records the earliest year to resolve",
    )?;
    leave
        .carry_forward(manager, company, current_year - 2)
        .await
        .map_err(leave_error)?;
    let carried = leave
        .entries(manager, company, employee, current_year - 1)
        .await
        .map_err(leave_error)?;
    let expired = leave
        .entries(manager, company, employee, current_year - 2)
        .await
        .map_err(leave_error)?;
    require(
        carried.iter().any(|e| {
            e.leave_type_id == carry.id && e.entry_type == "carried_forward" && e.quantity == dec(3)
        }) && expired.iter().any(|e| {
            e.leave_type_id == carry.id && e.entry_type == "expired" && e.quantity == dec(-5)
        }),
        "closing carries the cap and expires only the excess",
    )?;
    let pending = leave
        .submit_request(
            manager,
            company,
            &NewRequest::new(carry.id, last_ended, "pl2-ended-pending"),
        )
        .await
        .map_err(leave_error)?;
    let skipped = leave
        .carry_forward(manager, company, current_year - 1)
        .await
        .map_err(leave_error)?;
    require(skipped.pending > 0, "pending requests keep their year open")?;
    leave
        .cancel_request(manager, company, pending.id)
        .await
        .map_err(leave_error)?;
    let closed = leave
        .carry_forward(manager, company, current_year - 1)
        .await
        .map_err(leave_error)?;
    require(
        closed.carried > 0
            && leave
                .carry_forward_skipped(manager, company)
                .await
                .map_err(leave_error)?
                .skips
                .iter()
                .all(|s| s.employee_id != employee || s.leave_type_id != carry.id),
        "closing resolves and clears stored skips",
    )?;
    require(
        leave
            .carry_forward(manager, company, current_year - 1)
            .await
            .map_err(leave_error)?
            .carried
            == 0,
        "carry-forward replay adds no balance twice",
    )?;
    require(
        matches!(
            leave
                .submit_request(
                    manager,
                    company,
                    &NewRequest::new(carry.id, last_ended, "pl2-closed-year")
                )
                .await,
            Err(LeaveError::YearClosed)
        ),
        "a carried year refuses new requests",
    )?;
    require(
        leave
            .carried_forward_count(manager, company, current_year - 1)
            .await
            .map_err(leave_error)?
            > 0,
        "closed-year count reads durable closing entries",
    )?;

    let negative = leave
        .create_type(
            manager,
            company,
            &NewLeaveType::new("pl2-negative", "Deficit walkthrough", "day"),
        )
        .await
        .map_err(leave_error)?;
    let (ended_start, _) = rules.year_range(current_year - 1).map_err(leave_error)?;
    leave
        .add_policy(
            manager,
            company,
            negative.id,
            &NewPolicy {
                effective_from: ended_start,
                entitlement: dec(0),
                carry_forward_cap: Some(dec(3)),
            },
        )
        .await
        .map_err(leave_error)?;
    opening(
        &leave,
        manager,
        company,
        employee,
        negative.id,
        last_ended,
        "negative",
        -2,
    )
    .await?;
    leave
        .carry_forward(manager, company, current_year - 1)
        .await
        .map_err(leave_error)?;
    let negative_next = leave
        .entries(manager, company, employee, current_year)
        .await
        .map_err(leave_error)?;
    let negative_old = leave
        .entries(manager, company, employee, current_year - 1)
        .await
        .map_err(leave_error)?;
    require(
        negative_next.iter().any(|e| {
            e.leave_type_id == negative.id
                && e.entry_type == "carried_forward"
                && e.quantity.is_zero()
        }) && negative_old
            .iter()
            .filter(|e| e.leave_type_id == negative.id)
            .map(|e| e.quantity)
            .sum::<Decimal>()
            == dec(-2),
        "a deficit carries zero and stays in the old year",
    )?;

    let worker_type = leave
        .create_type(
            manager,
            company,
            &NewLeaveType::new("pl2-worker", "Worker walkthrough", "day"),
        )
        .await
        .map_err(leave_error)?;
    leave
        .add_policy(
            manager,
            company,
            worker_type.id,
            &NewPolicy {
                effective_from: ended_start,
                entitlement: dec(0),
                carry_forward_cap: Some(dec(2)),
            },
        )
        .await
        .map_err(leave_error)?;
    opening(
        &leave,
        manager,
        company,
        employee,
        worker_type.id,
        last_ended,
        "worker",
        4,
    )
    .await?;
    let queue = services.get::<bilibi_queue::Queue>("base/queue.store")?;
    let worker = queue.workers().get("people-leave/carry-forward")?;
    require((worker.definition.validate)(serde_json::json!({"company_id":company,"from_year":current_year-1,"actor_id":manager.user()})).is_err() && (worker.definition.validate)(serde_json::json!({"company_id":company,"from_year":9998})).is_err(), "worker arguments reject identity fields and unsupported years")?;

    // The host executor performs this queued job when the proof app runs.
    let job = leave
        .enqueue_carry_forward(manager, company, current_year - 1)
        .await
        .map_err(leave_error)?;
    let revoked_job = leave
        .enqueue_carry_forward(viewer, company, current_year - 1)
        .await;
    require(
        matches!(revoked_job, Err(LeaveError::Unauthorized)),
        "an unauthorized operator cannot enqueue carry-forward",
    )?;
    println!("PL2 worker job queued as operator: {}", job.id());
    let row=client.query_one("SELECT actor_id,worker_id,max_attempts FROM base_queue_jobs WHERE tenant_id=$1 AND id=$2",&[&manager.tenant(),&job.id()]).await?;
    require(
        row.get::<_, i64>("actor_id") == manager.user()
            && row.get::<_, String>("worker_id") == "people-leave/carry-forward"
            && row.get::<_, i32>("max_attempts") == 5,
        "Queue captures the operator and five-attempt budget",
    )?;
    grants
        .set_direct(admin, manager.user(), SELF_VIEW, Some(false))
        .await?;
    require(
        matches!(
            leave
                .submit_request(
                    manager,
                    company,
                    &NewRequest::new(day.id, hour_date, "pl2-revoked")
                )
                .await,
            Err(LeaveError::Unauthorized)
        ),
        "revoked self-service grant refuses the next write",
    )?;
    grants
        .set_direct(admin, manager.user(), SELF_VIEW, Some(true))
        .await?;
    let authentication =
        services.get::<bilibi_tenancy::Authentication>("base/tenancy.authentication")?;
    let staff = authentication
        .authenticate(&**client, manager.tenant(), crate::STAFF, String::new())
        .await?;
    grants
        .set_direct(admin, crate::STAFF, POLICIES_MANAGE, Some(true))
        .await?;
    let denied = leave
        .enqueue_carry_forward(&staff, company, current_year - 1)
        .await
        .map_err(leave_error)?;
    grants
        .set_direct(admin, crate::STAFF, POLICIES_MANAGE, Some(false))
        .await?;
    let not_ended = leave
        .enqueue_carry_forward(admin, company, current_year)
        .await
        .map_err(leave_error)?;
    println!(
        "PL2 worker refusal jobs: revoked {}, year-not-ended {}",
        denied.id(),
        not_ended.id()
    );
    println!("PL2 walkthrough complete");
    Ok(())
}
