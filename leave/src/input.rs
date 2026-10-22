//! Bounds shared by the facade's operations (Bilimbi
//! `Bilimbi.People.Leave.Input`). Private to the facade.
use rust_decimal::Decimal;

/// The most a quantity, an entitlement or a cap may be, in either direction.
const MAX_QUANTITY: Decimal = Decimal::from_parts(999_999, 0, 0, false, 2);

/// A quantity in hundredths of the type's unit, within `MAX_QUANTITY` in
/// either direction and with at most two decimals, normalized to two:
/// finer values are refused rather than rounded.
pub(crate) fn bounded_quantity(value: Decimal) -> Option<Decimal> {
    let mut rounded = value.round_dp(2);
    if rounded != value || value.abs() > MAX_QUANTITY {
        return None;
    }
    rounded.rescale(2);
    Some(rounded)
}

/// `bounded_quantity`, and not negative.
pub(crate) fn non_negative_quantity(value: Decimal) -> Option<Decimal> {
    bounded_quantity(value).filter(|value| !value.is_sign_negative() || value.is_zero())
}

/// A trimmed optional text of at most `max` characters; blank is `None`,
/// longer is refused.
pub(crate) fn optional_text(value: Option<&str>, max: usize) -> Result<Option<String>, ()> {
    match value.map(str::trim) {
        None | Some("") => Ok(None),
        Some(text) if text.chars().count() <= max => Ok(Some(text.to_owned())),
        Some(_) => Err(()),
    }
}
