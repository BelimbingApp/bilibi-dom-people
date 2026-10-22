//! Skills (`people/skills`, Bilimbi `Bilimbi.People.Skills`): a company's
//! skill catalog, its versioned proficiency scales, its versioned
//! requirement profiles and the policy values that steer assessments.
//!
//! This is the module's one facade. Pages and sibling modules call
//! [`Skills`], resolved from the registry as [`SERVICE`]; the schema, SQL,
//! validation and publish rules stay in private modules, so no caller can
//! bypass the capability check, the company lock or a rule by reaching a
//! lower layer. Every function takes the sealed [`Scope`] it was handed and
//! an explicit platform company id whose Workforce company read is current
//! (a stale or unavailable read is [`SkillsError::NotCurrent`]). Missing,
//! sibling-company and cross-tenant records are indistinguishable
//! (`NotFound`).
//!
//! Every write authorizes its own capability for the scope's signed-in actor
//! when it runs, through the Workforce `Authorization` helpers: catalog,
//! scale and profile-draft writes need `people.skills.catalog.manage`,
//! publishing and retiring a profile need `people.skills.profiles.publish`,
//! the policy write needs `people.skills.policy.manage`, and a system scope
//! cannot reach the helpers at all. A page's `can_*` only decides which
//! controls render; a grant revoked while the page stays open refuses the
//! next event. Each write runs inside a `MutationBoundary` transaction that
//! first holds the company's writable lock, so an archived company is
//! refused and a company's writes are serialized.
//!
//! Catalog and assessments (PK1–PK2) share this facade. Reassessment and
//! development action writes (PK3) and reminders (PK7) follow; pages land
//! in PK4–PK6.
//! See `README.md`.
mod access;
mod assessment_api;
mod assessment_types;
mod assessments;
mod catalog;
mod contributions;
mod policy;
mod positions;
mod profiles;
mod rows;
mod scales;
mod standing;
mod validate;

pub use assessment_types::*;
pub use contributions::{
    ASSESSMENTS_APPROVE, ASSESSMENTS_MANAGE, ASSESSMENTS_REVIEW, ASSESSMENTS_SUBMIT,
    ASSESSMENTS_VIEW, CAPABILITIES, CATALOG_MANAGE, CATALOG_VIEW, POLICY_MANAGE, PROFILES_PUBLISH,
    REASSESSMENTS_SUBMIT, SELF_VIEW,
};
pub use policy::{Policy, PolicyChanges, POLICY_KEYS};

use bilibi_company::{organisation, WriteError};
use bilibi_database::{Db, Error, MutationBoundary, MutationTransaction};
use bilibi_people_workforce::{authorization, NotCurrent, Workforce, WorkforceError};
use bilibi_settings::Settings;
use bilibi_tenancy::Scope;
use jiff::civil::{Date, DateTime};
use rust_decimal::Decimal;
use serde::Serialize;
use std::sync::Arc;
use tokio_postgres::{Client, Transaction};

pub use bilibi_people_workforce::FieldError;

/// The module ID, Bilimbi's, kept because IDs are persisted data.
pub const MODULE: &str = "people/skills";
/// The registry service ID of the [`Skills`] facade.
pub const SERVICE: &str = "people/skills.skills";
/// The three lifecycle states of a scale or profile version.
pub const STATUSES: [&str; 3] = ["draft", "published", "retired"];
/// The three criticalities of a requirement.
pub const CRITICALITIES: [&str; 3] = ["critical", "essential", "development"];
/// The highest level number a scale or requirement may use.
pub const MAX_LEVEL: i32 = 20;

pub type Result<T> = std::result::Result<T, SkillsError>;

/// Why a call did not happen: Bilimbi's refusal atoms, each with the
/// sentence its pages show, plus the field errors of a changeset.
#[derive(Debug, thiserror::Error)]
pub enum SkillsError {
    /// A denied or unknown capability, or a company outside the actor's reach.
    #[error("You cannot do that for this company.")]
    Unauthorized,
    /// A company or record the scope cannot see: missing, archived, of a
    /// sibling company or another tenant's (indistinguishable on purpose).
    #[error("That record is not available to you.")]
    NotFound,
    /// The Workforce company read is stale or unavailable.
    #[error("The workforce is not current, so nothing was changed.")]
    NotCurrent(NotCurrent),
    #[error("This company is archived and read-only")]
    CompanyArchived,
    /// Another scale or profile of the company already has this code.
    #[error("That code is already in use.")]
    CodeExists,
    /// The code already has an open draft.
    #[error("That code already has an open draft.")]
    DraftExists,
    /// The version is published or retired, so it cannot be edited or discarded.
    #[error("Only a draft can be changed.")]
    NotDraft,
    /// Only a published version can be retired.
    #[error("Only a published version can be retired.")]
    NotPublished,
    #[error(
        "A scale needs at least two levels numbered from 0 without gaps and with distinct names."
    )]
    InvalidLevels,
    #[error("A requirement profile uses this draft.")]
    ScaleInUse,
    #[error("The profile's scale is no longer published.")]
    ScaleUnavailable,
    #[error("Deactivate the category's active skills first.")]
    CategoryInUse,
    /// The category is missing, of another company, or inactive where an
    /// active one is needed.
    #[error("Choose an active category.")]
    CategoryUnavailable,
    #[error("Every skill must be active in this company.")]
    SkillUnavailable,
    #[error("Each required level must exist on the scale; change those requirements first.")]
    LevelNotOnScale,
    #[error("Add at least one skill requirement.")]
    NoItems,
    #[error("Target the company or at least one position.")]
    NoSelectors,
    #[error("Requirement weights must total 100.")]
    WeightsNot100,
    /// The position is not one the Workforce position read exposes, or
    /// that read is unavailable.
    #[error("That position is not available from Organisation.")]
    PositionUnavailable,
    #[error("A profile targets the whole company or positions, not both.")]
    MixedSelectors,
    #[error("That target is already listed.")]
    DuplicateSelector,
    #[error("The new version must start after the current version's start date.")]
    NotAfterLatest,
    #[error("Another requirement profile already targets some of these employees from that date.")]
    OverlappingProfile,
    #[error("Choose a valid date on or after the version's start.")]
    InvalidDate,
    /// More than one published or retired version is in force on the day.
    #[error("More than one requirement profile is in force on that day.")]
    Ambiguous,
    #[error("Enter whole numbers within the shown limits.")]
    InvalidPolicy,
    #[error("That employee is outside the people you may act on.")]
    OutOfReach,
    #[error("You cannot assess yourself.")]
    SelfAssessment,
    #[error("You cannot decide on work you did or that concerns you.")]
    SelfApproval,
    #[error("No published requirement covers that skill for this employee.")]
    NoRequirement,
    #[error("That submission was already used for a different record.")]
    KeyConflict,
    #[error("An assessment cannot be dated in the future.")]
    FutureAssessment,
    #[error("The validity date cannot be before the assessment date.")]
    InvalidValidity,
    #[error("That assessment cannot be corrected.")]
    NotSupersedable,
    #[error("Only the original assessor can correct a returned assessment.")]
    NotOriginalAssessor,
    #[error("That assessment already has a correction.")]
    AlreadySuperseded,
    #[error("That item is no longer pending.")]
    NotPending,
    #[error("Only a verified assessment can be finalized.")]
    NotVerified,
    #[error("A note is required to return an assessment.")]
    NoteRequired,
    #[error("Check the assessment details.")]
    InvalidAssessment,
    #[error("Check the decision note.")]
    InvalidNote,
    #[error("A working employee must be linked to your account.")]
    Unavailable,
    /// Refused values, in the words a form shows beside each field.
    #[error("Check the entered values.")]
    Invalid(Vec<FieldError>),
    /// A database or capture failure; never a refusal.
    #[error(transparent)]
    Failed(Error),
}
impl From<Error> for SkillsError {
    fn from(error: Error) -> Self {
        match error {
            Error::Forbidden | Error::Unauthorized => Self::Unauthorized,
            Error::NotFound => Self::NotFound,
            other => Self::Failed(other),
        }
    }
}
impl From<WorkforceError> for SkillsError {
    fn from(error: WorkforceError) -> Self {
        match error {
            WorkforceError::Unauthorized => Self::Unauthorized,
            WorkforceError::NotFound => Self::NotFound,
            WorkforceError::CompanyArchived => Self::CompanyArchived,
            WorkforceError::Invalid(errors) => Self::Invalid(errors),
            WorkforceError::NotLinked => Self::Unavailable,
            WorkforceError::Failed(error) => Self::Failed(error),
            _ => Self::NotFound,
        }
    }
}
impl From<NotCurrent> for SkillsError {
    fn from(error: NotCurrent) -> Self {
        Self::NotCurrent(error)
    }
}
impl From<bilibi_party_ids::InvalidId> for SkillsError {
    fn from(_: bilibi_party_ids::InvalidId) -> Self {
        Self::NotFound
    }
}
impl From<WriteError> for SkillsError {
    fn from(error: WriteError) -> Self {
        match error {
            WriteError::NotFound => Self::NotFound,
            WriteError::Archived => Self::CompanyArchived,
            WriteError::Access(error) => error.into(),
            _ => Self::Unauthorized,
        }
    }
}

fn as_optional_text<T: std::fmt::Display, S: serde::Serializer>(
    value: &Option<T>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    match value {
        Some(value) => serializer.collect_str(value),
        None => serializer.serialize_none(),
    }
}

/// A skill category of the company (Bilimbi `category_view/1`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Category {
    pub id: i64,
    pub code: String,
    pub name: String,
    pub description: Option<String>,
    pub active: bool,
}

/// What starts a category: a lowercase code unique in the company, a name
/// and an optional description.
#[derive(Clone, Debug, Default)]
pub struct NewCategory {
    pub code: String,
    pub name: String,
    pub description: Option<String>,
}

/// A category's revisable facts; its code is stable.
#[derive(Clone, Debug, Default)]
pub struct CategoryRevision {
    pub name: String,
    pub description: Option<String>,
}

/// A skill of the company (Bilimbi `skill_view/1`). The code is the stable
/// identity; a revision changes everything else but the company.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Skill {
    pub id: i64,
    pub category_id: i64,
    pub code: String,
    pub name: String,
    pub definition: String,
    pub evidence_guide: Option<String>,
    pub critical: bool,
    pub reassessment_months: Option<i32>,
    pub active: bool,
}

/// What starts a skill: an active category of the company, a lowercase code
/// unique in the company, a name, a definition, an optional evidence guide,
/// the critical flag and an optional reassessment interval in months (1–120).
#[derive(Clone, Debug, Default)]
pub struct NewSkill {
    pub category_id: i64,
    pub code: String,
    pub name: String,
    pub definition: String,
    pub evidence_guide: Option<String>,
    pub critical: bool,
    pub reassessment_months: Option<i32>,
}

/// A skill's revisable facts: everything but its code, company and
/// availability.
#[derive(Clone, Debug, Default)]
pub struct SkillRevision {
    pub category_id: i64,
    pub name: String,
    pub definition: String,
    pub evidence_guide: Option<String>,
    pub critical: bool,
    pub reassessment_months: Option<i32>,
}

/// One level of a proficiency scale (Bilimbi `level_view/1`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Level {
    pub level: i32,
    pub name: String,
    pub anchor: String,
    pub authority: String,
}

/// One version of a proficiency scale code with its levels, lowest first
/// (Bilimbi `scale_view/2`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Scale {
    pub id: i64,
    pub code: String,
    pub name: String,
    pub version: i32,
    pub status: String,
    #[serde(serialize_with = "as_optional_text")]
    pub published_at: Option<DateTime>,
    #[serde(serialize_with = "as_optional_text")]
    pub retired_at: Option<DateTime>,
    pub levels: Vec<Level>,
}

/// What starts version 1 of a scale code: the code and a name.
#[derive(Clone, Debug, Default)]
pub struct NewScale {
    pub code: String,
    pub name: String,
}

/// One version of a requirement profile code (Bilimbi `profile_view/1`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Profile {
    pub id: i64,
    pub code: String,
    pub name: String,
    pub version: i32,
    pub status: String,
    pub scale_id: i64,
    #[serde(serialize_with = "as_optional_text")]
    pub effective_from: Option<Date>,
    #[serde(serialize_with = "as_optional_text")]
    pub effective_to: Option<Date>,
    #[serde(serialize_with = "as_optional_text")]
    pub published_at: Option<DateTime>,
    #[serde(serialize_with = "as_optional_text")]
    pub retired_at: Option<DateTime>,
}

/// One profile version with its ordered requirements and its targets
/// (Bilimbi `full_profile_view/2`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProfileDetail {
    #[serde(flatten)]
    pub profile: Profile,
    pub items: Vec<Item>,
    pub selectors: Vec<Selector>,
}

/// The draft [`Skills::new_profile_version`] starts, and the required
/// levels the published version of its scale code lacks when the draft had
/// to keep the source's scale (`missing_levels` is empty when it adopted the
/// current scale version).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NewProfileVersion {
    #[serde(flatten)]
    pub profile: ProfileDetail,
    pub missing_levels: Vec<i32>,
}

/// What starts version 1 of a profile code: the code, a name and a
/// published scale of the company.
#[derive(Clone, Debug, Default)]
pub struct NewProfile {
    pub code: String,
    pub name: String,
    pub scale_id: i64,
}

/// What [`Skills::update_profile`] changes on a draft: a new name, another
/// published scale, or both; `None` keeps the current value.
#[derive(Clone, Debug, Default)]
pub struct ProfileChanges {
    pub name: Option<String>,
    pub scale_id: Option<i64>,
}

/// One skill requirement of a profile version (Bilimbi `item_view/1`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Item {
    pub id: i64,
    pub skill_id: i64,
    pub sequence: i32,
    pub required_level: i32,
    pub criticality: String,
    pub weight_percent: Decimal,
    pub mandatory: bool,
    pub evidence_standard: Option<String>,
}

/// A requirement as a draft takes it: an active skill of the company, a
/// level on the profile's scale, a criticality, a weight (0–100, at most two
/// decimals), the mandatory flag and an optional evidence standard.
#[derive(Clone, Debug)]
pub struct ItemInput {
    pub skill_id: i64,
    pub required_level: i32,
    pub criticality: String,
    pub weight_percent: Decimal,
    pub mandatory: bool,
    pub evidence_standard: Option<String>,
}

/// Where a requirement moves within its draft.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
}

/// Whom a profile version targets: the whole company, or one position the
/// Workforce position read exposes. A profile targets one or the other,
/// never both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Company,
    Position(i64),
}

/// One target of a profile version (Bilimbi `selector_view/1`):
/// `selector_type` is `company` or `position`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Selector {
    pub id: i64,
    pub selector_type: String,
    pub position_id: Option<i64>,
}

/// A position a profile may target, as the Workforce position read names
/// it (Bilimbi `position_option/1`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PositionOption {
    pub id: i64,
    pub code: String,
    pub title: String,
}

/// The facade, resolved from the registry as [`SERVICE`].
pub struct Skills {
    db: Db,
    mutations: Arc<MutationBoundary>,
    settings: Arc<Settings>,
    workforce: Arc<Workforce>,
}

/// A write's standing: the company lock is held on the transaction, and
/// authorization is rechecked under it after the company read proved current.
struct Write<'a> {
    tx: MutationTransaction<'a>,
    tenant: i64,
    company_id: i64,
    actor: i64,
}
impl<'a> Write<'a> {
    /// The transaction, for the reads that decide a refusal before the write.
    fn sql(&self) -> &Transaction<'a> {
        self.tx.sql()
    }
}

impl Skills {
    /// The capability that publishes and retires a profile.
    pub fn publish_capability() -> &'static str {
        PROFILES_PUBLISH
    }

    // ---- Categories -------------------------------------------------------

    /// The company's categories, active first, then by name.
    pub async fn list_categories(&self, scope: &Scope, company_id: i64) -> Result<Vec<Category>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.current_company(scope, company_id).await?;
        catalog::list_categories(&**client, scope.tenant(), company_id).await
    }

    /// Adds an active category (`people.skills.catalog.manage`): a lowercase
    /// code unique in the company and a name.
    pub async fn create_category(
        &self,
        scope: &Scope,
        company_id: i64,
        new: &NewCategory,
    ) -> Result<Category> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let valid = validate::new_category(new)?;
        let mut write = self
            .begin(
                &mut client,
                scope,
                company_id,
                CATALOG_MANAGE,
                "create_category",
            )
            .await?;
        let category = catalog::insert_category(&mut write, &valid).await?;
        write.tx.commit().await?;
        Ok(category)
    }

    /// Renames or describes a category; its code is stable.
    pub async fn update_category(
        &self,
        scope: &Scope,
        company_id: i64,
        category_id: i64,
        revision: &CategoryRevision,
    ) -> Result<Category> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let valid = validate::category_revision(revision)?;
        let mut write = self
            .begin(
                &mut client,
                scope,
                company_id,
                CATALOG_MANAGE,
                "update_category",
            )
            .await?;
        let current = catalog::lock_category(&write, category_id)
            .await?
            .ok_or(SkillsError::NotFound)?;
        let category = catalog::update_category(&mut write, current, valid).await?;
        write.tx.commit().await?;
        Ok(category)
    }

    /// Activates or deactivates a category. Deactivation is refused while
    /// the category has active skills (`CategoryInUse`); nothing is deleted.
    pub async fn set_category_active(
        &self,
        scope: &Scope,
        company_id: i64,
        category_id: i64,
        active: bool,
    ) -> Result<Category> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut write = self
            .begin(
                &mut client,
                scope,
                company_id,
                CATALOG_MANAGE,
                "set_category_active",
            )
            .await?;
        let current = catalog::lock_category(&write, category_id)
            .await?
            .ok_or(SkillsError::NotFound)?;
        if !active && catalog::has_active_skills(&write, current.id).await? {
            return Err(SkillsError::CategoryInUse);
        }
        if current.active == active {
            write.tx.rollback().await?;
            return Ok(current);
        }
        let category = catalog::set_category_active(&mut write, current, active).await?;
        write.tx.commit().await?;
        Ok(category)
    }

    // ---- Skills -----------------------------------------------------------

    /// The company's skills, active first, then by name.
    pub async fn list_skills(&self, scope: &Scope, company_id: i64) -> Result<Vec<Skill>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.current_company(scope, company_id).await?;
        catalog::list_skills(&**client, scope.tenant(), company_id).await
    }

    /// Adds an active skill under an active category of the same company.
    pub async fn create_skill(
        &self,
        scope: &Scope,
        company_id: i64,
        new: &NewSkill,
    ) -> Result<Skill> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let valid = validate::new_skill(new)?;
        let mut write = self
            .begin(
                &mut client,
                scope,
                company_id,
                CATALOG_MANAGE,
                "create_skill",
            )
            .await?;
        catalog::require_category(&write, valid.category_id, true).await?;
        let skill = catalog::insert_skill(&mut write, &valid).await?;
        write.tx.commit().await?;
        Ok(skill)
    }

    /// Revises a skill: its name, definition, evidence guide, critical flag,
    /// reassessment interval and category. The code is the stable identity
    /// and never changes; an active skill needs an active category.
    pub async fn update_skill(
        &self,
        scope: &Scope,
        company_id: i64,
        skill_id: i64,
        revision: &SkillRevision,
    ) -> Result<Skill> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let valid = validate::skill_revision(revision)?;
        let mut write = self
            .begin(
                &mut client,
                scope,
                company_id,
                CATALOG_MANAGE,
                "update_skill",
            )
            .await?;
        let current = catalog::lock_skill(&write, skill_id)
            .await?
            .ok_or(SkillsError::NotFound)?;
        catalog::require_category(&write, valid.category_id, current.active).await?;
        let skill = catalog::update_skill(&mut write, current, valid).await?;
        write.tx.commit().await?;
        Ok(skill)
    }

    /// Activates or deactivates a skill. Reactivation needs an active
    /// category; history is never deleted.
    pub async fn set_skill_active(
        &self,
        scope: &Scope,
        company_id: i64,
        skill_id: i64,
        active: bool,
    ) -> Result<Skill> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut write = self
            .begin(
                &mut client,
                scope,
                company_id,
                CATALOG_MANAGE,
                "set_skill_active",
            )
            .await?;
        let current = catalog::lock_skill(&write, skill_id)
            .await?
            .ok_or(SkillsError::NotFound)?;
        if active {
            catalog::require_category(&write, current.category_id, true).await?;
        }
        if current.active == active {
            write.tx.rollback().await?;
            return Ok(current);
        }
        let skill = catalog::set_skill_active(&mut write, current, active).await?;
        write.tx.commit().await?;
        Ok(skill)
    }

    // ---- Proficiency scales ----------------------------------------------

    /// Scale versions with their levels: by code, newest version first.
    pub async fn list_scales(&self, scope: &Scope, company_id: i64) -> Result<Vec<Scale>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.current_company(scope, company_id).await?;
        scales::list(&**client, scope.tenant(), company_id).await
    }

    /// Starts version 1 of a new scale code as a draft without levels. A
    /// code any version of the company already uses is `CodeExists`.
    pub async fn create_scale(
        &self,
        scope: &Scope,
        company_id: i64,
        new: &NewScale,
    ) -> Result<Scale> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let valid = validate::new_scale(new)?;
        let mut write = self
            .begin(
                &mut client,
                scope,
                company_id,
                CATALOG_MANAGE,
                "create_scale",
            )
            .await?;
        if scales::code_exists(&write, &valid.code).await? {
            return Err(SkillsError::CodeExists);
        }
        let scale = scales::insert(&mut write, &valid.code, &valid.name, 1).await?;
        write.tx.commit().await?;
        Ok(scale)
    }

    /// Drafts the next version of a scale code, copying the chosen
    /// version's levels. A code with an open draft is `DraftExists`.
    pub async fn new_scale_version(
        &self,
        scope: &Scope,
        company_id: i64,
        scale_id: i64,
    ) -> Result<Scale> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut write = self
            .begin(
                &mut client,
                scope,
                company_id,
                CATALOG_MANAGE,
                "new_scale_version",
            )
            .await?;
        let source = scales::get(&write, scale_id)
            .await?
            .ok_or(SkillsError::NotFound)?;
        if scales::open_draft(&write, &source.code).await? {
            return Err(SkillsError::DraftExists);
        }
        let version = scales::next_version(&write, &source.code).await?;
        let draft = scales::insert(&mut write, &source.code, &source.name, version).await?;
        let levels = scales::levels_of(write.sql(), source.id).await?;
        let scale = scales::copy_levels(&mut write, draft, &levels).await?;
        write.tx.commit().await?;
        Ok(scale)
    }

    /// Renames a draft scale.
    pub async fn rename_scale(
        &self,
        scope: &Scope,
        company_id: i64,
        scale_id: i64,
        name: &str,
    ) -> Result<Scale> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let name = validate::name(name)?;
        let mut write = self
            .begin(
                &mut client,
                scope,
                company_id,
                CATALOG_MANAGE,
                "rename_scale",
            )
            .await?;
        let draft = scales::lock_draft(&write, scale_id).await?;
        let scale = scales::rename(&mut write, draft, name).await?;
        write.tx.commit().await?;
        Ok(scale)
    }

    /// Adds or replaces one level of a draft scale, keyed by its level
    /// number (0–20): a name, an observable anchor and the authority the
    /// level grants.
    pub async fn put_scale_level(
        &self,
        scope: &Scope,
        company_id: i64,
        scale_id: i64,
        level: &Level,
    ) -> Result<Level> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let valid = validate::level(level)?;
        let mut write = self
            .begin(
                &mut client,
                scope,
                company_id,
                CATALOG_MANAGE,
                "put_scale_level",
            )
            .await?;
        let draft = scales::lock_draft(&write, scale_id).await?;
        let level = scales::put_level(&mut write, &draft, valid).await?;
        write.tx.commit().await?;
        Ok(level)
    }

    /// Removes one level of a draft scale; a level the scale lacks is
    /// `NotFound`.
    pub async fn delete_scale_level(
        &self,
        scope: &Scope,
        company_id: i64,
        scale_id: i64,
        level: i32,
    ) -> Result<()> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut write = self
            .begin(
                &mut client,
                scope,
                company_id,
                CATALOG_MANAGE,
                "delete_scale_level",
            )
            .await?;
        let draft = scales::lock_draft(&write, scale_id).await?;
        scales::delete_level(&mut write, &draft, level).await?;
        write.tx.commit().await?;
        Ok(())
    }

    /// Publishes a draft whose levels run contiguously from 0 with at least
    /// two levels and distinct names (`InvalidLevels` otherwise). The
    /// previously published version of the code is retired in the same
    /// transaction, so one version is current.
    pub async fn publish_scale(
        &self,
        scope: &Scope,
        company_id: i64,
        scale_id: i64,
    ) -> Result<Scale> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut write = self
            .begin(
                &mut client,
                scope,
                company_id,
                CATALOG_MANAGE,
                "publish_scale",
            )
            .await?;
        let draft = scales::lock_draft(&write, scale_id).await?;
        let levels = scales::levels_of(write.sql(), draft.id).await?;
        if !validate::publishable_levels(&levels) {
            return Err(SkillsError::InvalidLevels);
        }
        let previous = scales::lock_published(&write, &draft.code).await?;
        let scale = scales::publish(&mut write, draft, levels, previous).await?;
        write.tx.commit().await?;
        Ok(scale)
    }

    /// Retires a published scale; any other status is `NotPublished`.
    pub async fn retire_scale(
        &self,
        scope: &Scope,
        company_id: i64,
        scale_id: i64,
    ) -> Result<Scale> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut write = self
            .begin(
                &mut client,
                scope,
                company_id,
                CATALOG_MANAGE,
                "retire_scale",
            )
            .await?;
        let scale = scales::lock(&write, scale_id)
            .await?
            .ok_or(SkillsError::NotFound)?;
        if scale.status != "published" {
            return Err(SkillsError::NotPublished);
        }
        let scale = scales::retire(&mut write, scale).await?;
        write.tx.commit().await?;
        Ok(scale)
    }

    /// Discards a draft scale, with its levels, that no requirement profile
    /// uses (`ScaleInUse` otherwise).
    pub async fn discard_scale(&self, scope: &Scope, company_id: i64, scale_id: i64) -> Result<()> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut write = self
            .begin(
                &mut client,
                scope,
                company_id,
                CATALOG_MANAGE,
                "discard_scale",
            )
            .await?;
        let draft = scales::lock_draft(&write, scale_id).await?;
        if profiles::uses_scale(&write, draft.id).await? {
            return Err(SkillsError::ScaleInUse);
        }
        let levels = scales::levels_of(write.sql(), draft.id).await?;
        scales::discard(&mut write, draft, levels).await?;
        write.tx.commit().await?;
        Ok(())
    }

    // ---- Requirement profiles -------------------------------------------

    /// Profile versions: by code, newest version first.
    pub async fn list_profiles(&self, scope: &Scope, company_id: i64) -> Result<Vec<Profile>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.current_company(scope, company_id).await?;
        profiles::list(&**client, scope.tenant(), company_id).await
    }

    /// One profile version with its ordered items and selectors.
    pub async fn get_profile(
        &self,
        scope: &Scope,
        company_id: i64,
        profile_id: i64,
    ) -> Result<ProfileDetail> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.current_company(scope, company_id).await?;
        let profile = profiles::get(&**client, scope.tenant(), company_id, profile_id)
            .await?
            .ok_or(SkillsError::NotFound)?;
        profiles::detail(&**client, profile).await
    }

    /// Starts version 1 of a new profile code against a published scale of
    /// the company (`ScaleUnavailable` otherwise); a code any version
    /// already uses is `CodeExists`.
    pub async fn create_profile(
        &self,
        scope: &Scope,
        company_id: i64,
        new: &NewProfile,
    ) -> Result<Profile> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let valid = validate::new_profile(new)?;
        let mut write = self
            .begin(
                &mut client,
                scope,
                company_id,
                CATALOG_MANAGE,
                "create_profile",
            )
            .await?;
        let scale = scales::require_published(&write, valid.scale_id).await?;
        if profiles::code_exists(&write, &valid.code).await? {
            return Err(SkillsError::CodeExists);
        }
        let profile = profiles::insert(&mut write, &valid.code, &valid.name, 1, scale.id).await?;
        write.tx.commit().await?;
        Ok(profile)
    }

    /// Renames a draft or moves it to another published scale, or both.
    /// Moving scales requires every requirement's level to exist on the new
    /// scale (`LevelNotOnScale`).
    pub async fn update_profile(
        &self,
        scope: &Scope,
        company_id: i64,
        profile_id: i64,
        changes: &ProfileChanges,
    ) -> Result<Profile> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let name = changes.name.as_deref().map(validate::name).transpose()?;
        let mut write = self
            .begin(
                &mut client,
                scope,
                company_id,
                CATALOG_MANAGE,
                "update_profile",
            )
            .await?;
        let draft = profiles::lock_draft(&write, profile_id).await?;
        let scale_id = match changes.scale_id {
            Some(scale_id) => {
                let scale = scales::require_published(&write, scale_id).await?;
                let levels = scales::level_numbers(write.sql(), scale.id).await?;
                let items = profiles::items(write.sql(), draft.id).await?;
                if items
                    .iter()
                    .any(|item| !levels.contains(&item.required_level))
                {
                    return Err(SkillsError::LevelNotOnScale);
                }
                Some(scale.id)
            }
            None => None,
        };
        if name.is_none() && scale_id.is_none() {
            write.tx.rollback().await?;
            return Ok(draft);
        }
        let profile = profiles::update(&mut write, draft, name, scale_id).await?;
        write.tx.commit().await?;
        Ok(profile)
    }

    /// Drafts the next version of a profile code, copying items and
    /// selectors. The draft adopts the published version of the source's
    /// scale code when every required level exists on it; otherwise it keeps
    /// the source's scale and `missing_levels` lists the required levels
    /// that version lacks. A code with an open draft is `DraftExists`.
    pub async fn new_profile_version(
        &self,
        scope: &Scope,
        company_id: i64,
        profile_id: i64,
    ) -> Result<NewProfileVersion> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut write = self
            .begin(
                &mut client,
                scope,
                company_id,
                CATALOG_MANAGE,
                "new_profile_version",
            )
            .await?;
        let source = profiles::lock(&write, profile_id)
            .await?
            .ok_or(SkillsError::NotFound)?;
        if profiles::open_draft(&write, &source.code).await? {
            return Err(SkillsError::DraftExists);
        }
        let items = profiles::items(write.sql(), source.id).await?;
        let selectors = profiles::selectors(write.sql(), source.id).await?;
        let (scale_id, missing_levels) =
            scales::current_scale(&write, source.scale_id, &items).await?;
        let version = profiles::next_version(&write, &source.code).await?;
        let draft =
            profiles::insert(&mut write, &source.code, &source.name, version, scale_id).await?;
        let profile = profiles::copy_children(&mut write, draft, &items, &selectors).await?;
        write.tx.commit().await?;
        Ok(NewProfileVersion {
            profile,
            missing_levels,
        })
    }

    /// Adds a skill requirement to a draft, or replaces the requirement for
    /// that skill. The skill must be active in the same company
    /// (`SkillUnavailable`) and the required level must exist on the
    /// profile's scale (`LevelNotOnScale`).
    pub async fn put_item(
        &self,
        scope: &Scope,
        company_id: i64,
        profile_id: i64,
        input: &ItemInput,
    ) -> Result<Item> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let valid = validate::item(input)?;
        let mut write = self
            .begin(&mut client, scope, company_id, CATALOG_MANAGE, "put_item")
            .await?;
        let draft = profiles::lock_draft(&write, profile_id).await?;
        match catalog::get_skill(&write, valid.skill_id).await? {
            Some(skill) if skill.active => {}
            _ => return Err(SkillsError::SkillUnavailable),
        }
        if !scales::level_numbers(write.sql(), draft.scale_id)
            .await?
            .contains(&valid.required_level)
        {
            return Err(SkillsError::LevelNotOnScale);
        }
        let items = profiles::items(write.sql(), draft.id).await?;
        let item = profiles::put_item(&mut write, &draft, &items, valid).await?;
        write.tx.commit().await?;
        Ok(item)
    }

    /// Removes a requirement from a draft and closes the gap in the sequence.
    pub async fn remove_item(
        &self,
        scope: &Scope,
        company_id: i64,
        profile_id: i64,
        item_id: i64,
    ) -> Result<()> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut write = self
            .begin(
                &mut client,
                scope,
                company_id,
                CATALOG_MANAGE,
                "remove_item",
            )
            .await?;
        let draft = profiles::lock_draft(&write, profile_id).await?;
        let items = profiles::items(write.sql(), draft.id).await?;
        if !items.iter().any(|item| item.id == item_id) {
            return Err(SkillsError::NotFound);
        }
        profiles::remove_item(&mut write, &draft, items, item_id).await?;
        write.tx.commit().await?;
        Ok(())
    }

    /// Moves a draft requirement one place up or down; at either end it
    /// stays where it is.
    pub async fn move_item(
        &self,
        scope: &Scope,
        company_id: i64,
        profile_id: i64,
        item_id: i64,
        direction: Direction,
    ) -> Result<()> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut write = self
            .begin(&mut client, scope, company_id, CATALOG_MANAGE, "move_item")
            .await?;
        let draft = profiles::lock_draft(&write, profile_id).await?;
        let mut items = profiles::items(write.sql(), draft.id).await?;
        let index = items
            .iter()
            .position(|item| item.id == item_id)
            .ok_or(SkillsError::NotFound)?;
        let target = match direction {
            Direction::Up => index.checked_sub(1),
            Direction::Down => Some(index + 1).filter(|target| *target < items.len()),
        };
        let Some(target) = target else {
            write.tx.rollback().await?;
            return Ok(());
        };
        items.swap(index, target);
        profiles::resequence(&mut write, &draft, items).await?;
        write.tx.commit().await?;
        Ok(())
    }

    /// Targets a draft at the whole company, or at one position that the
    /// Workforce position read exposes (`PositionUnavailable` otherwise). A
    /// profile targets either the company or a set of positions, never both
    /// (`MixedSelectors`); a target listed twice is `DuplicateSelector`.
    pub async fn add_selector(
        &self,
        scope: &Scope,
        company_id: i64,
        profile_id: i64,
        target: Target,
    ) -> Result<Selector> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, CATALOG_MANAGE).await?;
        self.current_company(scope, company_id).await?;
        if let Target::Position(position_id) = target {
            let options = positions::options(&self.workforce, scope, company_id).await?;
            if !options.iter().any(|option| option.id == position_id) {
                return Err(SkillsError::PositionUnavailable);
            }
        }
        let mut write = self
            .begin(
                &mut client,
                scope,
                company_id,
                CATALOG_MANAGE,
                "add_selector",
            )
            .await?;
        let draft = profiles::lock_draft(&write, profile_id).await?;
        let (selector_type, position_id) = match target {
            Target::Company => ("company", None),
            Target::Position(id) => ("position", Some(id)),
        };
        let existing = profiles::selectors(write.sql(), draft.id).await?;
        if existing
            .iter()
            .any(|selector| selector.selector_type != selector_type)
        {
            return Err(SkillsError::MixedSelectors);
        }
        if existing.iter().any(|selector| {
            selector.selector_type == selector_type && selector.position_id == position_id
        }) {
            return Err(SkillsError::DuplicateSelector);
        }
        let selector =
            profiles::insert_selector(&mut write, &draft, selector_type, position_id).await?;
        write.tx.commit().await?;
        Ok(selector)
    }

    /// Removes one target of a draft.
    pub async fn remove_selector(
        &self,
        scope: &Scope,
        company_id: i64,
        profile_id: i64,
        selector_id: i64,
    ) -> Result<()> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut write = self
            .begin(
                &mut client,
                scope,
                company_id,
                CATALOG_MANAGE,
                "remove_selector",
            )
            .await?;
        let draft = profiles::lock_draft(&write, profile_id).await?;
        let selector = profiles::selectors(write.sql(), draft.id)
            .await?
            .into_iter()
            .find(|selector| selector.id == selector_id)
            .ok_or(SkillsError::NotFound)?;
        profiles::remove_selector(&mut write, &draft, selector).await?;
        write.tx.commit().await?;
        Ok(())
    }

    /// Discards a draft profile with its items and selectors.
    pub async fn discard_profile(
        &self,
        scope: &Scope,
        company_id: i64,
        profile_id: i64,
    ) -> Result<()> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut write = self
            .begin(
                &mut client,
                scope,
                company_id,
                CATALOG_MANAGE,
                "discard_profile",
            )
            .await?;
        let draft = profiles::lock_draft(&write, profile_id).await?;
        let items = profiles::items(write.sql(), draft.id).await?;
        let selectors = profiles::selectors(write.sql(), draft.id).await?;
        profiles::discard(&mut write, draft, items, selectors).await?;
        write.tx.commit().await?;
        Ok(())
    }

    /// Publishes a draft from `effective_from`
    /// (`people.skills.profiles.publish`). The draft needs at least one
    /// requirement (`NoItems`) and one target (`NoSelectors`), weights
    /// totalling 100 (`WeightsNot100`), active skills (`SkillUnavailable`),
    /// levels on a published scale (`ScaleUnavailable`, `LevelNotOnScale`)
    /// and positions still exposed by the Workforce position read
    /// (`PositionUnavailable`). It must start after the latest published
    /// version of its code (`NotAfterLatest`), which it retires the day
    /// before, and it may not target anyone that another code's published
    /// or retired version targets from that date on (`OverlappingProfile`).
    pub async fn publish_profile(
        &self,
        scope: &Scope,
        company_id: i64,
        profile_id: i64,
        effective_from: Date,
    ) -> Result<Profile> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, PROFILES_PUBLISH).await?;
        self.current_company(scope, company_id).await?;
        // The positions the company exposes today, read before the
        // transaction so the lock is not held across the read.
        let draft = profiles::get(&**client, scope.tenant(), company_id, profile_id)
            .await?
            .ok_or(SkillsError::NotFound)?;
        let wanted = profiles::selectors(&**client, draft.id).await?;
        let known_positions = if wanted.iter().any(|selector| selector.position_id.is_some()) {
            Some(
                positions::options(&self.workforce, scope, company_id)
                    .await
                    .map_err(|_| SkillsError::PositionUnavailable)?,
            )
        } else {
            None
        };
        let mut write = self
            .begin(
                &mut client,
                scope,
                company_id,
                PROFILES_PUBLISH,
                "publish_profile",
            )
            .await?;
        let draft = profiles::lock_draft(&write, profile_id).await?;
        let items = profiles::items(write.sql(), draft.id).await?;
        let selectors = profiles::selectors(write.sql(), draft.id).await?;
        profiles::validate_publishable(
            &write,
            &draft,
            &items,
            &selectors,
            known_positions.as_deref(),
        )
        .await?;
        if !profiles::after_code_history(&write, &draft.code, effective_from).await? {
            return Err(SkillsError::NotAfterLatest);
        }
        if profiles::overlapping(&write, &draft, &selectors, effective_from).await? {
            return Err(SkillsError::OverlappingProfile);
        }
        let previous = profiles::lock_published(&write, &draft.code).await?;
        let profile = profiles::publish(&mut write, draft, previous, effective_from).await?;
        write.tx.commit().await?;
        Ok(profile)
    }

    /// Retires a published profile after its last effective day
    /// (`people.skills.profiles.publish`); a day before the version's start
    /// is `InvalidDate`, any other status `NotPublished`.
    pub async fn retire_profile(
        &self,
        scope: &Scope,
        company_id: i64,
        profile_id: i64,
        effective_to: Date,
    ) -> Result<Profile> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut write = self
            .begin(
                &mut client,
                scope,
                company_id,
                PROFILES_PUBLISH,
                "retire_profile",
            )
            .await?;
        let profile = profiles::lock(&write, profile_id)
            .await?
            .ok_or(SkillsError::NotFound)?;
        if profile.status != "published" {
            return Err(SkillsError::NotPublished);
        }
        if profile
            .effective_from
            .is_some_and(|from| effective_to < from)
        {
            return Err(SkillsError::InvalidDate);
        }
        let profile = profiles::retire(&mut write, profile, effective_to).await?;
        write.tx.commit().await?;
        Ok(profile)
    }

    // ---- Resolution ------------------------------------------------------

    /// The requirements in force on `as_of` for the company, optionally for
    /// one position: the single published or retired version whose targets
    /// cover the company or that position, or `None` when none does. More
    /// than one is `Ambiguous`.
    pub async fn requirements(
        &self,
        scope: &Scope,
        company_id: i64,
        position_id: Option<i64>,
        as_of: Date,
    ) -> Result<Option<ProfileDetail>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.current_company(scope, company_id).await?;
        let mut matches =
            profiles::in_force(&**client, scope.tenant(), company_id, position_id, as_of).await?;
        match matches.len() {
            0 => Ok(None),
            1 => Ok(Some(profiles::detail(&**client, matches.remove(0)).await?)),
            _ => Err(SkillsError::Ambiguous),
        }
    }

    /// Positions a profile may target: every position the Workforce
    /// position read exposes for the company today, up to 2,000;
    /// `PositionUnavailable` while that read is unavailable (no Organisation
    /// mounted).
    pub async fn position_options(
        &self,
        scope: &Scope,
        company_id: i64,
    ) -> Result<Vec<PositionOption>> {
        self.current_company(scope, company_id).await?;
        positions::options(&self.workforce, scope, company_id).await
    }

    /// The position an employee substantively holds on a day, as the
    /// Workforce position read exposes it, or `None` when they hold none or
    /// no Organisation is mounted.
    pub async fn position_of(
        &self,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
        as_of: Date,
    ) -> Result<Option<i64>> {
        self.current_company(scope, company_id).await?;
        positions::position_of(&self.workforce, scope, company_id, employee_id, as_of).await
    }

    // ---- Company policy --------------------------------------------------

    /// The company's assessment, priority and reminder policy values.
    pub async fn policy(&self, scope: &Scope, company_id: i64) -> Result<Policy> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.current_company(scope, company_id).await?;
        policy::get(&self.settings, &**client, scope, company_id).await
    }

    /// Stores the given policy values (`people.skills.policy.manage`), each
    /// a whole number within its definition's range; values left `None`
    /// keep their current value. Nothing given, or a value out of range, is
    /// `InvalidPolicy`.
    pub async fn put_policy(
        &self,
        scope: &Scope,
        company_id: i64,
        changes: &PolicyChanges,
    ) -> Result<Policy> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        policy::validate(changes)?;
        let mut write = self
            .begin(&mut client, scope, company_id, POLICY_MANAGE, "put_policy")
            .await?;
        policy::put(&self.settings, &mut write.tx, scope, company_id, changes).await?;
        write.tx.commit().await?;
        self.policy(scope, company_id).await
    }

    // ---- Private ---------------------------------------------------------

    /// The company through the Workforce read, which must be current.
    async fn current_company(&self, scope: &Scope, company_id: i64) -> Result<()> {
        self.workforce
            .company(scope, company_id)
            .await?
            .require_current()?;
        Ok(())
    }

    /// Authorizes the capability for the company, proves the company read
    /// current, and begins the mutation transaction holding the company's
    /// writable lock. Every write starts here.
    async fn begin<'a>(
        &self,
        client: &'a mut Client,
        scope: &Scope,
        company_id: i64,
        capability: &str,
        action: &str,
    ) -> Result<Write<'a>> {
        authorization::authorize(&*client, scope, company_id, capability).await?;
        self.current_company(scope, company_id).await?;
        let tx = self
            .mutations
            .begin(
                client,
                scope.mutation_context(format!("people.skills.{action}")),
            )
            .await?;
        organisation::require_writable(tx.sql(), scope, company_id, true).await?;
        // A grant may have been revoked while this write waited for the
        // company lock. Authorize again on the transaction before mutating.
        authorization::authorize(tx.sql(), scope, company_id, capability).await?;
        Ok(Write {
            tx,
            tenant: scope.tenant(),
            company_id,
            actor: scope.user(),
        })
    }
}

impl Target {
    /// Bilimbi's `position_id/1`: the position of a position target.
    pub fn position_id(self) -> Option<i64> {
        match self {
            Target::Company => None,
            Target::Position(id) => Some(id),
        }
    }
}
