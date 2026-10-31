//! Bilimbi Skills.Reminders facade methods. Every write rechecks the operator.
use crate::{
    assessments, reminders, DueReminder, Reminder, ReminderCounts, ReminderRetryCounts, Result,
    Skills, SkillsError, Write, REMINDERS_SEND,
};
use bilibi_database::Error;
use bilibi_people_workforce::authorization;
use bilibi_queue::Execution;
use bilibi_tenancy::Scope;
use jiff::civil::Date;
use serde_json::json;
use tokio_postgres::Client;

impl Skills {
    /// Computes all four rules without writing. None uses today's UTC date.
    pub async fn due_reminders(
        &self,
        scope: &Scope,
        company: i64,
        as_of: Option<Date>,
    ) -> Result<Vec<DueReminder>> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company, REMINDERS_SEND).await?;
        self.current_company(&**client, scope, company).await?;
        reminders::compute(
            self,
            &**client,
            scope,
            company,
            as_of.unwrap_or_else(assessments::today),
        )
        .await
    }

    /// Claims one ledger row per item, recipient and period, then sends it.
    pub async fn issue_reminders(
        &self,
        scope: &Scope,
        company: i64,
        as_of: Option<Date>,
    ) -> Result<ReminderCounts> {
        self.issue_reminders_run(scope, company, as_of, None).await
    }
    pub(super) async fn issue_reminders_run(
        &self,
        scope: &Scope,
        company: i64,
        as_of: Option<Date>,
        execution: Option<&Execution>,
    ) -> Result<ReminderCounts> {
        let as_of = as_of.unwrap_or_else(assessments::today);
        let due = self.due_reminders(scope, company, Some(as_of)).await?;
        let mut counts = ReminderCounts::default();
        for item in due {
            if item.recipients.is_empty() {
                counts.unaddressed += 1;
            }
            for &recipient in &item.recipients {
                let Some(id) =
                    reminders::claim(self, scope, company, &item, recipient, as_of, execution)
                        .await?
                else {
                    counts.skipped += 1;
                    continue;
                };
                match reminders::deliver(self, scope, company, id, None, execution).await? {
                    reminders::Delivery::Sent => counts.sent += 1,
                    reminders::Delivery::Failed => counts.failed += 1,
                    reminders::Delivery::Skipped => counts.skipped += 1,
                }
            }
        }
        Ok(counts)
    }

    /// Retries failed and pending-over-15-minute rows in this week/month.
    /// Eligibility is checked again under the company and ledger row locks.
    pub async fn retry_reminders(
        &self,
        scope: &Scope,
        company: i64,
        as_of: Option<Date>,
    ) -> Result<ReminderRetryCounts> {
        let as_of = as_of.unwrap_or_else(assessments::today);
        let client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company, REMINDERS_SEND).await?;
        self.current_company(&**client, scope, company).await?;
        let ids = reminders::retry_ids(&**client, scope, company, as_of).await?;
        drop(client);
        let mut counts = ReminderRetryCounts::default();
        for id in ids {
            match reminders::deliver(self, scope, company, id, Some(as_of), None).await? {
                reminders::Delivery::Sent => counts.sent += 1,
                reminders::Delivery::Failed => counts.failed += 1,
                reminders::Delivery::Skipped => {}
            }
        }
        Ok(counts)
    }

    /// Company delivery ledger for the policy operator, newest first (1–300).
    pub async fn reminder_ledger(
        &self,
        scope: &Scope,
        company: i64,
        limit: i64,
    ) -> Result<Vec<Reminder>> {
        if !(1..=300).contains(&limit) {
            return Err(SkillsError::InvalidReminders);
        }
        let client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company, REMINDERS_SEND).await?;
        self.current_company(&**client, scope, company).await?;
        reminders::ledger(&**client, scope, company, limit).await
    }

    /// Sent rows addressed to this signed-in account, newest first (1–300).
    pub async fn reminder_inbox(
        &self,
        scope: &Scope,
        company: i64,
        limit: i64,
    ) -> Result<Vec<Reminder>> {
        if !(1..=300).contains(&limit) {
            return Err(SkillsError::InvalidReminders);
        }
        let client = self.db.get().await.map_err(Error::from)?;
        self.current_company(&**client, scope, company).await?;
        bilibi_user::UserService::company_account_on(&**client, scope, company, scope.user())
            .await
            .map_err(|e| {
                if matches!(e, Error::NotFound) {
                    SkillsError::Unauthorized
                } else {
                    e.into()
                }
            })?;
        reminders::inbox(&**client, scope, company, limit).await
    }

    /// Queues due reminders as this operator. No schedule or system actor.
    pub async fn enqueue_reminders(
        &self,
        scope: &Scope,
        company: i64,
    ) -> Result<bilibi_queue::JobRef> {
        if company <= 0 {
            return Err(SkillsError::InvalidReminders);
        }
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut w = self
            .begin_reminders(&mut client, scope, company, "enqueue_reminders", None)
            .await?;
        let job = self
            .queue
            .enqueue_for(
                &mut w.tx,
                scope,
                crate::reminder_worker::WORKER,
                json!({"company_id":company}),
            )
            .await?;
        w.tx.commit().await?;
        Ok(job)
    }

    pub(super) async fn begin_reminders<'a>(
        &self,
        client: &'a mut Client,
        scope: &Scope,
        company: i64,
        action: &str,
        execution: Option<&Execution>,
    ) -> Result<Write<'a>> {
        match execution {
            None => {
                self.begin(client, scope, company, REMINDERS_SEND, action)
                    .await
            }
            Some(execution) => {
                let tx = execution
                    .begin(client, &format!("people.skills.{action}"))
                    .await?;
                bilibi_company::require_writable(tx.sql(), scope, company, true).await?;
                authorization::authorize(tx.sql(), scope, company, REMINDERS_SEND).await?;
                self.current_company(tx.sql(), scope, company).await?;
                Ok(Write {
                    tx,
                    tenant: scope.tenant(),
                    company_id: company,
                    actor: scope.user(),
                })
            }
        }
    }
}
