//! Bilimbi `Leave.CarryForwardWorker`, as the enqueuing operator.
use crate::{Leave, LeaveError};
use bilibi_database::{Error, Result};
use bilibi_queue::execution::{Execution, Handler, HandlerFuture, Outcome};
use serde_json::{json, Value};
use std::sync::Arc;

pub(super) const WORKER: &str = "people-leave/carry-forward";
pub(super) const SERVICE: &str = "people/leave.carry_forward_worker";
pub(super) struct CarryForwardWorker(pub Arc<Leave>);
pub(super) fn validate(args: Value) -> Result<Value> {
    let object = args.as_object().ok_or(Error::Invalid)?;
    if object.len() != 2 {
        return Err(Error::Invalid);
    }
    let company = object
        .get("company_id")
        .and_then(Value::as_i64)
        .filter(|c| *c > 0)
        .ok_or(Error::Invalid)?;
    let year = object
        .get("from_year")
        .and_then(Value::as_i64)
        .filter(|y| (1900..=9997).contains(y))
        .ok_or(Error::Invalid)?;
    Ok(json!({"company_id":company,"from_year":year}))
}
impl Handler for CarryForwardWorker {
    fn perform<'a>(&'a self, execution: &'a Execution, args: Value) -> HandlerFuture<'a> {
        Box::pin(async move {
            let Some(scope) = execution.scope() else {
                return Outcome::Cancel("not_authorized");
            };
            let Ok(args) = validate(args) else {
                return Outcome::Cancel("invalid_carry_forward");
            };
            let Some(company) = args["company_id"].as_i64() else {
                return Outcome::Cancel("invalid_carry_forward");
            };
            let Some(year) = args["from_year"]
                .as_i64()
                .and_then(|y| i32::try_from(y).ok())
            else {
                return Outcome::Cancel("invalid_carry_forward");
            };
            match self
                .0
                .carry_forward_run(scope, company, year, Some(execution))
                .await
            {
                Ok(_) => Outcome::Completed,
                Err(LeaveError::NotCurrent(_)) => Outcome::Retry("workforce_not_current"),
                Err(
                    LeaveError::Unauthorized | LeaveError::CompanyArchived | LeaveError::NotFound,
                ) => Outcome::Cancel("not_authorized"),
                Err(LeaveError::YearNotEnded) => Outcome::Cancel("year_not_ended"),
                Err(LeaveError::Failed(_)) => Outcome::Retry("carry_forward_failed"),
                Err(_) => Outcome::Cancel("carry_forward_refused"),
            }
        })
    }
}
