//! Public records mirror Bilimbi People Performance schema-free maps.
//! Dates are ISO text; instants are UTC text with Z, independent of the DB session.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Description {
    pub id: i64,
    pub actor_user_id: i64,
    pub code: String,
    pub version: i32,
    pub position_id: i64,
    pub position_version: i32,
    pub effective_from: String,
    pub effective_to: Option<String>,
    pub purpose: String,
    pub responsibilities: String,
    pub duties: String,
    pub authority: String,
    pub qualifications: String,
    pub competency_links: Value,
    pub status: String,
    pub published_at: Option<String>,
    pub published_by_user_id: Option<i64>,
    pub inserted_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Definition {
    pub id: i64,
    pub actor_user_id: i64,
    pub code: String,
    pub version: i32,
    pub name: String,
    pub purpose: String,
    pub unit: String,
    pub measure: String,
    pub source_reference: String,
    pub calculation_version: String,
    pub direction: String,
    pub rubric: Option<String>,
    pub precision: i32,
    pub interpretation: String,
    pub inserted_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Target {
    pub id: i64,
    pub actor_user_id: i64,
    pub definition_id: i64,
    pub definition_version: i32,
    pub employee_id: i64,
    pub target: String,
    pub period_start: String,
    pub period_end: String,
    pub effective_from: String,
    pub version: i32,
    pub supersedes_id: Option<i64>,
    pub change_reason: Option<String>,
    pub confidential: bool,
    pub status: String,
    pub review_note: Option<String>,
    pub reviewed_by_user_id: Option<i64>,
    pub published_by_user_id: Option<i64>,
    pub published_at: Option<String>,
    pub inserted_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Observation {
    pub id: i64,
    pub actor_user_id: i64,
    pub employee_id: i64,
    pub window_start: String,
    pub window_end: String,
    pub evidence: String,
    pub source_reference: String,
    pub source_version: String,
    pub supersedes_id: Option<i64>,
    pub change_reason: Option<String>,
    pub inserted_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Review {
    pub id: i64,
    pub actor_user_id: i64,
    pub employee_id: i64,
    pub description_id: i64,
    pub period_start: String,
    pub period_end: String,
    pub cutoff_at: String,
    pub outcome: String,
    pub rationale: String,
    pub version: i32,
    pub supersedes_id: Option<i64>,
    pub change_reason: Option<String>,
    pub status: String,
    pub released_at: Option<String>,
    pub released_by_user_id: Option<i64>,
    pub inserted_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReviewObservation {
    pub id: i64,
    pub actor_user_id: i64,
    pub review_id: i64,
    pub observation_id: i64,
    pub inserted_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReviewTarget {
    pub id: i64,
    pub actor_user_id: i64,
    pub review_id: i64,
    pub target_id: i64,
    pub inserted_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Response {
    pub id: i64,
    pub actor_user_id: i64,
    pub review_id: i64,
    pub employee_id: i64,
    pub response: String,
    pub inserted_at: String,
    pub updated_at: String,
}

/// One review version, with the exact pinned history and its responses.
#[derive(Clone, Debug, Serialize)]
pub struct ReviewDetail {
    #[serde(flatten)]
    pub review: Review,
    pub observations: Vec<Observation>,
    pub targets: Vec<Target>,
    pub responses: Vec<Response>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Page<T> {
    pub rows: Vec<T>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}
#[derive(Clone, Debug, Serialize)]
pub struct MyRecords {
    pub reviews: Page<ReviewDetail>,
    pub targets: Vec<Target>,
}
#[derive(Clone, Debug, Serialize)]
pub struct PlanningRecords {
    pub definitions: Vec<Definition>,
    pub descriptions: Vec<Description>,
    pub targets: Vec<Target>,
    pub observations: Vec<Observation>,
    pub drafts: Vec<Review>,
    pub prior_reviews: Vec<Review>,
}
#[derive(Clone, Debug, Serialize)]
pub struct EmployeeChoice {
    pub id: i64,
    pub name: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct PositionChoice {
    pub id: i64,
    pub name: String,
    pub version: Option<i32>,
}
#[derive(Clone, Debug, Serialize)]
pub struct ProfileChoice {
    pub id: i64,
    pub code: String,
    pub name: String,
    pub version: i32,
}
#[derive(Clone, Debug, Serialize)]
pub struct PlanningChoices {
    pub employees: Vec<EmployeeChoice>,
    pub descriptions: Vec<Description>,
    pub targets: Vec<Target>,
    pub positions: Vec<PositionChoice>,
    pub profiles: Vec<ProfileChoice>,
    pub names: std::collections::BTreeMap<i64, String>,
}
