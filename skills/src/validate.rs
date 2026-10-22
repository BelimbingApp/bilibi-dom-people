//! Input validation in Bilimbi's changeset words (`Category`, `Skill`,
//! `Scale`, `ScaleLevel`, `Profile`, `ProfileItem` and `Code`): texts are
//! trimmed, lengths bounded, codes shaped, numbers ranged. Private to the
//! facade; every refusal is `SkillsError::Invalid` with one `FieldError` per
//! field.
use crate::{
    CategoryRevision, FieldError, ItemInput, Level, NewCategory, NewProfile, NewScale, NewSkill,
    Result, SkillRevision, SkillsError, CRITICALITIES, MAX_LEVEL,
};
use rust_decimal::Decimal;

/// A validated category.
#[derive(Clone, Debug)]
pub(crate) struct ValidCategory {
    pub code: String,
    pub name: String,
    pub description: Option<String>,
}

/// A validated skill.
#[derive(Clone, Debug)]
pub(crate) struct ValidSkill {
    pub category_id: i64,
    pub code: String,
    pub name: String,
    pub definition: String,
    pub evidence_guide: Option<String>,
    pub critical: bool,
    pub reassessment_months: Option<i32>,
}

/// A validated skill revision (everything but the code).
#[derive(Clone, Debug)]
pub(crate) struct ValidSkillRevision {
    pub category_id: i64,
    pub name: String,
    pub definition: String,
    pub evidence_guide: Option<String>,
    pub critical: bool,
    pub reassessment_months: Option<i32>,
}

/// A validated scale start.
#[derive(Clone, Debug)]
pub(crate) struct ValidScale {
    pub code: String,
    pub name: String,
}

/// A validated profile start.
#[derive(Clone, Debug)]
pub(crate) struct ValidProfile {
    pub code: String,
    pub name: String,
    pub scale_id: i64,
}

/// Collects field errors and ends in one `Invalid` refusal.
#[derive(Default)]
struct Errors(Vec<FieldError>);
impl Errors {
    fn push(&mut self, field: &'static str, message: impl Into<String>) {
        self.0.push(FieldError::new(field, message));
    }
    fn finish<T>(self, value: T) -> Result<T> {
        if self.0.is_empty() {
            Ok(value)
        } else {
            Err(SkillsError::Invalid(self.0))
        }
    }
}

/// A required text, trimmed and at most `max` characters.
fn required(errors: &mut Errors, field: &'static str, value: &str, max: usize) -> String {
    let value = value.trim().to_owned();
    if value.is_empty() {
        errors.push(field, "can't be blank");
    } else if value.chars().count() > max {
        errors.push(field, format!("should be at most {max} character(s)"));
    }
    value
}

/// An optional text, trimmed and at most `max` characters; blank is absent.
fn optional(
    errors: &mut Errors,
    field: &'static str,
    value: Option<&str>,
    max: usize,
) -> Option<String> {
    let value = value.map(str::trim).filter(|value| !value.is_empty())?;
    if value.chars().count() > max {
        errors.push(field, format!("should be at most {max} character(s)"));
    }
    Some(value.to_owned())
}

/// Codes are stable lowercase identifiers chosen by the company (Bilimbi
/// `Code.validate/2`: 1–80 characters, `^[a-z0-9][a-z0-9._-]*$`).
fn code(errors: &mut Errors, value: &str) -> String {
    let value = required(errors, "code", value, 80);
    if !value.is_empty() && !code_shape(&value) {
        errors.push("code", "has invalid format");
    }
    value
}

fn code_shape(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

fn positive(errors: &mut Errors, field: &'static str, value: i64) {
    if value <= 0 {
        errors.push(field, "is invalid");
    }
}

fn months(errors: &mut Errors, value: Option<i32>) {
    if let Some(months) = value {
        if months < 1 {
            errors.push("reassessment_months", "must be greater than or equal to 1");
        } else if months > 120 {
            errors.push("reassessment_months", "must be less than or equal to 120");
        }
    }
}

/// A name of at most 160 characters (scales and profiles).
pub(crate) fn name(value: &str) -> Result<String> {
    let mut errors = Errors::default();
    let name = required(&mut errors, "name", value, 160);
    errors.finish(name)
}

pub(crate) fn new_category(new: &NewCategory) -> Result<ValidCategory> {
    let mut errors = Errors::default();
    let code = code(&mut errors, &new.code);
    let name = required(&mut errors, "name", &new.name, 160);
    let description = optional(&mut errors, "description", new.description.as_deref(), 2000);
    errors.finish(ValidCategory {
        code,
        name,
        description,
    })
}

pub(crate) fn category_revision(revision: &CategoryRevision) -> Result<ValidCategory> {
    let mut errors = Errors::default();
    let name = required(&mut errors, "name", &revision.name, 160);
    let description = optional(
        &mut errors,
        "description",
        revision.description.as_deref(),
        2000,
    );
    errors.finish(ValidCategory {
        code: String::new(),
        name,
        description,
    })
}

pub(crate) fn new_skill(new: &NewSkill) -> Result<ValidSkill> {
    let mut errors = Errors::default();
    positive(&mut errors, "category_id", new.category_id);
    let code = code(&mut errors, &new.code);
    let name = required(&mut errors, "name", &new.name, 160);
    let definition = required(&mut errors, "definition", &new.definition, 2000);
    let evidence_guide = optional(
        &mut errors,
        "evidence_guide",
        new.evidence_guide.as_deref(),
        2000,
    );
    months(&mut errors, new.reassessment_months);
    errors.finish(ValidSkill {
        category_id: new.category_id,
        code,
        name,
        definition,
        evidence_guide,
        critical: new.critical,
        reassessment_months: new.reassessment_months,
    })
}

pub(crate) fn skill_revision(revision: &SkillRevision) -> Result<ValidSkillRevision> {
    let mut errors = Errors::default();
    positive(&mut errors, "category_id", revision.category_id);
    let name = required(&mut errors, "name", &revision.name, 160);
    let definition = required(&mut errors, "definition", &revision.definition, 2000);
    let evidence_guide = optional(
        &mut errors,
        "evidence_guide",
        revision.evidence_guide.as_deref(),
        2000,
    );
    months(&mut errors, revision.reassessment_months);
    errors.finish(ValidSkillRevision {
        category_id: revision.category_id,
        name,
        definition,
        evidence_guide,
        critical: revision.critical,
        reassessment_months: revision.reassessment_months,
    })
}

pub(crate) fn new_scale(new: &NewScale) -> Result<ValidScale> {
    let mut errors = Errors::default();
    let code = code(&mut errors, &new.code);
    let name = required(&mut errors, "name", &new.name, 160);
    errors.finish(ValidScale { code, name })
}

pub(crate) fn new_profile(new: &NewProfile) -> Result<ValidProfile> {
    let mut errors = Errors::default();
    let code = code(&mut errors, &new.code);
    let name = required(&mut errors, "name", &new.name, 160);
    positive(&mut errors, "scale_id", new.scale_id);
    errors.finish(ValidProfile {
        code,
        name,
        scale_id: new.scale_id,
    })
}

/// A level numbered 0–20 with a name (100), an anchor and an authority (2000).
pub(crate) fn level(level: &Level) -> Result<Level> {
    let mut errors = Errors::default();
    if level.level < 0 {
        errors.push("level", "must be greater than or equal to 0");
    } else if level.level > MAX_LEVEL {
        errors.push(
            "level",
            format!("must be less than or equal to {MAX_LEVEL}"),
        );
    }
    let name = required(&mut errors, "name", &level.name, 100);
    let anchor = required(&mut errors, "anchor", &level.anchor, 2000);
    let authority = required(&mut errors, "authority", &level.authority, 2000);
    errors.finish(Level {
        level: level.level,
        name,
        anchor,
        authority,
    })
}

/// A requirement: a positive skill id, a level 0–20, one of the three
/// criticalities, a weight from 0 to 100 with at most two decimals and an
/// optional evidence standard.
pub(crate) fn item(input: &ItemInput) -> Result<ItemInput> {
    let mut errors = Errors::default();
    positive(&mut errors, "skill_id", input.skill_id);
    if input.required_level < 0 {
        errors.push("required_level", "must be greater than or equal to 0");
    } else if input.required_level > MAX_LEVEL {
        errors.push(
            "required_level",
            format!("must be less than or equal to {MAX_LEVEL}"),
        );
    }
    let criticality = input.criticality.trim().to_owned();
    if !CRITICALITIES.contains(&criticality.as_str()) {
        errors.push("criticality", "is invalid");
    }
    if input.weight_percent < Decimal::ZERO {
        errors.push("weight_percent", "must be greater than or equal to 0");
    } else if input.weight_percent > Decimal::ONE_HUNDRED {
        errors.push("weight_percent", "must be less than or equal to 100");
    } else if input.weight_percent.round_dp(2) != input.weight_percent {
        errors.push("weight_percent", "has at most two decimals");
    }
    let evidence_standard = optional(
        &mut errors,
        "evidence_standard",
        input.evidence_standard.as_deref(),
        2000,
    );
    errors.finish(ItemInput {
        skill_id: input.skill_id,
        required_level: input.required_level,
        criticality,
        weight_percent: input.weight_percent.normalize(),
        mandatory: input.mandatory,
        evidence_standard,
    })
}

/// Bilimbi `validate_levels/1`: at least two levels, numbered contiguously
/// from 0, with distinct names (case-insensitively). `levels` is sorted by
/// level number, as every read returns it.
pub(crate) fn publishable_levels(levels: &[Level]) -> bool {
    let contiguous = levels
        .iter()
        .enumerate()
        .all(|(index, level)| i64::from(level.level) == index as i64);
    let mut names: Vec<String> = levels
        .iter()
        .map(|level| level.name.to_lowercase())
        .collect();
    names.sort();
    names.dedup();
    levels.len() >= 2 && contiguous && names.len() == levels.len()
}
