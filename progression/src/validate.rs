//! Bilimbi Progression.validate_rules!: bounded, company-owned criteria.
use crate::{DraftInput, PerformanceRule, ProgressionError as E, Result, Rules};
use jiff::civil::Date;

// PostgreSQL dates have no year zero. Canonical text also makes the released
// review comparisons use the same ordering as calendar dates.
fn date(value: &str) -> Option<Date> {
    let date = value.parse::<Date>().ok()?;
    (date.year() > 0 && date.to_string() == value).then_some(date)
}

pub(crate) fn draft(input: &DraftInput) -> Result<()> {
    if input.code.trim().is_empty()
        || input.code.trim().chars().count() > 100
        || input.name.trim().is_empty()
        || input.name.trim().chars().count() > 200
        || input.version <= 0
        || date(&input.effective_from).is_none()
    {
        return Err(E::InvalidPolicy);
    }
    if input.profile.is_none() && input.performance.is_none() {
        return Err(E::InvalidRules);
    }
    if let Some(pin) = &input.profile {
        if pin.id <= 0 || pin.version <= 0 {
            return Err(E::ProfileUnavailable);
        }
    }
    performance(input.performance.as_ref())
}
pub(crate) fn performance(rule: Option<&PerformanceRule>) -> Result<()> {
    let Some(rule) = rule else {
        return Ok(());
    };
    if rule.accepted_outcomes.is_empty()
        || rule.accepted_outcomes.len() > 100
        || rule
            .accepted_outcomes
            .iter()
            .any(|s| s.trim().is_empty() || s.len() > 200)
        || rule.periods.is_empty()
        || rule.periods.len() > 100
    {
        return Err(E::InvalidRules);
    }
    for period in &rule.periods {
        let first = date(&period.start).ok_or(E::InvalidRules)?;
        let last = date(&period.end).ok_or(E::InvalidRules)?;
        if first > last {
            return Err(E::InvalidRules);
        }
    }
    Ok(())
}
pub(crate) fn rules(rules: &Rules) -> Result<()> {
    if rules.competence.len() > 100 || (rules.competence.is_empty() && rules.performance.is_none())
    {
        return Err(E::InvalidRules);
    }
    let mut codes = std::collections::HashSet::new();
    for rule in &rules.competence {
        if rule.code.trim().is_empty()
            || rule.code.len() > 100
            || !codes.insert(&rule.code)
            || rule.profile_id <= 0
            || rule.profile_version <= 0
            || rule.skill_id <= 0
            || rule.required_level < 0
        {
            return Err(E::InvalidRules);
        }
    }
    performance(rules.performance.as_ref())
}
