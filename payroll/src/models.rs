//! Bilimbi Payroll record and input shapes; persistence stays private.
use jiff::civil::Date;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// An immutable classification version. All stored dates are inclusive ISO dates.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Classification {
    pub id: i64,
    pub created_by_actor_id: i64,
    pub inserted_at: String,
    pub updated_at: String,
    pub code: String,
    pub name: String,
    pub effective_from: String,
    pub effective_to: Option<String>,
}
/// A pay-item version. `amount` is an exact six-place decimal string.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    pub id: i64,
    pub created_by_actor_id: i64,
    pub inserted_at: String,
    pub updated_at: String,
    pub code: String,
    pub name: String,
    pub classification_id: i64,
    pub currency: String,
    pub amount: String,
    pub effective_from: String,
    pub effective_to: Option<String>,
}
/// A nonoverlapping pay window with its explicit payment date.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Period {
    pub id: i64,
    pub created_by_actor_id: i64,
    pub inserted_at: String,
    pub updated_at: String,
    pub code: String,
    pub starts_on: String,
    pub ends_on: String,
    pub pay_on: String,
}
/// A Leave or Claims source mapped to one item for an inclusive interval.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Mapping {
    pub id: i64,
    pub created_by_actor_id: i64,
    pub inserted_at: String,
    pub updated_at: String,
    pub source_kind: String,
    pub source_key: String,
    pub item_id: i64,
    pub effective_from: String,
    pub effective_to: Option<String>,
}
/// An Attendance rule code mapped to an item; later rule versions keep the code.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AttendanceAllowanceMapping {
    pub id: i64,
    pub created_by_actor_id: i64,
    pub inserted_at: String,
    pub updated_at: String,
    pub attendance_rule_code: String,
    pub item_id: i64,
    pub effective_from: String,
    pub effective_to: Option<String>,
}
/// A frozen period/currency setup. Only its initial lock can change.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Run {
    pub id: i64,
    pub created_by_actor_id: i64,
    pub inserted_at: String,
    pub updated_at: String,
    pub period_id: i64,
    pub country: String,
    pub currency: String,
    pub snapshot: Value,
    pub locked_at: Option<String>,
    pub locked_by_actor_id: Option<i64>,
}
#[derive(Clone, Debug)]
pub struct NewClassification {
    pub code: String,
    pub name: String,
    pub effective_from: Date,
    pub effective_to: Option<Date>,
}
#[derive(Clone, Debug)]
pub struct NewItem {
    pub code: String,
    pub name: String,
    pub classification_id: i64,
    pub currency: String,
    pub amount: Decimal,
    pub effective_from: Date,
    pub effective_to: Option<Date>,
}
#[derive(Clone, Debug)]
pub struct NewPeriod {
    pub code: String,
    pub starts_on: Date,
    pub ends_on: Date,
    pub pay_on: Date,
}
#[derive(Clone, Debug)]
pub struct NewMapping {
    pub source_kind: String,
    pub source_key: String,
    pub item_id: i64,
    pub effective_from: Date,
    pub effective_to: Option<Date>,
}
#[derive(Clone, Debug)]
pub struct NewAttendanceAllowanceMapping {
    pub attendance_rule_code: String,
    pub item_id: i64,
    pub effective_from: Date,
    pub effective_to: Option<Date>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Setup {
    pub country: String,
    pub currencies: Vec<String>,
    pub classifications: Vec<Classification>,
    pub items: Vec<Item>,
    pub periods: Vec<Period>,
    pub mappings: Vec<Mapping>,
    pub runs: Vec<Run>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Source {
    pub key: String,
    pub name: String,
    pub active: bool,
}
pub type Sources = BTreeMap<String, Vec<Source>>;
#[derive(Clone, Debug)]
pub struct AttendanceAllowances {
    pub sources: Vec<bilibi_people_attendance::AllowanceRule>,
    pub items: Vec<Item>,
    pub mappings: Vec<AttendanceAllowanceMapping>,
}

/// Exact operator-attested units; construct Decimal from text, never floats.
#[derive(Clone, Debug)]
pub struct NewContribution {
    pub employee_id: i64,
    pub item_id: i64,
    pub source_key: String,
    pub evidence: String,
    pub on_date: Date,
    pub units: Decimal,
    pub direction: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Contribution {
    pub id: i64,
    pub created_by_actor_id: i64,
    pub inserted_at: String,
    pub updated_at: String,
    pub run_id: i64,
    pub employee_id: i64,
    pub item_id: i64,
    pub source_key: String,
    pub evidence: String,
    pub on_date: String,
    pub units: String,
    pub direction: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Calculation {
    pub id: i64,
    pub created_by_actor_id: i64,
    pub inserted_at: String,
    pub updated_at: String,
    pub run_id: i64,
    pub snapshot: Value,
    pub digest: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Decision {
    pub id: i64,
    pub created_by_actor_id: i64,
    pub inserted_at: String,
    pub updated_at: String,
    pub run_id: i64,
    pub outcome: String,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResultLine {
    pub id: i64,
    pub created_by_actor_id: i64,
    pub inserted_at: String,
    pub updated_at: String,
    pub run_id: i64,
    pub contribution_id: i64,
    pub employee_id: i64,
    pub direction: String,
    pub amount: String,
}
/// Immutable provenance; Base Artifacts owns byte storage and retention.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Document {
    pub id: i64,
    pub created_by_actor_id: i64,
    pub inserted_at: String,
    pub updated_at: String,
    pub run_id: i64,
    pub employee_id: Option<i64>,
    pub artifact_id: String,
    pub kind: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct RunOutput {
    pub run: Run,
    pub contributions: Vec<Contribution>,
    pub calculation: Option<Calculation>,
    pub decision: Option<Decision>,
    pub documents: Vec<Document>,
    pub entries: Vec<ResultLine>,
}
