//! Explicit input types mirror Bilimbi Performance's allowlisted attributes.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProfileLink {
    pub id: i64,
    pub version: i32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DescriptionInput {
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
    pub competency_links: Vec<ProfileLink>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DefinitionInput {
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
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TargetInput {
    pub definition_id: i64,
    pub employee_id: i64,
    pub target: String,
    pub period_start: String,
    pub period_end: String,
    pub effective_from: String,
    pub confidential: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TargetAmendment {
    pub target: String,
    pub effective_from: String,
    pub change_reason: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ObservationInput {
    pub employee_id: i64,
    pub window_start: String,
    pub window_end: String,
    pub evidence: String,
    pub source_reference: String,
    pub source_version: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ObservationCorrection {
    pub evidence: String,
    pub source_version: String,
    pub change_reason: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReviewInput {
    pub employee_id: i64,
    pub description_id: i64,
    pub period_start: String,
    pub period_end: String,
    /// Explicit ISO UTC instant, with a Z or +00:00 suffix.
    pub cutoff_at: String,
    pub outcome: String,
    pub rationale: String,
    pub observation_ids: Vec<i64>,
    pub target_ids: Vec<i64>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReviewCorrection {
    pub description_id: i64,
    /// None retains the preceding review's cutoff.
    pub cutoff_at: Option<String>,
    pub outcome: String,
    pub rationale: String,
    pub change_reason: String,
    pub observation_ids: Vec<i64>,
    pub target_ids: Vec<i64>,
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
