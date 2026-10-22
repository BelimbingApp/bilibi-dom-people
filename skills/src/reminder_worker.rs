//! Bilimbi Skills.ReminderWorker, running as the enqueuing operator.
use crate::{Skills, SkillsError};
use bilibi_database::{Error, Result};
use bilibi_queue::execution::{Execution, Handler, HandlerFuture, Outcome};
use serde_json::{json, Value};
use std::sync::Arc;

pub(super) const WORKER: &str = "people-skills/reminders";
pub(super) const SERVICE: &str = "people/skills.reminder_worker";
pub(super) struct ReminderWorker(pub Arc<Skills>);

pub(super) fn validate(args: Value) -> Result<Value> {
    let object = args.as_object().ok_or(Error::Invalid)?;
    if object.len() != 1 {
        return Err(Error::Invalid);
    }
    let company = object
        .get("company_id")
        .and_then(Value::as_i64)
        .filter(|id| *id > 0)
        .ok_or(Error::Invalid)?;
    Ok(json!({"company_id": company}))
}
impl Handler for ReminderWorker {
    fn perform<'a>(&'a self, execution: &'a Execution, args: Value) -> HandlerFuture<'a> {
        Box::pin(async move {
            let Some(scope) = execution.scope() else {
                return Outcome::Cancel("not_authorized");
            };
            let Ok(args) = validate(args) else {
                return Outcome::Cancel("invalid_reminders");
            };
            let Some(company) = args["company_id"].as_i64() else {
                return Outcome::Cancel("invalid_reminders");
            };
            match self
                .0
                .issue_reminders_run(scope, company, None, Some(execution))
                .await
            {
                Ok(_) => Outcome::Completed,
                Err(SkillsError::NotCurrent(_)) => Outcome::Retry("workforce_not_current"),
                Err(SkillsError::Failed(_)) => Outcome::Retry("reminders_failed"),
                Err(_) => Outcome::Cancel("not_authorized"),
            }
        })
    }
}
