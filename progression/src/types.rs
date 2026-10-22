//! Bilimbi Progression's policy views and explanatory results.
use serde::{Deserialize, Serialize};

/// An exact Skills profile version to copy, rather than caller-supplied thresholds.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfilePin {
    pub id: i64,
    pub version: i32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftInput {
    pub code: String,
    pub version: i32,
    pub name: String,
    /// ISO date, YYYY-MM-DD.
    pub effective_from: String,
    pub profile: Option<ProfilePin>,
    pub performance: Option<PerformanceRule>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompetenceRule {
    pub code: String,
    pub profile_id: i64,
    pub profile_version: i32,
    pub skill_id: i64,
    pub required_level: i32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Period {
    pub start: String,
    pub end: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingEvidence {
    Unknown,
    NotMet,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerformanceRule {
    pub periods: Vec<Period>,
    pub accepted_outcomes: Vec<String>,
    pub missing_evidence: MissingEvidence,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    pub competence: Vec<CompetenceRule>,
    pub performance: Option<PerformanceRule>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Policy {
    pub id: i64,
    pub code: String,
    pub version: i32,
    pub name: String,
    pub effective_from: String,
    pub rules: Rules,
    pub status: String,
    pub actor_user_id: i64,
    pub published_by_user_id: Option<i64>,
    pub published_at: Option<String>,
}
#[derive(Clone, Copy, Debug)]
pub struct PageOptions {
    pub page: i64,
    pub page_size: i64,
}
impl Default for PageOptions {
    fn default() -> Self {
        Self {
            page: 1,
            page_size: 25,
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct PolicyPage {
    pub rows: Vec<Policy>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Met,
    NotMet,
    Unknown,
}
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum Criterion {
    Skill {
        code: String,
        status: Status,
        required_level: i32,
        observed_level: Option<i32>,
        assessment_id: Option<i64>,
    },
    Performance {
        code: String,
        status: Status,
        review_id: Option<i64>,
        review_version: Option<i32>,
        supersedes_id: Option<i64>,
        period_start: Option<String>,
        period_end: Option<String>,
    },
}
impl Criterion {
    pub fn status(&self) -> Status {
        match self {
            Self::Skill { status, .. } | Self::Performance { status, .. } => *status,
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Explanation {
    pub policy: Policy,
    pub rules: Vec<Criterion>,
    pub status: Status,
}
#[derive(Clone, Debug, Serialize)]
pub struct Standing {
    pub employee_id: i64,
    pub explanations: Vec<Explanation>,
}
