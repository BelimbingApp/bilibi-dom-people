//! Bilimbi Progression.validate_rules!: bounded, company-owned criteria.
use crate::{DraftInput, PerformanceRule, ProgressionError as E, Result, Rules};
use jiff::civil::Date;

pub(crate) fn draft(input: &DraftInput) -> Result<()> {
    if input.code.trim().is_empty()
        || input.code.trim().chars().count() > 100
        || input.name.trim().is_empty()
        || input.name.trim().chars().count() > 200
        || input.version <= 0
        || input.effective_from.parse::<Date>().is_err()
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
        let first = period.start.parse::<Date>().map_err(|_| E::InvalidRules)?;
        let last = period.end.parse::<Date>().map_err(|_| E::InvalidRules)?;
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
