//! Bilimbi Training evaluation records. Employment IDs identify every learner.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EvaluationKind {
    Evaluation,
    Effectiveness,
}
impl EvaluationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Evaluation => "evaluation",
            Self::Effectiveness => "effectiveness",
        }
    }
}
/// No policy timing or disclosure minimum is silently chosen for the company.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationSettings {
    pub evaluation_days: Option<i32>,
    pub checkpoints: Option<Vec<i32>>,
    pub reminder_days: Option<i32>,
    pub minimum_cohort: Option<i32>,
    pub report_months: Option<i32>,
    pub report_grace_days: Option<i32>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationCriterion {
    pub code: String,
    pub label: String,
    pub minimum: i32,
    pub maximum: i32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvaluationCriteria {
    pub items: Vec<EvaluationCriterion>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvaluationCheckpoints {
    pub days: Vec<i32>,
}
#[derive(Clone, Debug)]
pub struct NewEvaluationPolicy {
    pub effective_from: String,
    pub effective_to: String,
    pub criteria: Vec<EvaluationCriterion>,
    pub effectiveness_criteria: Vec<EvaluationCriterion>,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvaluationPolicy {
    pub id: i64,
    pub company_id: i64,
    pub actor_user_id: i64,
    pub impersonator_id: Option<i64>,
    pub version: i32,
    pub effective_from: String,
    pub effective_to: String,
    pub criteria: EvaluationCriteria,
    pub effectiveness_criteria: EvaluationCriteria,
    pub evaluation_days: i32,
    pub checkpoints: EvaluationCheckpoints,
    pub reminder_days: i32,
    pub reason: String,
    pub inserted_at: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvaluationReview {
    pub id: i64,
    pub company_id: i64,
    pub actor_user_id: i64,
    pub impersonator_id: Option<i64>,
    pub policy_id: i64,
    pub fact_id: i64,
    pub kind: EvaluationKind,
    pub checkpoint_days: i32,
    pub due_on: String,
    pub inserted_at: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvaluationAnswer {
    pub id: i64,
    pub company_id: i64,
    pub actor_user_id: i64,
    pub impersonator_id: Option<i64>,
    pub review_id: i64,
    /// Exactly the policy's codes; JSON null is an explicit unknown answer.
    pub values: Value,
    pub reason: String,
    pub inserted_at: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvaluationReminder {
    pub available_on: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvaluationTask {
    #[serde(flatten)]
    pub review: EvaluationReview,
    pub employee_id: i64,
    pub session_id: i64,
    pub criteria: Vec<EvaluationCriterion>,
    pub policy_version: i32,
    pub answer: Option<EvaluationAnswer>,
    pub reminder: Option<EvaluationReminder>,
}
#[derive(Clone, Debug, Serialize)]
pub struct PreparedEvaluationReviews {
    pub reviews: Vec<EvaluationReview>,
    /// Latest confirmed learners no longer in the current workforce.
    pub unknown: usize,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct EvaluationReminders {
    pub created: usize,
    pub unknown: usize,
}
#[derive(Clone, Debug, Serialize)]
pub struct FrozenEffectivenessSummaries {
    pub frozen: usize,
}
/// Frozen output contains no employee IDs, cohort totals or complementary counts.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EffectivenessCriterion {
    pub label: String,
    pub status: String,
    pub answered: Option<usize>,
    /// Exact, half-up rounded decimal with two fractional digits.
    pub mean: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EffectivenessGroup {
    pub policy_version: i32,
    pub checkpoint_days: i32,
    pub status: String,
    pub criteria: Vec<EffectivenessCriterion>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EffectivenessGroups {
    pub items: Vec<EffectivenessGroup>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EffectivenessSummary {
    pub id: i64,
    pub company_id: i64,
    pub actor_user_id: i64,
    pub impersonator_id: Option<i64>,
    pub period_start: String,
    pub period_end: String,
    pub minimum_cohort: i32,
    pub status: String,
    pub groups: EffectivenessGroups,
    pub inserted_at: String,
}
