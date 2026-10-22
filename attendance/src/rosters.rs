//! Bilimbi Attendance.Rosters: drafts and the separately published employee view.
use crate::workflow::{action, decode, lock};
use crate::{
    shifts, Attendance, AttendanceError as Refusal, Result, ShiftTemplate, ROSTER_MANAGE, SELF_VIEW,
};
use bilibi_database::{Change, Error};
use bilibi_tenancy::Scope;
use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
pub const MAX_ROSTER_DAYS: i32 = 31;
pub const MAX_ROSTER_EMPLOYEES: usize = 200;
pub const SELF_ROSTER_DAYS: i32 = 14;
#[derive(Clone, Copy, Debug)]
/// A working shift, rest day or planned removal.
pub enum RosterValue {
    Shift(i64),
    Rest,
    None,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
/// The planner value and its separate publication; dates are ISO text.
pub struct RosterEntry {
    pub id: i64,
    pub employee_id: i64,
    pub on_date: String,
    pub kind: String,
    pub shift_template_id: Option<i64>,
    pub published_kind: Option<String>,
    pub published_shift_template_id: Option<i64>,
    pub revision: i32,
}
impl RosterEntry {
    pub fn pending(&self) -> bool {
        self.kind != self.published_kind.as_deref().unwrap_or("none")
            || self.shift_template_id != self.published_shift_template_id
    }
}
#[derive(Clone, Debug, Serialize)]
/// A current employee selected for the planner grid.
pub struct RosterEmployee {
    pub id: i64,
    pub name: String,
    pub number: String,
}
#[derive(Clone, Debug, Serialize)]
/// A bounded planner grid; pending counts the whole company period.
pub struct Roster {
    pub dates: Vec<String>,
    pub employees: Vec<RosterEmployee>,
    pub truncated: bool,
    pub templates: Vec<ShiftTemplate>,
    pub entries: Vec<RosterEntry>,
    pub pending: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
/// One published employee day with the template's local times.
pub struct PublishedShift {
    pub on_date: String,
    pub kind: String,
    pub shift_code: Option<String>,
    pub shift_name: Option<String>,
    pub start_minute: Option<i32>,
    pub end_minute: Option<i32>,
    pub break_minutes: Option<i32>,
}
pub(crate) fn period(from: Date, days: i32) -> Result<Date> {
    if !(1..=MAX_ROSTER_DAYS).contains(&days) {
        return Err(Refusal::InvalidPeriod);
    }
    from.checked_add(jiff::Span::new().days(days - 1))
        .map_err(|_| Refusal::InvalidPeriod)
}
impl Attendance {
    /// Up to 31 days and 200 working employees matching the name/number search.
    pub async fn roster(
        &self,
        scope: &Scope,
        company_id: i64,
        from: Date,
        days: i32,
        query: Option<&str>,
    ) -> Result<Roster> {
        let to = period(from, days)?;
        {
            let c = self.db.get().await.map_err(Error::from)?;
            bilibi_people_workforce::authorization::authorize(
                &**c,
                scope,
                company_id,
                ROSTER_MANAGE,
            )
            .await?;
        }
        let mut matching = self
            .workforce
            .employees(scope, company_id)
            .await?
            .require_current()?;
        let needle = query.unwrap_or("").to_lowercase();
        matching.retain(|e| {
            e.display_name.to_lowercase().contains(&needle)
                || e.employee_number.to_lowercase().contains(&needle)
        });
        matching.sort_by(|a, b| {
            (&a.display_name, &a.employee_number).cmp(&(&b.display_name, &b.employee_number))
        });
        let truncated = matching.len() > MAX_ROSTER_EMPLOYEES;
        let employees: Vec<_> = matching
            .into_iter()
            .take(MAX_ROSTER_EMPLOYEES)
            .map(|e| {
                Ok(RosterEmployee {
                    id: e
                        .reference
                        .stable_id
                        .parse()
                        .map_err(|_| Refusal::NotFound)?,
                    name: e.display_name,
                    number: e.employee_number,
                })
            })
            .collect::<Result<_>>()?;
        let ids: Vec<_> = employees.iter().map(|e| e.id).collect();
        let c = self.db.get().await.map_err(Error::from)?;
        let entries = c
            .query(
                ROSTER_ENTRIES,
                &[
                    &scope.tenant(),
                    &company_id,
                    &ids,
                    &from.to_string(),
                    &to.to_string(),
                ],
            )
            .await
            .map_err(Error::from)?
            .into_iter()
            .map(|r| decode(r.get(0)))
            .collect::<Result<_>>()?;
        let pending = c
            .query_one(
                COUNT_PENDING,
                &[
                    &scope.tenant(),
                    &company_id,
                    &from.to_string(),
                    &to.to_string(),
                ],
            )
            .await
            .map_err(Error::from)?
            .get(0);
        Ok(Roster {
            dates: (0..days)
                .map(|i| {
                    from.checked_add(jiff::Span::new().days(i))
                        .expect("validated period")
                        .to_string()
                })
                .collect(),
            employees,
            truncated,
            templates: shifts::list(&**c, scope, company_id).await?,
            entries,
            pending,
        })
    }
    /// Plan a working value; clear an unpublished value immediately. Requires roster management.
    pub async fn plan_roster_entry(
        &self,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
        on_date: Date,
        value: RosterValue,
    ) -> Result<Option<RosterEntry>> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .mutations
            .begin(
                &mut c,
                scope.mutation_context("people.attendance.plan_roster_entry"),
            )
            .await?;
        lock(&tx, scope, company_id, ROSTER_MANAGE).await?;
        self.workforce
            .lock_working_employee(&tx, scope, company_id, employee_id)
            .await?;
        let (kind, template) = match value {
            RosterValue::Shift(id) => ("shift", Some(id)),
            RosterValue::Rest => ("rest", None),
            RosterValue::None => ("none", None),
        };
        if let Some(id) = template {
            if tx
                .sql()
                .query_opt(ACTIVE_SHIFT, &[&scope.tenant(), &company_id, &id])
                .await
                .map_err(Error::from)?
                .is_none()
            {
                return Err(Refusal::ShiftUnavailable);
            }
        }
        let old: Option<Value> = tx
            .sql()
            .query_opt(
                LOCK_ENTRY,
                &[
                    &scope.tenant(),
                    &company_id,
                    &employee_id,
                    &on_date.to_string(),
                ],
            )
            .await
            .map_err(Error::from)?
            .map(|r| r.get(0));
        if old.is_none() && kind == "none" {
            return Ok(None);
        }
        let delete = kind == "none" && old.as_ref().is_some_and(|v| v["published_kind"].is_null());
        let tenant = scope.tenant();
        let actor = scope.user();
        let date = on_date.to_string();
        let value = tx
            .mutate(move |sql| {
                Box::pin(async move {
                    let after: Value = if delete {
                        sql.execute(DELETE_DRAFT, &[&tenant, &company_id, &employee_id, &date])
                            .await?;
                        json!({})
                    } else {
                        sql.query_one(
                            UPSERT_DRAFT,
                            &[
                                &tenant,
                                &company_id,
                                &employee_id,
                                &date,
                                &kind,
                                &template,
                                &actor,
                            ],
                        )
                        .await?
                        .get(0)
                    };
                    let id = old.as_ref().unwrap_or(&after)["id"]
                        .as_i64()
                        .ok_or(Error::Internal)?;
                    Ok((
                        if delete { None } else { Some(after.clone()) },
                        vec![Change {
                            kind: "people_attendance_roster_entry".into(),
                            id,
                            old: old.unwrap_or(json!({})),
                            new: after,
                        }],
                    ))
                })
            })
            .await?;
        tx.commit().await?;
        value.map(decode).transpose()
    }
    /// Publish all pending values in an inclusive period of at most 31 days, with an audit action.
    pub async fn publish_roster(
        &self,
        scope: &Scope,
        company_id: i64,
        from: Date,
        to: Date,
    ) -> Result<usize> {
        let days = to
            .since(from)
            .map_err(|_| Refusal::InvalidPeriod)?
            .get_days()
            + 1;
        period(from, days)?;
        let mut c = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .mutations
            .begin(
                &mut c,
                scope.mutation_context("people.attendance.publish_roster"),
            )
            .await?;
        lock(&tx, scope, company_id, ROSTER_MANAGE).await?;
        let rows: Vec<Value> = tx
            .sql()
            .query(
                LOCK_PENDING,
                &[
                    &scope.tenant(),
                    &company_id,
                    &from.to_string(),
                    &to.to_string(),
                ],
            )
            .await
            .map_err(Error::from)?
            .into_iter()
            .map(|r| r.get(0))
            .collect();
        let count = rows.len();
        let tenant = scope.tenant();
        let actor = scope.user();
        if count > 0 {
            tx.mutate(move |sql| {
                Box::pin(async move {
                    let mut changes = Vec::new();
                    for old in rows {
                        let id = old["id"].as_i64().ok_or(Error::Internal)?;
                        let new = if old["kind"] == "none" {
                            sql.execute(DELETE_PUBLISHED, &[&tenant, &company_id, &id])
                                .await?;
                            json!({})
                        } else {
                            sql.query_one(PUBLISH_ENTRY, &[&tenant, &company_id, &id, &actor])
                                .await?
                                .get(0)
                        };
                        changes.push(Change {
                            kind: "people_attendance_roster_entry".into(),
                            id,
                            old,
                            new,
                        });
                    }
                    Ok(((), changes))
                })
            })
            .await?;
        }
        action(
            &tx,
            scope,
            "people.attendance.roster_published",
            json!({
            "company_id":company_id,
            "from":from.to_string(),
            "to":to.to_string(),
            "entries":count}
            ),
        )
        .await?;
        tx.commit().await?;
        Ok(count)
    }
    /// The linked employee's published days; the self page requests 14 days.
    pub async fn self_roster(
        &self,
        scope: &Scope,
        company_id: i64,
        from: Date,
        days: i32,
    ) -> Result<Vec<PublishedShift>> {
        let to = period(from, days)?;
        let me = self
            .workforce
            .authorize_self(scope, company_id, SELF_VIEW)
            .await?;
        let c = self.db.get().await.map_err(Error::from)?;
        c.query(
            PUBLISHED_SHIFTS,
            &[
                &scope.tenant(),
                &company_id,
                &me.employee_id,
                &from.to_string(),
                &to.to_string(),
            ],
        )
        .await
        .map_err(Error::from)?
        .into_iter()
        .map(|r| decode(r.get(0)))
        .collect()
    }
}

const ROSTER_ENTRIES: &str = concat!(
    "SELECT to_jsonb(e) FROM people_attendance_roster_entries e WHERE tenant_id=$1 AND ",
    "company_id=$2 AND employee_id=ANY($3) AND on_date BETWEEN $4::text::date AND $5::text::date ",
    "ORDER BY employee_id,on_date",
);

const COUNT_PENDING: &str = concat!(
    "SELECT count(*) FROM people_attendance_roster_entries WHERE tenant_id=$1 AND company_id=$2 AND ",
    "on_date BETWEEN $3::text::date AND $4::text::date AND (kind <> coalesce(published_kind,'none') ",
    "OR shift_template_id IS DISTINCT FROM published_shift_template_id)",
);

const ACTIVE_SHIFT: &str = concat!(
    "SELECT 1 FROM people_attendance_shift_definitions WHERE tenant_id=$1 AND company_id=$2 AND ",
    "id=$3 AND status='active'",
);

const LOCK_ENTRY: &str = concat!(
    "SELECT to_jsonb(e) FROM people_attendance_roster_entries e WHERE tenant_id=$1 AND ",
    "company_id=$2 AND employee_id=$3 AND on_date=$4::text::date FOR UPDATE",
);

const DELETE_DRAFT: &str = concat!(
    "DELETE FROM people_attendance_roster_entries WHERE tenant_id=$1 AND company_id=$2 AND ",
    "employee_id=$3 AND on_date=$4::text::date",
);

const UPSERT_DRAFT: &str = concat!(
    "INSERT INTO people_attendance_roster_entries AS e ",
    "(tenant_id,company_id,employee_id,on_date,kind,shift_template_id,updated_by_user_id) ",
    "VALUES($1,$2,$3,$4::text::date,$5,$6,$7) ON CONFLICT(company_id,employee_id,on_date) DO UPDATE ",
    "SET kind=$5,shift_template_id=$6,updated_by_user_id=$7,updated_at=now() RETURNING to_jsonb(e)",
);

const LOCK_PENDING: &str = concat!(
    "SELECT to_jsonb(e) FROM people_attendance_roster_entries e WHERE tenant_id=$1 AND ",
    "company_id=$2 AND on_date BETWEEN $3::text::date AND $4::text::date AND (kind <> ",
    "coalesce(published_kind,'none') OR shift_template_id IS DISTINCT FROM ",
    "published_shift_template_id) ORDER BY id FOR UPDATE",
);

const DELETE_PUBLISHED: &str =
    "DELETE FROM people_attendance_roster_entries WHERE tenant_id=$1 AND company_id=$2 AND id=$3";

const PUBLISH_ENTRY: &str = concat!(
    "UPDATE people_attendance_roster_entries e SET ",
    "published_kind=kind,published_shift_template_id=shift_template_id,published_at=date_trunc('second',now()),published_by_user_id=$4,revision=CASE ",
    "WHEN published_kind IS NULL THEN revision ELSE revision+1 END,updated_at=now() WHERE ",
    "tenant_id=$1 AND company_id=$2 AND id=$3 RETURNING to_jsonb(e)",
);

const PUBLISHED_SHIFTS: &str = concat!(
    "SELECT ",
    "jsonb_build_object('on_date',e.on_date,'kind',e.published_kind,'shift_code',t.code,'shift_name',t.name,'start_minute',t.start_minute,'end_minute',t.end_minute,'break_minutes',t.break_minutes) ",
    "FROM people_attendance_roster_entries e LEFT JOIN people_attendance_shift_definitions t ON ",
    "t.id=e.published_shift_template_id AND t.tenant_id=e.tenant_id AND t.company_id=e.company_id ",
    "WHERE e.tenant_id=$1 AND e.company_id=$2 AND e.employee_id=$3 AND e.on_date BETWEEN ",
    "$4::text::date AND $5::text::date AND e.published_kind IS NOT NULL ORDER BY e.on_date",
);
