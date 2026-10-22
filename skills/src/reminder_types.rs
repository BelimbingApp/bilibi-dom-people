//! Bilimbi Skills.Reminder public projections; SQL remains private.
use jiff::civil::{Date, DateTime};
use serde::Serialize;

/// One due item. An empty recipient list is counted as unaddressed, never sent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DueReminder {
    pub rule: String,
    pub employee_id: Option<i64>,
    pub skill_id: i64,
    pub action_id: Option<i64>,
    #[serde(serialize_with = "date_text")]
    pub due_on: Date,
    pub recipients: Vec<i64>,
}
fn date_text<S: serde::Serializer>(date: &Date, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.collect_str(date)
}

/// This run's results. Skipped counts existing recipient ledger rows;
/// unaddressed counts due items that have no account to notify.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ReminderCounts {
    pub sent: usize,
    pub failed: usize,
    pub skipped: usize,
    pub unaddressed: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ReminderRetryCounts {
    pub sent: usize,
    pub failed: usize,
}

/// The durable delivery ledger. A sent row cannot be changed or deleted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Reminder {
    pub id: i64,
    pub rule: String,
    pub employee_id: Option<i64>,
    pub skill_id: i64,
    pub action_id: Option<i64>,
    pub period_key: String,
    pub recipient_user_id: i64,
    #[serde(serialize_with = "date_text")]
    pub due_on: Date,
    pub state: String,
    pub failure: Option<String>,
    #[serde(serialize_with = "crate::as_optional_text")]
    pub sent_at: Option<DateTime>,
}
