//! Bilimbi Performance.build/7 and its bounded period/evidence validation.
use crate::{FieldError, PerformanceError as E, Result};
use bilibi_database::Error;
use jiff::{civil::Date, Timestamp};
use serde::Serialize;
use serde_json::Value;

pub(crate) fn clean(input: &impl Serialize, required: &[&'static str]) -> Result<Value> {
    let mut value = serde_json::to_value(input).map_err(|_| E::Failed(Error::Internal))?;
    let fields = value.as_object_mut().ok_or(E::Failed(Error::Internal))?;
    let mut errors = Vec::new();
    for (key, value) in fields.iter_mut() {
        if let Some(text) = value.as_str() {
            let text = text.trim().to_owned();
            if text.chars().count() > 8000 {
                // FieldError owns a static field key; known inputs are checked below.
                let field = match key.as_str() {
                    "rubric" => "rubric",
                    "effective_to" => "effective_to",
                    _ => required
                        .iter()
                        .copied()
                        .find(|f| *f == key)
                        .unwrap_or("values"),
                };
                errors.push(FieldError::new(
                    field,
                    "should be at most 8000 character(s)",
                ));
            }
            *value = if text.is_empty() && !required.contains(&key.as_str()) {
                Value::Null
            } else {
                Value::String(text)
            };
        }
    }
    for &field in required {
        if fields
            .get(field)
            .is_none_or(|v| v.is_null() || v.as_str().is_some_and(str::is_empty))
        {
            errors.push(FieldError::new(field, "can't be blank"));
        }
    }
    if errors.is_empty() {
        Ok(value)
    } else {
        Err(E::Invalid(errors))
    }
}
pub(crate) fn date(value: &str) -> Result<Date> {
    let day: Date = value.parse().map_err(|_| E::InvalidDates)?;
    if day.to_string() != value {
        return Err(E::InvalidDates);
    }
    Ok(day)
}
pub(crate) fn period(start: &str, end: &str) -> Result<()> {
    if date(start)? > date(end)? {
        Err(E::InvalidDates)
    } else {
        Ok(())
    }
}
pub(crate) fn cutoff(value: &str) -> Result<Timestamp> {
    if !value.ends_with('Z') && !value.ends_with("+00:00") {
        return Err(E::InvalidDates);
    }
    value.parse().map_err(|_| E::InvalidDates)
}
pub(crate) fn positive(field: &'static str, value: i64) -> Result<()> {
    if value > 0 {
        Ok(())
    } else {
        Err(E::Invalid(vec![FieldError::new(
            field,
            "must be greater than 0",
        )]))
    }
}
pub(crate) fn ids(values: &[i64]) -> Result<Vec<i64>> {
    if values.len() > 100 || values.iter().any(|id| *id <= 0) {
        return Err(E::InvalidEvidence);
    }
    let mut values = values.to_vec();
    values.sort_unstable();
    values.dedup();
    Ok(values)
}
pub(crate) fn reason(text: &str) -> Result<String> {
    if text.trim().is_empty() {
        return Err(E::ReasonRequired);
    }
    if text.trim().chars().count() > 8000 {
        return Err(E::Invalid(vec![FieldError::new(
            "reason",
            "should be at most 8000 character(s)",
        )]));
    }
    Ok(text.trim().to_owned())
}
pub(crate) fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_owned()
}
