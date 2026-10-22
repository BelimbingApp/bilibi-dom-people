//! Public assessment inputs and read models, re-exported only by the facade.
//! Mirrors Bilimbi Skills.Assessments and Skills.Standing.
use jiff::civil::{Date, DateTime};
use rust_decimal::Decimal;
use serde::Serialize;

/// An evidence-backed submission. The scope supplies the assessor; the
/// requirement, scale, gap and policy values are resolved by the owner.
#[derive(Clone, Debug, Default)]
pub struct AssessmentInput {
    pub employee_id: i64,
    pub skill_id: i64,
    pub assessed_level: i32,
    pub evidence: String,
    /// None means today in UTC.
    pub assessed_on: Option<Date>,
    pub valid_until: Option<Date>,
    pub method: Option<String>,
    pub notes: Option<String>,
    pub request_key: String,
    pub supersedes_assessment_id: Option<i64>,
}

/// An immutable assessment snapshot and its decision state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Assessment {
    pub id: i64,
    pub employee_id: i64,
    pub skill_id: i64,
    pub profile_id: i64,
    pub scale_id: i64,
    pub required_level: i32,
    pub criticality: String,
    pub weight_percent: Decimal,
    pub mandatory: bool,
    pub assessed_level: i32,
    pub gap: i32,
    pub priority_multiplier: i32,
    pub priority_score: i32,
    pub result_band: String,
    pub method: Option<String>,
    pub evidence: String,
    pub notes: Option<String>,
    #[serde(serialize_with = "crate::as_optional_text")]
    pub assessed_on: Option<Date>,
    #[serde(serialize_with = "crate::as_optional_text")]
    pub valid_until: Option<Date>,
    #[serde(serialize_with = "crate::as_optional_text")]
    pub next_due_on: Option<Date>,
    pub status: String,
    pub assessor_user_id: i64,
    pub reviewed_by_user_id: Option<i64>,
    #[serde(serialize_with = "crate::as_optional_text")]
    pub reviewed_at: Option<DateTime>,
    pub review_note: Option<String>,
    pub finalized_by_user_id: Option<i64>,
    #[serde(serialize_with = "crate::as_optional_text")]
    pub finalized_at: Option<DateTime>,
    pub supersedes_assessment_id: Option<i64>,
}

/// Assessment with the names the register and review queue display.
#[derive(Clone, Debug, Serialize)]
pub struct AssessmentView {
    #[serde(flatten)]
    pub assessment: Assessment,
    pub employee_name: String,
    pub skill_code: Option<String>,
    pub skill_name: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewDecision {
    Verify,
    Return,
}

/// Optional register filters; results are bounded to the newest 200.
#[derive(Clone, Debug, Default)]
pub struct AssessmentFilters {
    pub status: Option<String>,
    pub employee_id: Option<i64>,
    pub skill_id: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AssessmentDecision {
    pub id: i64,
    pub assessment_id: i64,
    pub decision: String,
    pub actor_user_id: i64,
    pub note: Option<String>,
    #[serde(serialize_with = "crate::as_optional_text")]
    pub inserted_at: Option<DateTime>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AssessmentQueue {
    pub review: Vec<AssessmentView>,
    pub finalize: Vec<AssessmentView>,
    pub returned: Vec<AssessmentView>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AssessableEmployee {
    pub id: i64,
    pub name: String,
}

/// The finalized assessment the current-score projection mirrors, with its
/// original requirement version and scale for consumers such as Progression.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Score {
    pub id: i64,
    pub employee_id: i64,
    pub skill_id: i64,
    pub assessment_id: i64,
    pub profile_id: i64,
    pub required_level: i32,
    pub current_level: i32,
    pub gap: i32,
    pub criticality: String,
    pub mandatory: bool,
    pub priority_score: i32,
    #[serde(serialize_with = "crate::as_optional_text")]
    pub assessed_on: Option<Date>,
    #[serde(serialize_with = "crate::as_optional_text")]
    pub valid_until: Option<Date>,
    #[serde(serialize_with = "crate::as_optional_text")]
    pub next_due_on: Option<Date>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScoreView {
    #[serde(flatten)]
    pub score: Score,
    pub assessment_profile_id: i64,
    pub assessment_profile_version: i32,
    pub assessment_scale_id: i64,
    pub employee_name: String,
    pub skill_code: Option<String>,
    pub skill_name: Option<String>,
    pub result_band: String,
    /// current, overdue (inclusive due day), or expired (after validity day).
    pub state: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Standing {
    pub employee_id: i64,
    pub scores: Vec<ScoreView>,
    pub actions: Vec<StandingAction>,
    pub reassessments: Vec<StandingReassessment>,
}

#[derive(Clone, Debug, Serialize)]
pub struct StandingAction {
    pub id: i64,
    pub skill_id: i64,
    pub skill_name: Option<String>,
    pub status: String,
    pub closure: String,
    pub starting_level: i32,
    pub target_level: i32,
    pub objective: String,
    #[serde(serialize_with = "crate::as_optional_text")]
    pub start_on: Option<Date>,
    #[serde(serialize_with = "crate::as_optional_text")]
    pub due_on: Option<Date>,
    pub post_level: Option<i32>,
    pub improvement: Option<i32>,
}

#[derive(Clone, Debug, Serialize)]
pub struct StandingReassessment {
    pub id: i64,
    pub skill_id: i64,
    pub skill_name: Option<String>,
    #[serde(serialize_with = "crate::as_optional_text")]
    pub due_on: Option<Date>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Coverage {
    pub skill_id: i64,
    pub code: String,
    pub name: String,
    pub holders: i64,
    pub minimum: i64,
    pub covered: bool,
}
