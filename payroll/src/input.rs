//! Bilimbi Payroll changeset rules and inclusive intervals.
use crate::{FieldError, PayrollError, Result};
use jiff::civil::Date;
pub(crate) fn text(value: &str, field: &'static str, max: usize) -> Result<String> {
    if value.trim().is_empty() {
        return Err(PayrollError::Invalid(vec![FieldError::new(
            field,
            "can't be blank",
        )]));
    }
    if value.chars().count() > max {
        return Err(PayrollError::Invalid(vec![FieldError::new(
            field,
            format!("should be at most {max} character(s)"),
        )]));
    }
    Ok(value.to_owned())
}
pub(crate) fn dates(from: Date, to: Option<Date>) -> Result<()> {
    if to.is_some_and(|to| to < from) {
        Err(PayrollError::Invalid(vec![FieldError::new(
            "effective_to",
            "must follow start",
        )]))
    } else {
        Ok(())
    }
}
pub(crate) fn overlaps(
    from: &str,
    to: Option<&str>,
    other_from: &str,
    other_to: Option<&str>,
) -> bool {
    to.is_none_or(|to| other_from <= to) && other_to.is_none_or(|to| from <= to)
}
pub(crate) fn covers(
    from: &str,
    to: Option<&str>,
    child_from: &str,
    child_to: Option<&str>,
) -> bool {
    from <= child_from && to.is_none_or(|to| child_to.is_some_and(|end| end <= to))
}
