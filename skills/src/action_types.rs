//! Public development and reassessment models, mirroring Bilimbi Skills.Actions
//! and Skills.Reassessments. Only the facade exports these types.
use crate::Assessment;
use jiff::civil::{Date, DateTime};
use serde::Serialize;

#[derive(Clone, Debug, Default)]
pub struct NewActionType {
    pub code: String,
    pub name: String,
    pub requires_provider: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ActionType {
    pub id: i64,
    pub code: String,
    pub name: String,
    pub requires_provider: bool,
    pub active: bool,
}

/// The source of an action's immutable gap snapshot. Assessment sources must
/// be current finalized gaps or expired critical certificates. Manual sources
/// record their reason and use levels 0–20.
#[derive(Clone, Debug)]
pub enum ActionSource {
    Assessment(i64),
    Manual {
        employee_id: i64,
        skill_id: i64,
        starting_level: i32,
        target_level: i32,
        criticality: String,
        mandatory: bool,
        reason: String,
    },
}

/// The part of a proposal a manager may tailor before approval.
#[derive(Clone, Debug, Default)]
pub struct ActionPlan {
    pub action_type_id: i64,
    pub owner_employee_id: i64,
    pub coordinator_employee_id: i64,
    pub provider_employee_id: Option<i64>,
    pub provider_name: Option<String>,
    pub objective: String,
    pub intervention: String,
    pub expected_evidence: String,
    pub next_steps: Option<String>,
    /// None means today in UTC.
    pub start_on: Option<Date>,
    pub due_on: Option<Date>,
}

#[derive(Clone, Debug)]
pub struct ActionInput {
    pub source: ActionSource,
    pub plan: ActionPlan,
    pub request_key: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ActionGroup {
    #[default]
    Open,
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Action {
    pub id: i64,
    pub employee_id: i64,
    pub employee_name: String,
    pub skill_id: i64,
    pub source_assessment_id: Option<i64>,
    pub action_type_id: i64,
    pub starting_level: i32,
    pub target_level: i32,
    pub gap_at_start: i32,
    pub criticality: String,
    pub mandatory: bool,
    pub priority_multiplier: i32,
    pub priority_score: i32,
    pub priority_explanation: String,
    pub manual_reason: Option<String>,
    pub objective: String,
    pub intervention: String,
    pub expected_evidence: String,
    pub owner_employee_id: i64,
    pub coordinator_employee_id: i64,
    pub provider_employee_id: Option<i64>,
    pub provider_name: Option<String>,
    #[serde(serialize_with = "crate::as_optional_text")]
    pub start_on: Option<Date>,
    #[serde(serialize_with = "crate::as_optional_text")]
    pub due_on: Option<Date>,
    pub status: String,
    pub closure: String,
    pub approved_by_user_id: Option<i64>,
    #[serde(serialize_with = "crate::as_optional_text")]
    pub approved_at: Option<DateTime>,
    #[serde(serialize_with = "crate::as_optional_text")]
    pub completed_at: Option<DateTime>,
    pub completion_evidence: Option<String>,
    #[serde(serialize_with = "crate::as_optional_text")]
    pub reassessment_due_on: Option<Date>,
    pub post_assessment_id: Option<i64>,
    pub post_level: Option<i32>,
    pub improvement: Option<i32>,
    pub next_steps: Option<String>,
    pub created_by_user_id: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReassessmentRequest {
    pub id: i64,
    pub employee_id: i64,
    pub skill_id: i64,
    pub reason: String,
    pub requested_by_user_id: i64,
    #[serde(serialize_with = "crate::as_optional_text")]
    pub due_on: Option<Date>,
    pub status: String,
    pub performed_by_user_id: Option<i64>,
    #[serde(serialize_with = "crate::as_optional_text")]
    pub performed_at: Option<DateTime>,
    pub assessment_id: Option<i64>,
    #[serde(serialize_with = "crate::as_optional_text")]
    pub cancelled_at: Option<DateTime>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ActionEvent {
    pub id: i64,
    pub action_id: i64,
    pub event_type: String,
    pub from_status: Option<String>,
    pub to_status: Option<String>,
    pub comment: Option<String>,
    pub evidence: Option<String>,
    pub actor_user_id: i64,
    #[serde(serialize_with = "crate::as_optional_text")]
    pub inserted_at: Option<DateTime>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ActionView {
    #[serde(flatten)]
    pub action: Action,
    pub skill_name: Option<String>,
    pub type_name: Option<String>,
    pub owner_name: String,
    pub coordinator_name: String,
    pub reassessments: Vec<ActionReassessment>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ActionReassessment {
    pub id: i64,
    #[serde(serialize_with = "crate::as_optional_text")]
    pub assessed_on: Option<Date>,
    pub assessed_level: i32,
}

#[derive(Clone, Debug, Serialize)]
pub struct ReassessmentView {
    #[serde(flatten)]
    pub request: ReassessmentRequest,
    pub employee_name: String,
    pub skill_name: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PerformedReassessment {
    pub request: ReassessmentRequest,
    pub assessment: Assessment,
}
