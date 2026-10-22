//! Bilimbi Attendance.Adjustments and self_clock: missing punches resolve the signed-in employee again under lock.
use crate::{
    access, clock, locations,
    rosters::{action, decode, lock},
    rules, Attendance, AttendanceError as Refusal, ClockEvent, ClockInput, EventType, FieldError,
    Result, ADJUSTMENTS_APPROVE, SELF_VIEW,
};
use bilibi_database::{Change, Error, MutationTransaction};
use bilibi_tenancy::Scope;
use jiff::{civil::DateTime, Timestamp};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio_postgres::GenericClient;

#[derive(Clone, Debug)]
pub struct NewAdjustment {
    pub request_key: String,
    pub event_type: EventType,
    pub local_at: String,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AdjustmentRequest {
    pub id: i64,
    pub employee_id: i64,
    pub request_key: String,
    pub event_type: String,
    pub proposed_at: String,
    pub on_date: String,
    pub timezone: String,
    pub reason: String,
    pub status: String,
    pub requested_by_user_id: i64,
    pub decided_by_user_id: Option<i64>,
    pub decided_at: Option<String>,
    pub decision_note: Option<String>,
    pub applied_clock_event_id: Option<i64>,
}
#[derive(Clone, Debug, Serialize)]
pub struct PendingAdjustment {
    pub request: AdjustmentRequest,
    pub employee_name: Option<String>,
}
async fn request(
    tx: &MutationTransaction<'_>,
    scope: &Scope,
    company: i64,
    id: i64,
) -> Result<Value> {
    tx.sql().query_opt("SELECT to_jsonb(r) FROM people_attendance_corrections r WHERE tenant_id=$1 AND company_id=$2 AND id=$3 FOR UPDATE",&[&scope.tenant(),&company,&id]).await.map_err(Error::from)?.map(|r|r.get(0)).ok_or(Refusal::NotFound)
}
async fn decide(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company: i64,
    old: Value,
    status: &'static str,
    note: Option<String>,
    event: Option<i64>,
) -> Result<AdjustmentRequest> {
    let id = old["id"].as_i64().ok_or(Refusal::NotFound)?;
    let tenant = scope.tenant();
    let actor = scope.user();
    let saved:Value=tx.mutate(move|sql|Box::pin(async move{
  let new:Value=sql.query_one("UPDATE people_attendance_corrections r SET status=$4,decided_by_user_id=$5,decided_at=date_trunc('second',now()),decision_note=$6,applied_clock_event_id=$7,updated_at=now() WHERE tenant_id=$1 AND company_id=$2 AND id=$3 RETURNING to_jsonb(r)",&[&tenant,&company,&id,&status,&actor,&note,&event]).await?.get(0);
  Ok((new.clone(),vec![Change{kind:"people_attendance_correction".into(),id,old,new}]))
 })).await?;
    decode(saved)
}
impl Attendance {
    /// Browser self-clock; only in/out, the web source and the current server instant are accepted.
    pub async fn self_clock(
        &self,
        scope: &Scope,
        company_id: i64,
        event_type: EventType,
        key: &str,
        latitude: Option<Decimal>,
        longitude: Option<Decimal>,
    ) -> Result<ClockEvent> {
        if !matches!(event_type, EventType::In | EventType::Out) {
            return Err(Refusal::InvalidEvent);
        }
        if scope.impersonator().is_some() || scope.agent().is_some() {
            return Err(Refusal::SelfClockUnavailable);
        }
        let mut c = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .mutations
            .begin(
                &mut c,
                scope.mutation_context("people.attendance.self_clock"),
            )
            .await?;
        let me = self
            .workforce
            .lock_self_employee(&tx, scope, company_id, SELF_VIEW)
            .await?;
        // Recheck the capability after waiting for the writable company/employment locks.
        bilibi_people_workforce::authorization::authorize(tx.sql(), scope, company_id, SELF_VIEW)
            .await?;
        let company = access::current_company(&self.workforce, tx.sql(), scope, company_id).await?;
        let rules = rules::read(tx.sql(), &self.settings, scope, &company).await?;
        if !rules.self_clock_enabled {
            return Err(Refusal::SelfClockUnavailable);
        }
        let input = ClockInput {
            event_key: key.into(),
            event_type,
            source: "web".into(),
            occurred_at: Timestamp::now(),
            latitude,
            longitude,
        };
        let event = clock::normalize(&input, &rules.timezone)?;
        let location = locations::place(
            tx.sql(),
            scope,
            company_id,
            event.point(),
            rules.location_required,
        )
        .await?;
        let saved = clock::write(
            &mut tx,
            scope,
            company_id,
            me.employee_id,
            event,
            location,
            rules.max_shift_hours,
        )
        .await?;
        tx.commit().await?;
        Ok(saved)
    }
    pub async fn submit_adjustment(
        &self,
        scope: &Scope,
        company_id: i64,
        new: &NewAdjustment,
    ) -> Result<AdjustmentRequest> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .mutations
            .begin(
                &mut c,
                scope.mutation_context("people.attendance.submit_adjustment"),
            )
            .await?;
        let me = self
            .workforce
            .lock_self_employee(&tx, scope, company_id, SELF_VIEW)
            .await?;
        bilibi_people_workforce::authorization::authorize(tx.sql(), scope, company_id, SELF_VIEW)
            .await?;
        let company = access::current_company(&self.workforce, tx.sql(), scope, company_id).await?;
        let rules = rules::read(tx.sql(), &self.settings, scope, &company).await?;
        let text = if new.local_at.len() == 16 {
            format!("{}:00", new.local_at)
        } else {
            new.local_at.clone()
        };
        let local: DateTime = text.parse().map_err(|_| Refusal::InvalidTime)?;
        let tz = jiff::tz::TimeZone::get(&rules.timezone).map_err(|_| Refusal::InvalidTime)?;
        // Reject ambiguous and nonexistent local times, as DateTime.from_naive does in Bilimbi.
        let at = tz
            .to_ambiguous_zoned(local)
            .unambiguous()
            .map_err(|_| Refusal::InvalidTime)?
            .timestamp();
        let at = Timestamp::from_second(at.as_second()).map_err(|_| Refusal::InvalidTime)?;
        let today = Timestamp::now().to_zoned(tz.clone()).date();
        let date = at.to_zoned(tz).date();
        if at > Timestamp::now() {
            return Err(Refusal::FutureTime);
        }
        if today
            .since(date)
            .map_err(|_| Refusal::InvalidTime)?
            .get_days()
            >= rules.adjustment_window_days
        {
            return Err(Refusal::OutsideWindow);
        }
        let mut errors = Vec::new();
        if new.request_key.is_empty() {
            errors.push(FieldError::new("request_key", "can't be blank"));
        } else if new.request_key.len() > 160 {
            errors.push(FieldError::new(
                "request_key",
                "should be at most 160 character(s)",
            ));
        }
        if !matches!(new.event_type, EventType::In | EventType::Out) {
            errors.push(FieldError::new("event_type", "is invalid"));
        }
        let reason = new.reason.trim().to_owned();
        if reason.is_empty() {
            errors.push(FieldError::new("reason", "can't be blank"));
        } else if reason.chars().count() > 500 {
            errors.push(FieldError::new(
                "reason",
                "should be at most 500 character(s)",
            ));
        }
        if !errors.is_empty() {
            return Err(Refusal::Invalid(errors));
        }
        if let Some(row)=tx.sql().query_opt("SELECT to_jsonb(r) FROM people_attendance_corrections r WHERE tenant_id=$1 AND company_id=$2 AND request_key=$3",&[&scope.tenant(),&company_id,&new.request_key]).await.map_err(Error::from)?{
   let saved:AdjustmentRequest=decode(row.get(0))?;
   if saved.employee_id!=me.employee_id||saved.event_type!=new.event_type.as_str()||saved.proposed_at.parse::<Timestamp>().ok()!=Some(at){return Err(Refusal::RequestKeyConflict);}
   return Ok(saved);
  }
        if tx.sql().query_opt("SELECT 1 FROM people_attendance_corrections WHERE tenant_id=$1 AND company_id=$2 AND employee_id=$3 AND event_type=$4 AND proposed_at=$5::text::timestamptz AND status IN ('pending','approved')",&[&scope.tenant(),&company_id,&me.employee_id,&new.event_type.as_str(),&at.to_string()]).await.map_err(Error::from)?.is_some(){return Err(Refusal::DuplicateRequest);}
        let tenant = scope.tenant();
        let actor = scope.user();
        let key = new.request_key.clone();
        let kind = new.event_type.as_str();
        let timezone = rules.timezone;
        let saved:Value=tx.mutate(move|sql|Box::pin(async move{
   let value:Value=sql.query_one("INSERT INTO people_attendance_corrections AS r (tenant_id,company_id,employee_id,request_key,event_type,proposed_at,on_date,timezone,reason,requested_by_user_id) VALUES($1,$2,$3,$4,$5,$6::text::timestamptz,$7::text::date,$8,$9,$10) RETURNING to_jsonb(r)",&[&tenant,&company_id,&me.employee_id,&key,&kind,&at.to_string(),&date.to_string(),&timezone,&reason,&actor]).await?.get(0);
   Ok((value.clone(),vec![Change{kind:"people_attendance_correction".into(),id:value["id"].as_i64().ok_or(Error::Internal)?,old:json!({}),new:value}]))
  })).await?;
        tx.commit().await?;
        decode(saved)
    }
    pub async fn cancel_adjustment(
        &self,
        scope: &Scope,
        company_id: i64,
        id: i64,
    ) -> Result<AdjustmentRequest> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .mutations
            .begin(
                &mut c,
                scope.mutation_context("people.attendance.cancel_adjustment"),
            )
            .await?;
        let me = self
            .workforce
            .lock_self_employee(&tx, scope, company_id, SELF_VIEW)
            .await?;
        bilibi_people_workforce::authorization::authorize(tx.sql(), scope, company_id, SELF_VIEW)
            .await?;
        let old = request(&tx, scope, company_id, id).await?;
        if old["employee_id"].as_i64() != Some(me.employee_id) {
            return Err(Refusal::NotFound);
        }
        if old["status"] != "pending" {
            return Err(Refusal::NotPending);
        }
        let saved = decide(&mut tx, scope, company_id, old, "cancelled", None, None).await?;
        tx.commit().await?;
        Ok(saved)
    }
    pub async fn decide_adjustment(
        &self,
        scope: &Scope,
        company_id: i64,
        id: i64,
        decision: &str,
        note: Option<&str>,
    ) -> Result<AdjustmentRequest> {
        let status = match decision {
            "approve" => "approved",
            "reject" => "rejected",
            _ => return Err(Refusal::InvalidDecision),
        };
        let note = note
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        if status == "rejected" && note.is_none() {
            return Err(Refusal::NoteRequired);
        }
        if note.as_ref().is_some_and(|s| s.len() > 500) {
            return Err(Refusal::NoteTooLong);
        }
        let mut c = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .mutations
            .begin(
                &mut c,
                scope.mutation_context("people.attendance.decide_adjustment"),
            )
            .await?;
        lock(&tx, scope, company_id, ADJUSTMENTS_APPROVE).await?;
        let old = request(&tx, scope, company_id, id).await?;
        let req: AdjustmentRequest = decode(old.clone())?;
        if req.status != "pending" {
            return Err(Refusal::NotPending);
        }
        if req.requested_by_user_id == scope.user()
            || self
                .workforce
                .owns_employee(&tx, scope, company_id, req.employee_id, ADJUSTMENTS_APPROVE)
                .await?
        {
            return Err(Refusal::SelfApproval);
        }
        let event = if status == "approved" {
            access::require_employee(
                &self.workforce,
                tx.sql(),
                scope,
                company_id,
                req.employee_id,
            )
            .await
            .map_err(|e| match e {
                Refusal::NotFound => Refusal::EmployeeUnavailable,
                other => other,
            })?;
            let company =
                access::current_company(&self.workforce, tx.sql(), scope, company_id).await?;
            let rules = rules::read(tx.sql(), &self.settings, scope, &company).await?;
            let input = ClockInput::new(
                format!("request:{}", req.id),
                EventType::parse(&req.event_type).ok_or(Refusal::InvalidEvent)?,
                "adjustment",
                req.proposed_at.parse().map_err(|_| Refusal::InvalidTime)?,
            );
            let event = clock::normalize(&input, &rules.timezone)?;
            Some(
                clock::write(
                    &mut tx,
                    scope,
                    company_id,
                    req.employee_id,
                    event,
                    None,
                    rules.max_shift_hours,
                )
                .await?
                .id,
            )
        } else {
            None
        };
        let saved = decide(&mut tx, scope, company_id, old, status, note, event).await?;
        action(&tx,scope,if status=="approved"{"people.attendance.adjustment_approved"}else{"people.attendance.adjustment_rejected"},json!({"company_id":company_id,"request_id":id,"employee_id":saved.employee_id,"event_type":saved.event_type,"proposed_at":saved.proposed_at,"applied_clock_event_id":saved.applied_clock_event_id})).await?;
        tx.commit().await?;
        Ok(saved)
    }
    pub async fn self_adjustments(
        &self,
        scope: &Scope,
        company_id: i64,
    ) -> Result<Vec<AdjustmentRequest>> {
        let me = self
            .workforce
            .authorize_self(scope, company_id, SELF_VIEW)
            .await?;
        let c = self.db.get().await.map_err(Error::from)?;
        c.query("SELECT to_jsonb(r) FROM people_attendance_corrections r WHERE tenant_id=$1 AND company_id=$2 AND employee_id=$3 ORDER BY inserted_at DESC,id DESC LIMIT 20",&[&scope.tenant(),&company_id,&me.employee_id]).await.map_err(Error::from)?.into_iter().map(|r|decode(r.get(0))).collect()
    }
    pub async fn pending_adjustments(
        &self,
        scope: &Scope,
        company_id: i64,
    ) -> Result<Vec<PendingAdjustment>> {
        let requests: Vec<AdjustmentRequest> = {
            let c = self.db.get().await.map_err(Error::from)?;
            bilibi_people_workforce::authorization::authorize(
                &**c,
                scope,
                company_id,
                ADJUSTMENTS_APPROVE,
            )
            .await?;
            access::current_company(&self.workforce, &**c, scope, company_id).await?;
            c.query("SELECT to_jsonb(r) FROM people_attendance_corrections r WHERE tenant_id=$1 AND company_id=$2 AND status='pending' ORDER BY proposed_at,id LIMIT 200",&[&scope.tenant(),&company_id]).await.map_err(Error::from)?.into_iter().map(|r|decode(r.get(0))).collect::<Result<_>>()?
        };
        let mut ids: Vec<_> = requests.iter().map(|r| r.employee_id).collect();
        ids.sort_unstable();
        ids.dedup();
        let employees = self
            .workforce
            .employees_by_ids(scope, company_id, &ids)
            .await?
            .require_current()?;
        Ok(requests
            .into_iter()
            .map(|request| {
                let employee_name = employees
                    .iter()
                    .find(|e| e.reference.stable_id == request.employee_id.to_string())
                    .map(|e| format!("{} ({})", e.display_name, e.employee_number));
                PendingAdjustment {
                    request,
                    employee_name,
                }
            })
            .collect())
    }
}
