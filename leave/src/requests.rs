//! Bilimbi `Leave.Requests`: counted dates, reservations and transitions.
use crate::{
    authorization, input, ledger, policies, types, Balance, Company, Date, Decimal, Error, Leave,
    LeaveError, LedgerEntry, MutationTransaction, Result, Scope, REQUESTS_APPROVE, SELF_VIEW,
};
use bilibi_database::Change;
use jiff::Timestamp;
use serde::Serialize;
use serde_json::json;
use tokio_postgres::{GenericClient, Row};

/// A self-service request. Full days may span up to 366 dates; the other
/// parts cover one date. Hours use hundredths, above zero and at most 24.
#[derive(Clone, Debug)]
pub struct NewRequest {
    pub leave_type_id: i64,
    pub starts_on: Date,
    pub ends_on: Date,
    pub day_part: String,
    pub hours: Option<Decimal>,
    pub reason: Option<String>,
    pub request_key: String,
}
impl NewRequest {
    pub fn new(leave_type_id: i64, starts_on: Date, request_key: impl Into<String>) -> Self {
        Self {
            leave_type_id,
            starts_on,
            ends_on: starts_on,
            day_part: "full".into(),
            hours: None,
            reason: None,
            request_key: request_key.into(),
        }
    }
}

/// A request's saved facts (Bilimbi `request_view`); timestamps are UTC.
#[derive(Clone, Debug, Serialize)]
pub struct Request {
    pub id: i64,
    pub employee_id: i64,
    pub leave_type_id: i64,
    pub leave_year: i32,
    #[serde(serialize_with = "crate::as_text")]
    pub starts_on: Date,
    #[serde(serialize_with = "crate::as_text")]
    pub ends_on: Date,
    pub day_part: String,
    #[serde(serialize_with = "crate::as_text")]
    pub quantity: Decimal,
    pub unit: String,
    pub status: String,
    pub reason: Option<String>,
    pub requested_by_user_id: i64,
    pub decided_by_user_id: Option<i64>,
    pub decided_at: Option<String>,
    pub decision_note: Option<String>,
    pub cancelled_by_user_id: Option<i64>,
    pub cancelled_at: Option<String>,
    pub inserted_at: String,
}
/// An approver's bounded queue row, including the Workforce display name.
#[derive(Clone, Debug, Serialize)]
pub struct PendingRequest {
    #[serde(flatten)]
    pub request: Request,
    pub employee_name: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Approve,
    Reject,
}
#[derive(Clone, Debug, Serialize)]
pub struct SelfSummary {
    pub leave_year: i32,
    #[serde(serialize_with = "crate::as_text")]
    pub starts_on: Date,
    #[serde(serialize_with = "crate::as_text")]
    pub ends_on: Date,
    pub balances: Vec<Balance>,
    pub entries: Vec<LedgerEntry>,
}

const COLUMNS: &str = "id, employee_id, leave_type_id, leave_year,
    to_char(starts_on, 'YYYY-MM-DD') AS starts_on, to_char(ends_on, 'YYYY-MM-DD') AS ends_on,
    day_part, quantity, unit, status, reason, requested_by_user_id, decided_by_user_id,
    to_char(decided_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS decided_at,
    decision_note, cancelled_by_user_id,
    to_char(cancelled_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS cancelled_at,
    to_char(inserted_at, 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS inserted_at";
fn read(row: &Row) -> Result<Request> {
    Ok(Request {
        id: row.get("id"),
        employee_id: row.get("employee_id"),
        leave_type_id: row.get("leave_type_id"),
        leave_year: row.get("leave_year"),
        starts_on: policies::date(row, "starts_on")?.ok_or(Error::Internal)?,
        ends_on: policies::date(row, "ends_on")?.ok_or(Error::Internal)?,
        day_part: row.get("day_part"),
        quantity: row.get("quantity"),
        unit: row.get("unit"),
        status: row.get("status"),
        reason: row.get("reason"),
        requested_by_user_id: row.get("requested_by_user_id"),
        decided_by_user_id: row.get("decided_by_user_id"),
        decided_at: row.get("decided_at"),
        decision_note: row.get("decision_note"),
        cancelled_by_user_id: row.get("cancelled_by_user_id"),
        cancelled_at: row.get("cancelled_at"),
        inserted_at: row.get("inserted_at"),
    })
}
struct CountedDate {
    on_date: Date,
    am: bool,
    pm: bool,
    quantity: Decimal,
}

pub(super) struct SubmittedRequest {
    pub request: Request,
    pub created: bool,
}

impl Leave {
    /// Today's date in the company's configured `localization.timezone`.
    pub async fn today(&self, scope: &Scope, company_id: i64) -> Result<Date> {
        let client = self.db.get().await.map_err(Error::from)?;
        let company = self.current_company(&**client, scope, company_id).await?;
        self.today_on(&**client, scope, &company).await
    }
    pub(super) async fn today_on(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company: &Company,
    ) -> Result<Date> {
        let resolved = self
            .settings
            .get(
                client,
                scope,
                crate::target(company.platform_company_id)?,
                bilibi_datetime::TIMEZONE_KEY,
            )
            .await?;
        let zone = resolved
            .value
            .as_str()
            .unwrap_or(bilibi_datetime::DEFAULT_TIMEZONE);
        Ok(Timestamp::now()
            .in_tz(zone)
            .map_err(|_| LeaveError::Unavailable)?
            .date())
    }
    /// Balances and ledger of the account's currently linked employee.
    pub async fn self_summary(
        &self,
        scope: &Scope,
        company_id: i64,
        on: Option<Date>,
    ) -> Result<SelfSummary> {
        let client = self.db.get().await.map_err(bilibi_database::Error::from)?;
        let me = self
            .workforce
            .authorize_self(&**client, scope, company_id, SELF_VIEW)
            .await?;
        drop(client);
        let rules = self.rules(scope, company_id).await?;
        let year = rules.leave_year(match on {
            Some(date) => date,
            None => self.today(scope, company_id).await?,
        });
        let (starts_on, ends_on) = rules.year_range(year)?;
        Ok(SelfSummary {
            leave_year: year,
            starts_on,
            ends_on,
            balances: self
                .balances(scope, company_id, me.employee_id, year)
                .await?,
            entries: self
                .entries(scope, company_id, me.employee_id, year)
                .await?,
        })
    }
    /// Latest 50 own requests, newest start first. Resolves the link again.
    pub async fn self_requests(&self, scope: &Scope, company_id: i64) -> Result<Vec<Request>> {
        let client = self.db.get().await.map_err(Error::from)?;

        let me = self
            .workforce
            .authorize_self(&**client, scope, company_id, SELF_VIEW)
            .await?;
        client.query(&format!("SELECT {COLUMNS} FROM people_leave_applications WHERE tenant_id=$1 AND company_id=$2 AND employee_id=$3 ORDER BY starts_on DESC, id DESC LIMIT 50"), &[&scope.tenant(), &company_id, &me.employee_id]).await.map_err(Error::from)?.iter().map(read).collect()
    }
    /// One leave request home. Self ownership is resolved anew for every read.
    pub async fn request(&self, scope: &Scope, company_id: i64, id: i64) -> Result<PendingRequest> {
        let client = self.db.get().await.map_err(Error::from)?;
        let reviewer =
            match authorization::authorize(&**client, scope, company_id, REQUESTS_APPROVE).await {
                Ok(_) => true,
                Err(crate::WorkforceError::Unauthorized | crate::WorkforceError::NotFound) => false,
                Err(error) => return Err(error.into()),
            };
        let owner = match self
            .workforce
            .authorize_self(&**client, scope, company_id, SELF_VIEW)
            .await
        {
            Ok(employee) => Some(employee.employee_id),
            Err(
                crate::WorkforceError::Unauthorized
                | crate::WorkforceError::NotFound
                | crate::WorkforceError::NotLinked,
            ) if reviewer => None,
            Err(error) => return Err(error.into()),
        };
        let row=client.query_opt(&format!("SELECT {COLUMNS} FROM people_leave_applications WHERE tenant_id=$1 AND company_id=$2 AND id=$3 AND (employee_id=$4 OR ($5 AND status='pending'))"),&[&scope.tenant(),&company_id,&id,&owner,&reviewer]).await.map_err(Error::from)?.ok_or(LeaveError::NotFound)?;
        let request = read(&row)?;
        self.current_company(&**client, scope, company_id).await?;
        let employees = self
            .workforce
            .employees_by_ids(&**client, scope, company_id, &[request.employee_id])
            .await?
            .require_current()
            .map_err(|e| LeaveError::NotCurrent(e.0))?;
        let employee_name = employees
            .iter()
            .find(|e| e.reference.stable_id == request.employee_id.to_string())
            .map(|e| format!("{} ({})", e.display_name, e.employee_number));
        Ok(PendingRequest {
            request,
            employee_name,
        })
    }
    /// Pending requests (at most 200), earliest start first, for an approver.
    pub async fn pending_requests(
        &self,
        scope: &Scope,
        company_id: i64,
    ) -> Result<Vec<PendingRequest>> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, REQUESTS_APPROVE).await?;
        self.current_company(&**client, scope, company_id).await?;
        let requests: Vec<Request> = client.query(&format!("SELECT {COLUMNS} FROM people_leave_applications WHERE tenant_id=$1 AND company_id=$2 AND status='pending' ORDER BY starts_on, id LIMIT 200"), &[&scope.tenant(), &company_id]).await.map_err(Error::from)?.iter().map(read).collect::<Result<_>>()?;
        let mut ids: Vec<i64> = requests.iter().map(|r| r.employee_id).collect();
        ids.sort_unstable();
        ids.dedup();
        let employees = self
            .workforce
            .employees_by_ids(&**client, scope, company_id, &ids)
            .await?
            .require_current()
            .map_err(|e| LeaveError::NotCurrent(e.0))?;
        Ok(requests
            .into_iter()
            .map(|request| {
                let employee_name = employees
                    .iter()
                    .find(|e| e.reference.stable_id == request.employee_id.to_string())
                    .map(|e| format!("{} ({})", e.display_name, e.employee_number));
                PendingRequest {
                    request,
                    employee_name,
                }
            })
            .collect())
    }
    /// Pending or approved type IDs overlapping the inclusive Payroll window.
    pub async fn requested_type_ids(
        &self,
        scope: &Scope,
        company_id: i64,
        from: Date,
        to: Date,
    ) -> Result<Vec<i64>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.current_company(&**client, scope, company_id).await?;
        Ok(client.query("SELECT DISTINCT leave_type_id FROM people_leave_applications WHERE tenant_id=$1 AND company_id=$2 AND status IN ('pending','approved') AND starts_on <= $4::text::date AND ends_on >= $3::text::date ORDER BY leave_type_id", &[&scope.tenant(), &company_id, &from.to_string(), &to.to_string()]).await.map_err(Error::from)?.iter().map(|r| r.get(0)).collect())
    }
    /// Submits for the signed-in employee, re-proved under the employment lock.
    /// Identical key replays return the saved request without new audit rows.
    pub async fn submit_request(
        &self,
        scope: &Scope,
        company_id: i64,
        new: &NewRequest,
    ) -> Result<Request> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .begin(
                &mut client,
                scope,
                company_id,
                "people.leave.submit_request",
            )
            .await?;
        let result = self
            .submit_request_in(&mut tx, scope, company_id, new)
            .await?;
        if result.created {
            tx.commit().await?;
        } else {
            tx.rollback().await?;
        }
        Ok(result.request)
    }
    pub(super) async fn submit_request_in(
        &self,
        tx: &mut MutationTransaction<'_>,
        scope: &Scope,
        company_id: i64,
        new: &NewRequest,
    ) -> Result<SubmittedRequest> {
        crate::entries::serialize(tx.sql(), scope, company_id).await?;
        bilibi_company::require_writable(tx.sql(), scope, company_id, true).await?;
        let new = normalize(new)?;
        let me = self
            .workforce
            .lock_self_employee(tx, scope, company_id, SELF_VIEW)
            .await?;
        let tenant = scope.tenant();
        if let Some(row) = tx.sql().query_opt(&format!("SELECT {COLUMNS} FROM people_leave_applications WHERE tenant_id=$1 AND company_id=$2 AND employee_id=$3 AND request_key=$4"), &[&tenant, &company_id, &me.employee_id, &new.request_key]).await.map_err(Error::from)? {
            let existing = read(&row)?;
            if existing.leave_type_id != new.leave_type_id || existing.starts_on != new.starts_on || existing.ends_on != new.ends_on || existing.day_part != new.day_part || existing.reason != new.reason || new.hours.is_some_and(|h| h != existing.quantity) { return Err(LeaveError::RequestKeyConflict); }
            return Ok(SubmittedRequest {
                request: existing,
                created: false,
            });
        }
        let leave_type = types::lock_active(tx.sql(), tenant, company_id, new.leave_type_id)
            .await?
            .ok_or(LeaveError::NotFound)?;
        if (leave_type.unit == "hour") != (new.day_part == "hours") {
            return Err(LeaveError::InvalidRequest);
        }
        let company = self.current_company(tx.sql(), scope, company_id).await?;
        let rules = self.read_rules(tx.sql(), scope, &company).await?;
        let request_rules = self.read_request_rules(tx.sql(), scope, &company).await?;
        let today = self.today_on(tx.sql(), scope, &company).await?;
        let year = rules.leave_year(new.starts_on);
        if rules.leave_year(new.ends_on) != year {
            return Err(LeaveError::SpansLeaveYears);
        }
        if new.starts_on.duration_since(today).as_secs() / 86400
            < -i64::from(request_rules.backdate_days)
        {
            return Err(LeaveError::TooFarBack);
        }
        if ledger::closed(
            tx.sql(),
            tenant,
            company_id,
            me.employee_id,
            leave_type.id,
            year,
        )
        .await?
        {
            return Err(LeaveError::YearClosed);
        }
        let exceptions = self
            .reference_data
            .list_calendar_exceptions(scope, company_id)
            .await
            .map_err(|_| LeaveError::Unavailable)?;
        let mut days = Vec::new();
        let mut date = new.starts_on;
        loop {
            if request_rules
                .working_weekdays
                .contains(&i32::from(date.weekday().to_monday_one_offset()))
                && !exceptions.iter().any(|e| e.on_date == date)
            {
                days.push(CountedDate {
                    on_date: date,
                    am: new.day_part != "pm",
                    pm: new.day_part != "am",
                    quantity: match new.day_part.as_str() {
                        "full" => Decimal::new(100, 2),
                        "hours" => new.hours.ok_or(LeaveError::InvalidRequest)?,
                        _ => Decimal::new(50, 2),
                    },
                });
            }
            if date == new.ends_on {
                break;
            }
            date = date.tomorrow().map_err(|_| LeaveError::InvalidRequest)?;
        }
        if days.is_empty() {
            return Err(LeaveError::NoWorkingDays);
        }
        for day in &days {
            if tx.sql().query_opt("SELECT id FROM people_leave_application_dates WHERE tenant_id=$1 AND company_id=$2 AND employee_id=$3 AND active AND on_date=$4::text::date AND ((am AND $5) OR (pm AND $6)) LIMIT 1", &[&tenant, &company_id, &me.employee_id, &day.on_date.to_string(), &day.am, &day.pm]).await.map_err(Error::from)?.is_some() { return Err(LeaveError::OverlappingRequest); }
        }
        let quantity = days.iter().map(|d| d.quantity).sum();
        covered(
            tx.sql(),
            scope,
            company_id,
            me.employee_id,
            &leave_type,
            year,
            quantity,
            0,
        )
        .await?;
        let employee_id = me.employee_id;
        let actor = scope.actor().user();
        let request = tx.mutate(move |sql| Box::pin(async move {
            let row = sql.query_one(&format!("INSERT INTO people_leave_applications (tenant_id, company_id, employee_id, leave_type_id, leave_year, starts_on, ends_on, day_part, quantity, unit, status, reason, request_key, requested_by_user_id) VALUES ($1,$2,$3,$4,$5,$6::text::date,$7::text::date,$8,$9,$10,'pending',$11,$12,$13) RETURNING {COLUMNS}"), &[&tenant, &company_id, &employee_id, &leave_type.id, &year, &new.starts_on.to_string(), &new.ends_on.to_string(), &new.day_part, &quantity, &leave_type.unit, &new.reason, &new.request_key, &actor]).await?;
            let request = read(&row).map_err(|_| Error::Internal)?;
            let mut changes = vec![Change { kind: "people_leave_application".into(), id: request.id, old: json!({}), new: json!(request) }];
            for day in days {
                let row = sql.query_one("INSERT INTO people_leave_application_dates (tenant_id,company_id,employee_id,request_id,on_date,am,pm,quantity,active) VALUES ($1,$2,$3,$4,$5::text::date,$6,$7,$8,true) RETURNING id,to_jsonb(people_leave_application_dates) AS facts", &[&tenant,&company_id,&employee_id,&request.id,&day.on_date.to_string(),&day.am,&day.pm,&day.quantity]).await?;
                changes.push(Change { kind: "people_leave_application_date".into(), id: row.get("id"), old: json!({}), new: row.get("facts") });
            }
            changes.push(event(sql, tenant, company_id, &request, None, actor, None).await?);
            Ok((request, changes))
        })).await?;
        Ok(SubmittedRequest {
            request,
            created: true,
        })
    }
    /// Approves or rejects a pending request. The requester and linked subject
    /// cannot decide; rejection requires a note. Approval rechecks cover.
    pub async fn decide_request(
        &self,
        scope: &Scope,
        company_id: i64,
        request_id: i64,
        decision: Decision,
        note: Option<&str>,
    ) -> Result<Request> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .begin(
                &mut client,
                scope,
                company_id,
                "people.leave.decide_request",
            )
            .await?;
        let result = self
            .decide_request_in(&mut tx, scope, company_id, request_id, decision, note)
            .await?;
        tx.commit().await?;
        Ok(result)
    }
    pub(super) async fn decide_request_in(
        &self,
        tx: &mut MutationTransaction<'_>,
        scope: &Scope,
        company_id: i64,
        request_id: i64,
        decision: Decision,
        note: Option<&str>,
    ) -> Result<Request> {
        crate::entries::serialize(tx.sql(), scope, company_id).await?;
        bilibi_company::require_writable(tx.sql(), scope, company_id, true).await?;
        let note = input::optional_text(note, 500).map_err(|_| LeaveError::InvalidDecision)?;
        if decision == Decision::Reject && note.is_none() {
            return Err(LeaveError::NoteRequired);
        }
        authorization::authorize(tx.sql(), scope, company_id, REQUESTS_APPROVE).await?;
        let request = lock(tx.sql(), scope, company_id, request_id).await?;
        if request.status != "pending" {
            return Err(LeaveError::NotPending);
        }
        if request.requested_by_user_id == scope.actor().user() {
            return Err(LeaveError::SelfApproval);
        }
        match self
            .workforce
            .self_employee(tx.sql(), scope, company_id)
            .await
        {
            Ok(me) if me.employee_id == request.employee_id => {
                return Err(LeaveError::SelfApproval)
            }
            Err(crate::WorkforceError::NotLinked) | Ok(_) => (),
            Err(e) => return Err(e.into()),
        }
        if decision == Decision::Approve {
            self.current_employee(tx.sql(), scope, company_id, request.employee_id)
                .await
                .map_err(|e| match e {
                    LeaveError::NotCurrent(_) | LeaveError::NotFound => {
                        LeaveError::EmployeeUnavailable
                    }
                    e => e,
                })?;
            let leave_type =
                types::lock_active(tx.sql(), scope.tenant(), company_id, request.leave_type_id)
                    .await?
                    .ok_or(LeaveError::LeaveTypeUnavailable)?;
            if ledger::closed(
                tx.sql(),
                scope.tenant(),
                company_id,
                request.employee_id,
                request.leave_type_id,
                request.leave_year,
            )
            .await?
            {
                return Err(LeaveError::YearClosed);
            }
            covered(
                tx.sql(),
                scope,
                company_id,
                request.employee_id,
                &leave_type,
                request.leave_year,
                request.quantity,
                request.id,
            )
            .await?;
            write_entry(tx, scope, company_id, &request, "taken", -request.quantity).await?;
        }
        let after = transition(
            tx,
            scope,
            company_id,
            request,
            if decision == Decision::Approve {
                "approved"
            } else {
                "rejected"
            },
            note,
        )
        .await?;
        Ok(after)
    }
    /// Cancels one's own pending request at any time, or approved leave before
    /// its start. An approved cancellation restores the taken quantity.
    pub async fn cancel_request(
        &self,
        scope: &Scope,
        company_id: i64,
        request_id: i64,
    ) -> Result<Request> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .begin(
                &mut client,
                scope,
                company_id,
                "people.leave.cancel_request",
            )
            .await?;
        let result = self
            .cancel_request_in(&mut tx, scope, company_id, request_id)
            .await?;
        tx.commit().await?;
        Ok(result)
    }
    pub(super) async fn cancel_request_in(
        &self,
        tx: &mut MutationTransaction<'_>,
        scope: &Scope,
        company_id: i64,
        request_id: i64,
    ) -> Result<Request> {
        crate::entries::serialize(tx.sql(), scope, company_id).await?;
        bilibi_company::require_writable(tx.sql(), scope, company_id, true).await?;
        let me = self
            .workforce
            .lock_self_employee(tx, scope, company_id, SELF_VIEW)
            .await?;
        let request = lock(tx.sql(), scope, company_id, request_id).await?;
        if request.employee_id != me.employee_id {
            return Err(LeaveError::NotFound);
        }
        match request.status.as_str() {
            "pending" => (),
            "approved" => {
                let company = self.current_company(tx.sql(), scope, company_id).await?;
                if request.starts_on <= self.today_on(tx.sql(), scope, &company).await? {
                    return Err(LeaveError::NotCancellable);
                }
                if ledger::closed(
                    tx.sql(),
                    scope.tenant(),
                    company_id,
                    request.employee_id,
                    request.leave_type_id,
                    request.leave_year,
                )
                .await?
                {
                    return Err(LeaveError::YearClosed);
                }
                write_entry(
                    tx,
                    scope,
                    company_id,
                    &request,
                    "cancelled",
                    request.quantity,
                )
                .await?;
            }
            _ => return Err(LeaveError::NotCancellable),
        }
        let after = transition(tx, scope, company_id, request, "cancelled", None).await?;
        Ok(after)
    }
}
pub(super) fn normalize(new: &NewRequest) -> Result<NewRequest> {
    let span = new.ends_on.duration_since(new.starts_on).as_secs() / 86400;
    if new.leave_type_id <= 0
        || !(0..366).contains(&span)
        || !["full", "am", "pm", "hours"].contains(&new.day_part.as_str())
        || (new.day_part != "full" && span != 0)
        || !(1..=160).contains(&new.request_key.len())
    {
        return Err(LeaveError::InvalidRequest);
    }
    let hours = if new.day_part == "hours" {
        Some(
            new.hours
                .and_then(input::bounded_quantity)
                .filter(|h| *h > Decimal::ZERO && *h <= Decimal::from(24))
                .ok_or(LeaveError::InvalidRequest)?,
        )
    } else {
        None
    };
    Ok(NewRequest {
        hours,
        reason: input::optional_text(new.reason.as_deref(), 500)
            .map_err(|_| LeaveError::InvalidRequest)?,
        ..new.clone()
    })
}
async fn lock(
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company: i64,
    id: i64,
) -> Result<Request> {
    let row = client.query_opt(&format!("SELECT {COLUMNS} FROM people_leave_applications WHERE tenant_id=$1 AND company_id=$2 AND id=$3 FOR UPDATE"), &[&scope.tenant(), &company, &id]).await.map_err(Error::from)?.ok_or(LeaveError::NotFound)?;
    read(&row)
}
#[allow(clippy::too_many_arguments)]
async fn covered(
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company: i64,
    employee: i64,
    leave_type: &crate::LeaveType,
    year: i32,
    quantity: Decimal,
    except: i64,
) -> Result<()> {
    if !leave_type.balance_required {
        return Ok(());
    }
    let row = client.query_one("SELECT (SELECT coalesce(sum(quantity),0) FROM people_leave_ledger_entries WHERE tenant_id=$1 AND company_id=$2 AND employee_id=$3 AND leave_type_id=$4 AND leave_year=$5) - (SELECT coalesce(sum(quantity),0) FROM people_leave_applications WHERE tenant_id=$1 AND company_id=$2 AND employee_id=$3 AND leave_type_id=$4 AND leave_year=$5 AND status='pending' AND id<>$6) AS available", &[&scope.tenant(), &company, &employee, &leave_type.id, &year, &except]).await.map_err(Error::from)?;
    if row.get::<_, Decimal>("available") < quantity {
        Err(LeaveError::InsufficientBalance)
    } else {
        Ok(())
    }
}
async fn write_entry(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company: i64,
    request: &Request,
    kind: &str,
    quantity: Decimal,
) -> Result<()> {
    ledger::insert(
        tx,
        scope.tenant(),
        company,
        vec![ledger::NewRow {
            employee_id: request.employee_id,
            leave_type_id: request.leave_type_id,
            leave_year: request.leave_year,
            entry_type: kind.into(),
            quantity,
            unit: request.unit.clone(),
            policy_id: None,
            policy_version: None,
            occurred_on: request.starts_on,
            source: "request".into(),
            entry_key: format!("{kind}:{}", request.id),
            actor_user_id: scope.actor().user(),
            note: None,
        }],
    )
    .await?;
    Ok(())
}
async fn event(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company: i64,
    request: &Request,
    from: Option<&str>,
    actor: i64,
    note: Option<&str>,
) -> bilibi_database::Result<Change> {
    let row = client.query_one("INSERT INTO people_leave_request_events (tenant_id,company_id,request_id,from_status,to_status,actor_user_id,note,occurred_at) VALUES ($1,$2,$3,$4,$5,$6,$7,clock_timestamp()) RETURNING id,to_jsonb(people_leave_request_events) AS facts", &[&tenant,&company,&request.id,&from,&request.status,&actor,&note]).await?;
    Ok(Change {
        kind: "people_leave_request_event".into(),
        id: row.get("id"),
        old: json!({}),
        new: row.get("facts"),
    })
}
async fn transition(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company: i64,
    before: Request,
    status: &str,
    note: Option<String>,
) -> Result<Request> {
    let tenant = scope.tenant();
    let actor = scope.actor().user();
    let status = status.to_owned();
    Ok(tx.mutate(move |sql| Box::pin(async move {
        let statement = if status == "cancelled" { "UPDATE people_leave_applications SET status=$4,cancelled_by_user_id=$5,cancelled_at=clock_timestamp(),updated_at=now() AT TIME ZONE 'UTC' WHERE tenant_id=$1 AND company_id=$2 AND id=$3" } else { "UPDATE people_leave_applications SET status=$4,decided_by_user_id=$5,decided_at=clock_timestamp(),decision_note=$6,updated_at=now() AT TIME ZONE 'UTC' WHERE tenant_id=$1 AND company_id=$2 AND id=$3" };
        let base: &[&(dyn tokio_postgres::types::ToSql + Sync)] = &[&tenant,&company,&before.id,&status,&actor];
        let mut params = base.to_vec(); if status != "cancelled" { params.push(&note); }
        let row = sql.query_one(&format!("{statement} RETURNING {COLUMNS}"), &params).await?;
        let after = read(&row).map_err(|_| Error::Internal)?;
        let mut changes = vec![Change { kind: "people_leave_application".into(), id: after.id, old: json!(before), new: json!(after) }];
        if status == "cancelled" || status == "rejected" {
            let rows = sql.query("WITH previous AS (SELECT id,to_jsonb(d) AS old FROM people_leave_application_dates d WHERE tenant_id=$1 AND company_id=$2 AND request_id=$3 AND active), updated AS (UPDATE people_leave_application_dates d SET active=false FROM previous p WHERE d.id=p.id AND d.tenant_id=$1 AND d.company_id=$2 RETURNING d.id,to_jsonb(d) AS new,p.old) SELECT * FROM updated", &[&tenant,&company,&after.id]).await?;
            for row in rows { changes.push(Change { kind: "people_leave_application_date".into(), id: row.get("id"), old: row.get("old"), new: row.get("new") }); }
        }
        changes.push(event(sql,tenant,company,&after,Some(&before.status),actor,note.as_deref()).await?);
        Ok((after,changes))
    })).await?)
}
