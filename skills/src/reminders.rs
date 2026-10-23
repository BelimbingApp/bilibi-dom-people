//! Bilimbi Skills.Reminders: due rules, period claims and durable delivery.
use crate::{
    access, policy, rows, standing, DueReminder, Reminder, Result, Skills, SkillsError, Write,
    ASSESSMENTS_MANAGE,
};
use bilibi_database::Error;
use bilibi_notifications::Notice;
use bilibi_queue::execution::Execution;
use bilibi_tenancy::Scope;
use jiff::civil::Date;
use std::collections::HashMap;
use tokio_postgres::{GenericClient, Row};

const COLUMNS: &str = "id,rule,employee_id,skill_id,action_id,period_key,recipient_user_id,to_char(due_on,'YYYY-MM-DD') AS due_on,state,failure,to_char(sent_at,'YYYY-MM-DD\"T\"HH24:MI:SS') AS sent_at";
const KIND: &str = "people_skill_reminder";
const RETRY: &str = "period_key=ANY($4::text[]) AND (state='failed' OR (state='pending' AND inserted_at < (now() AT TIME ZONE 'UTC')-interval '15 minutes'))";

fn read(row: &Row) -> Result<Reminder> {
    Ok(Reminder {
        id: row.get("id"),
        rule: row.get("rule"),
        employee_id: row.get("employee_id"),
        skill_id: row.get("skill_id"),
        action_id: row.get("action_id"),
        period_key: row.get("period_key"),
        recipient_user_id: row.get("recipient_user_id"),
        due_on: rows::date(row, "due_on")?.ok_or(Error::Internal)?,
        state: row.get("state"),
        failure: row.get("failure"),
        sent_at: rows::datetime(row, "sent_at")?,
    })
}
fn week(as_of: Date) -> String {
    let iso = as_of.iso_week_date();
    format!("{}-W{:02}", iso.year(), iso.week())
}
fn month(as_of: Date) -> String {
    format!("{}-M{:02}", as_of.year(), as_of.month())
}
fn periods(as_of: Date) -> Vec<String> {
    vec![week(as_of), month(as_of)]
}
fn period(rule: &str, as_of: Date) -> String {
    if rule == "coverage_gap" {
        month(as_of)
    } else {
        week(as_of)
    }
}
fn item(
    rule: &str,
    employee: Option<i64>,
    skill: i64,
    action: Option<i64>,
    due_on: Date,
    mut recipients: Vec<i64>,
) -> DueReminder {
    recipients.sort_unstable();
    recipients.dedup();
    DueReminder {
        rule: rule.into(),
        employee_id: employee,
        skill_id: skill,
        action_id: action,
        due_on,
        recipients,
    }
}

pub(super) async fn compute(
    skills: &Skills,
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company: i64,
    as_of: Date,
) -> Result<Vec<DueReminder>> {
    let policy = policy::get(&skills.settings, client, scope, company).await?;
    let employees = skills
        .workforce
        .employees(scope, company)
        .await?
        .require_current()?;
    let mut supervisors = HashMap::new();
    for employee in &employees {
        let supervisor = employee
            .supervisor_reference
            .as_ref()
            .map(|r| r.stable_id.parse::<i64>().map_err(|_| Error::Internal))
            .transpose()?;
        supervisors.insert(access::employee_id(employee)?, supervisor);
    }
    let accounts = bilibi_user::company_accounts::list(client, scope, company).await?;
    let mut by_affiliation = HashMap::<i64, Vec<i64>>::new();
    let mut holders = Vec::new();
    for account in accounts {
        by_affiliation
            .entry(account.affiliation.get())
            .or_default()
            .push(account.id);
        // Audience discovery only; authority for every write remains the caller's
        // sealed scope, checked through Workforce Authorization.
        if bilibi_authz::effective_capabilities_of(client, scope, account.id)
            .await?
            .allowed
            .contains(ASSESSMENTS_MANAGE)
        {
            holders.push(account.id);
        }
    }
    let affiliations: Vec<i64> = by_affiliation.keys().copied().collect();
    let mut users = HashMap::<i64, Vec<i64>>::new();
    for batch in affiliations.chunks(bilibi_people_workforce::LOOKUP_LIMIT) {
        for (affiliation, employee) in skills
            .workforce
            .employment_links_on(client, scope, company, batch)
            .await?
            .require_current()?
        {
            if let Some(accounts) = by_affiliation.get(&affiliation) {
                users.insert(employee, accounts.clone());
            }
        }
    }
    let supervisor_users = |employee: i64| {
        supervisors
            .get(&employee)
            .copied()
            .flatten()
            .and_then(|id| users.get(&id))
            .cloned()
            .unwrap_or_default()
    };
    let working: Vec<i64> = supervisors.keys().copied().collect();
    let horizon = as_of
        .checked_add(jiff::Span::new().days(policy.reminder_window_days))
        .map_err(|_| SkillsError::InvalidReminders)?;
    let scores = client.query("SELECT employee_id,skill_id,to_char(next_due_on,'YYYY-MM-DD') AS next_due_on,to_char(valid_until,'YYYY-MM-DD') AS valid_until FROM people_skill_scores WHERE tenant_id=$1 AND company_id=$2 AND employee_id=ANY($3::bigint[]) AND (next_due_on <= $4::text::date OR valid_until <= $5::text::date) ORDER BY id", &[&scope.tenant(),&company,&working,&as_of.to_string(),&horizon.to_string()]).await.map_err(Error::from)?;
    let mut due = Vec::new();
    for row in &scores {
        let employee: i64 = row.get("employee_id");
        if let Some(day) = rows::date(row, "next_due_on")?.filter(|d| *d <= as_of) {
            due.push(item(
                "overdue_reassessment",
                Some(employee),
                row.get("skill_id"),
                None,
                day,
                supervisor_users(employee),
            ));
        }
    }
    for row in &scores {
        let employee: i64 = row.get("employee_id");
        if let Some(day) = rows::date(row, "valid_until")?.filter(|d| *d <= horizon) {
            let mut recipients = supervisor_users(employee);
            recipients.extend(users.get(&employee).into_iter().flatten());
            due.push(item(
                "expiring_certificate",
                Some(employee),
                row.get("skill_id"),
                None,
                day,
                recipients,
            ));
        }
    }
    let actions = client.query("SELECT id,employee_id,skill_id,owner_employee_id,to_char(due_on,'YYYY-MM-DD') AS due_on FROM people_skill_actions WHERE tenant_id=$1 AND company_id=$2 AND status IN ('not_started','scheduled','in_progress','pending_reassessment') AND due_on < $3::text::date ORDER BY id", &[&scope.tenant(),&company,&as_of.to_string()]).await.map_err(Error::from)?;
    for row in actions {
        due.push(item(
            "overdue_action",
            Some(row.get("employee_id")),
            row.get("skill_id"),
            Some(row.get("id")),
            rows::date(&row, "due_on")?.ok_or(Error::Internal)?,
            users
                .get(&row.get::<_, i64>("owner_employee_id"))
                .cloned()
                .unwrap_or_default(),
        ));
    }
    for coverage in standing::coverage(skills, client, scope, company, as_of).await? {
        if !coverage.covered {
            due.push(item(
                "coverage_gap",
                None,
                coverage.skill_id,
                None,
                as_of,
                holders.clone(),
            ));
        }
    }
    Ok(due)
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn claim(
    skills: &Skills,
    scope: &Scope,
    company: i64,
    item: &DueReminder,
    recipient: i64,
    as_of: Date,
    execution: Option<&Execution>,
) -> Result<Option<i64>> {
    let mut client = skills.db.get().await.map_err(Error::from)?;
    let mut w = skills
        .begin_reminders(&mut client, scope, company, "claim_reminder", execution)
        .await?;
    let key = period(&item.rule, as_of);
    let exists = w.sql().query_opt("SELECT id FROM people_skill_reminders WHERE tenant_id=$1 AND company_id=$2 AND rule=$3 AND employee_id IS NOT DISTINCT FROM $4 AND skill_id=$5 AND action_id IS NOT DISTINCT FROM $6 AND period_key=$7 AND recipient_user_id=$8", &[&w.tenant,&company,&item.rule,&item.employee_id,&item.skill_id,&item.action_id,&key,&recipient]).await.map_err(Error::from)?.is_some();
    if exists {
        w.tx.rollback().await?;
        return Ok(None);
    }
    let (tenant, item) = (w.tenant, item.clone());
    let id = w.tx.mutate(move |sql| Box::pin(async move {
        let row = sql.query_one(&format!("INSERT INTO people_skill_reminders(tenant_id,company_id,rule,employee_id,skill_id,action_id,period_key,recipient_user_id,due_on,state) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9::text::date,'pending') RETURNING {COLUMNS}"), &[&tenant,&company,&item.rule,&item.employee_id,&item.skill_id,&item.action_id,&key,&recipient,&item.due_on.to_string()]).await?;
        let reminder = read(&row).map_err(db_error)?;
        Ok((reminder.id,vec![rows::created(KIND,reminder.id,&reminder)]))
    })).await?;
    w.tx.commit().await?;
    Ok(Some(id))
}
fn db_error(error: SkillsError) -> Error {
    match error {
        SkillsError::Failed(e) => e,
        _ => Error::Internal,
    }
}

pub(super) enum Delivery {
    Sent,
    Failed,
    Skipped,
}
async fn locked(w: &Write<'_>, id: i64) -> Result<Reminder> {
    read(&w.sql().query_opt(&format!("SELECT {COLUMNS} FROM people_skill_reminders WHERE tenant_id=$1 AND company_id=$2 AND id=$3 FOR UPDATE"), &[&w.tenant,&w.company_id,&id]).await.map_err(Error::from)?.ok_or(SkillsError::NotFound)?)
}
async fn retryable(w: &Write<'_>, id: i64, as_of: Date) -> Result<bool> {
    Ok(w.sql().query_opt(&format!("SELECT id FROM people_skill_reminders WHERE tenant_id=$1 AND company_id=$2 AND id=$3 AND {RETRY}"), &[&w.tenant,&w.company_id,&id,&periods(as_of)]).await.map_err(Error::from)?.is_some())
}
async fn state(w: &mut Write<'_>, old: Reminder, failure: Option<&str>) -> Result<()> {
    let (tenant, company, id) = (w.tenant, w.company_id, old.id);
    let failure = failure.map(str::to_owned);
    let next = if failure.is_some() { "failed" } else { "sent" };
    w.tx.mutate(move |sql| Box::pin(async move {
        let row = sql.query_one(&format!("UPDATE people_skill_reminders SET state=$4::text,failure=$5,sent_at=CASE WHEN $4::text='sent' THEN now() AT TIME ZONE 'UTC' ELSE NULL END,updated_at=now() AT TIME ZONE 'UTC' WHERE tenant_id=$1 AND company_id=$2 AND id=$3 RETURNING {COLUMNS}"), &[&tenant,&company,&id,&next,&failure]).await?;
        let new = read(&row).map_err(db_error)?;
        Ok(((),vec![rows::changed(KIND,id,&old,&new)]))
    })).await?;
    Ok(())
}
fn notice(row: &Reminder, skill: &str) -> Result<Notice> {
    let date = row.due_on;
    let (title, body, url) = match row.rule.as_str() {
        "overdue_reassessment" => (
            "Skill reassessment overdue",
            format!("{skill} reassessment was due on {date}."),
            "/people/skills/my",
        ),
        "expiring_certificate" => (
            "Skill validity ending",
            format!("{skill} validity ends on {date}."),
            "/people/skills/my",
        ),
        "overdue_action" => (
            "Development action overdue",
            format!("{skill} development action was due on {date}."),
            "/people/skills/actions",
        ),
        "coverage_gap" => (
            "Critical skill under-covered",
            format!("{skill} has fewer qualified holders than the backup minimum as of {date}."),
            "/people/skills/assessments",
        ),
        _ => return Err(SkillsError::InvalidReminders),
    };
    Ok(Notice::new(title)
        .kind("people.skills.reminder")
        .body(body)
        .url(url))
}

pub(super) async fn deliver(
    skills: &Skills,
    scope: &Scope,
    company: i64,
    id: i64,
    retry_as_of: Option<Date>,
    execution: Option<&Execution>,
) -> Result<Delivery> {
    let mut client = skills.db.get().await.map_err(Error::from)?;
    let failure = {
        let mut w = skills
            .begin_reminders(&mut client, scope, company, "send_reminder", execution)
            .await?;
        let row = locked(&w, id).await?;
        let eligible = match retry_as_of {
            Some(day) => retryable(&w, id, day).await?,
            None => true,
        };
        if row.state == "sent" || !eligible {
            w.tx.rollback().await?;
            return Ok(Delivery::Skipped);
        }
        // The ledger names an account, never the operator. Re-prove that it is
        // still affiliated with this company before creating its notification.
        match bilibi_user::company_accounts::get(w.sql(), scope, company, row.recipient_user_id)
            .await
        {
            Ok(_) => {}
            Err(Error::NotFound) => {
                state(&mut w, row, Some("recipient not found")).await?;
                w.tx.commit().await?;
                return Ok(Delivery::Failed);
            }
            Err(e) => return Err(e.into()),
        }
        let skill: Option<String> = w
            .sql()
            .query_opt(
                "SELECT name FROM people_skills WHERE tenant_id=$1 AND company_id=$2 AND id=$3",
                &[&w.tenant, &company, &row.skill_id],
            )
            .await
            .map_err(Error::from)?
            .map(|r| r.get(0));
        let notice = notice(&row, skill.as_deref().unwrap_or("A skill"))?;
        let notified = skills
            .notifications
            .notify(&mut w.tx, scope, &[row.recipient_user_id], &notice)
            .await;
        match notified {
            Ok(()) => {
                state(&mut w, row, None).await?;
                w.tx.commit().await?;
                return Ok(Delivery::Sent);
            }
            Err(error) => {
                // A failed notify may poison its transaction. Roll it back before
                // leaving this scope and opening failure recovery on the client.
                let failure = if matches!(error, Error::NotFound) {
                    "recipient not found"
                } else {
                    "notification could not be created"
                };
                w.tx.rollback().await?;
                failure
            }
        }
    };
    let mut w = skills
        .begin_reminders(&mut client, scope, company, "fail_reminder", execution)
        .await?;
    let row = locked(&w, id).await?;
    if row.state == "sent" {
        w.tx.rollback().await?;
        return Ok(Delivery::Skipped);
    }
    state(&mut w, row, Some(failure)).await?;
    w.tx.commit().await?;
    Ok(Delivery::Failed)
}

pub(super) async fn retry_ids(
    client: &impl GenericClient,
    scope: &Scope,
    company: i64,
    as_of: Date,
) -> Result<Vec<i64>> {
    // Reuse the same eligibility predicate and numbered binds as the locked read.
    Ok(client.query(&format!("SELECT id FROM people_skill_reminders WHERE tenant_id=$1 AND company_id=$2 AND id>$3 AND {RETRY} ORDER BY id"), &[&scope.tenant(),&company,&0i64,&periods(as_of)]).await.map_err(Error::from)?.iter().map(|r|r.get(0)).collect())
}
pub(super) async fn inbox(
    client: &impl GenericClient,
    scope: &Scope,
    company: i64,
    limit: i64,
) -> Result<Vec<Reminder>> {
    client.query(&format!("SELECT {COLUMNS} FROM people_skill_reminders WHERE tenant_id=$1 AND company_id=$2 AND recipient_user_id=$3 AND state='sent' ORDER BY id DESC LIMIT $4"), &[&scope.tenant(),&company,&scope.user(),&limit]).await.map_err(Error::from)?.iter().map(read).collect()
}

// Policy operators see delivery failures as well as sent rows, always scoped.
pub(super) async fn ledger(
    client: &impl GenericClient,
    scope: &Scope,
    company: i64,
    limit: i64,
) -> Result<Vec<Reminder>> {
    client.query(&format!("SELECT {COLUMNS} FROM people_skill_reminders WHERE tenant_id=$1 AND company_id=$2 ORDER BY id DESC LIMIT $3"), &[&scope.tenant(), &company, &limit]).await.map_err(Error::from)?.iter().map(read).collect()
}
