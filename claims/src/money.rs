//! Two-decimal amounts in a `numeric(14, 2)` column (Bilimbi
//! `Bilimbi.People.Claims.Money`): twelve integer digits and two minor-unit
//! digits, in whatever currency the row names.
use crate::FieldError;
use rust_decimal::Decimal;

/// The lower bound an amount must respect.
#[derive(Clone, Copy)]
pub(crate) enum Min {
    /// Greater than zero (a limit).
    Positive,
    /// Not negative (a threshold).
    Zero,
}

/// The first value a `numeric(14, 2)` column cannot hold.
fn ceiling() -> Decimal {
    Decimal::from(1_000_000_000_000_i64)
}

/// Validates one optional amount in Bilimbi's words, appending to `errors`.
pub(crate) fn validate_amount(
    errors: &mut Vec<FieldError>,
    field: &'static str,
    amount: Option<Decimal>,
    min: Min,
) {
    let Some(amount) = amount else {
        return;
    };
    if amount.normalize().scale() > 2 {
        errors.push(FieldError::new(
            field,
            "must have at most two decimal places",
        ));
    } else if matches!(min, Min::Positive) && amount <= Decimal::ZERO {
        errors.push(FieldError::new(field, "must be greater than zero"));
    } else if matches!(min, Min::Zero) && amount < Decimal::ZERO {
        errors.push(FieldError::new(field, "must not be negative"));
    } else if amount >= ceiling() {
        errors.push(FieldError::new(field, "is too large"));
    }
}

/// An amount as audit records it: exact text, never a float.
pub(crate) fn text(amount: Option<Decimal>) -> Option<String> {
    amount.map(|amount| amount.to_string())
}
