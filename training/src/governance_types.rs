//! Learning governance records; mirrors Bilimbi Training Governance schemas.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The audience determines both the required capability and visible records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LearningAudience {
    SelfService,
    Team,
    Hr,
}
/// The parent whose permanent decision history is requested.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LearningKind {
    Request,
    Plan,
}
pub type LearningHistories = BTreeMap<i64, Vec<LearningDecision>>;

/// Decimal inputs and outputs are exact strings, never floating point.
/// Dates are ISO `YYYY-MM-DD`; amounts have at most four decimal places.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewBudgetPolicy {
    pub currency: String,
    pub effective_from: String,
    pub effective_to: String,
    pub amount: String,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewLearningRequest {
    pub course_id: Option<i64>,
    pub need: String,
    pub objective: String,
    pub expected_result: String,
    pub proposed_on: String,
    pub estimated_cost: String,
    pub currency: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewLearningPlan {
    pub period_start: String,
    pub period_end: String,
    pub objectives: String,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewLearningPlanItem {
    pub request_id: Option<i64>,
    pub need: String,
    pub expected_result: String,
    pub target_cohort: String,
    pub responsible_owner: String,
    pub intended_timing: String,
    pub evaluation_approach: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BudgetPolicy {
    pub id: i64,
    pub company_id: i64,
    pub actor_user_id: i64,
    pub impersonator_id: Option<i64>,
    pub currency: String,
    pub effective_from: String,
    pub effective_to: String,
    pub amount: String,
    pub reason: String,
    pub supersedes_id: Option<i64>,
    pub inserted_at: String,
}
/// Historical policies remain visible with their successor and current commitments.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LearningBudget {
    #[serde(flatten)]
    pub policy: BudgetPolicy,
    pub superseded_by: Option<i64>,
    pub committed: String,
    pub remaining: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LearningRequest {
    pub id: i64,
    pub company_id: i64,
    pub actor_user_id: i64,
    pub impersonator_id: Option<i64>,
    pub employee_id: i64,
    pub course_id: Option<i64>,
    pub need: String,
    pub objective: String,
    pub expected_result: String,
    pub proposed_on: String,
    pub estimated_cost: String,
    pub currency: String,
    pub status: String,
    pub budget_policy_id: Option<i64>,
    pub approved_cost: Option<String>,
    pub inserted_at: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LearningPlan {
    pub id: i64,
    pub company_id: i64,
    pub actor_user_id: i64,
    pub impersonator_id: Option<i64>,
    pub plan_key: String,
    pub version: i32,
    pub manager_employee_id: i64,
    pub period_start: String,
    pub period_end: String,
    pub objectives: String,
    pub status: String,
    pub prior_plan_id: Option<i64>,
    pub reason: String,
    pub inserted_at: String,
    #[serde(default)]
    pub items: Vec<LearningPlanItem>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LearningPlanItem {
    pub id: i64,
    pub company_id: i64,
    pub actor_user_id: i64,
    pub impersonator_id: Option<i64>,
    pub plan_id: i64,
    pub request_id: Option<i64>,
    pub need: String,
    pub expected_result: String,
    pub target_cohort: String,
    pub responsible_owner: String,
    pub intended_timing: String,
    pub evaluation_approach: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LearningDecision {
    pub id: i64,
    pub company_id: i64,
    pub actor_user_id: i64,
    pub impersonator_id: Option<i64>,
    pub request_id: Option<i64>,
    pub plan_id: Option<i64>,
    pub action: String,
    pub from_status: String,
    pub to_status: String,
    pub reason: String,
    pub inserted_at: String,
}
