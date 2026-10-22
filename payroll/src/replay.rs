//! Bilimbi Payroll.Replay: pure exact arithmetic over frozen data, no live reads.
use crate::{Contribution, Item, PayrollError, Result, Run};
use rust_decimal::Decimal;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const SCALE: u32 = 12;
/// Rust Decimal arithmetic can silently reduce scale on overflow. Work on
/// integer mantissas instead and require the entire twelve-place result to fit.
fn at_scale(value: Decimal, scale: u32) -> Result<i128> {
    let value = value.normalize();
    if value.scale() > scale {
        return Err(PayrollError::DecimalRange);
    }
    value
        .mantissa()
        .checked_mul(10_i128.pow(scale - value.scale()))
        .ok_or(PayrollError::DecimalRange)
}
fn fixed(mantissa: i128) -> Result<Decimal> {
    Decimal::try_from_i128_with_scale(mantissa, SCALE).map_err(|_| PayrollError::DecimalRange)
}
pub(crate) fn multiply(rate: &str, units: Decimal) -> Result<Decimal> {
    let rate = Decimal::from_str_exact(rate).map_err(|_| PayrollError::DecimalRange)?;
    let a = at_scale(rate, 6)?;
    let b = at_scale(units, 6)?;
    fixed(a.checked_mul(b).ok_or(PayrollError::DecimalRange)?)
}
pub(crate) fn add(a: Decimal, b: Decimal) -> Result<Decimal> {
    fixed(
        at_scale(a, SCALE)?
            .checked_add(at_scale(b, SCALE)?)
            .ok_or(PayrollError::DecimalRange)?,
    )
}
pub(crate) fn amount(text: &str) -> Result<Decimal> {
    let value = Decimal::from_str_exact(text).map_err(|_| PayrollError::DecimalRange)?;
    fixed(at_scale(value, SCALE)?)
}
pub(crate) fn calculate(run: &Run, inputs: &[Contribution]) -> Result<Value> {
    let items: Vec<Item> = crate::store::decode(run.snapshot["items"].clone())?;
    let mut lines = Vec::new();
    let mut totals: BTreeMap<i64, [Decimal; 3]> = BTreeMap::new();
    for input in inputs {
        let item = items
            .iter()
            .find(|i| i.id == input.item_id)
            .ok_or(PayrollError::IntakeUnavailable)?;
        let units =
            Decimal::from_str_exact(&input.units).map_err(|_| PayrollError::DecimalRange)?;
        let product = multiply(&item.amount, units)?;
        let mut line = serde_json::to_value(input).map_err(|_| bilibi_database::Error::Internal)?;
        line["item_code"] = json!(item.code);
        line["rate"] = json!(item.amount);
        line["amount"] = json!(product.to_string());
        lines.push(line);
        let index = match input.direction.as_str() {
            "earning" => 0,
            "deduction" => 1,
            "employer" => 2,
            _ => return Err(PayrollError::IntakeUnavailable),
        };
        let sums = totals
            .entry(input.employee_id)
            .or_insert([Decimal::ZERO; 3]);
        sums[index] = add(sums[index], product)?;
    }
    let totals=totals.into_iter().map(|(employee,sums)| {
        let net=add(sums[0],-sums[1])?;
        Ok(json!({"employee_id":employee,"earning":amount(&sums[0].to_string())?.to_string(),"deduction":amount(&sums[1].to_string())?.to_string(),"employer":amount(&sums[2].to_string())?.to_string(),"net":net.to_string()}))
    }).collect::<Result<Vec<_>>>()?;
    Ok(json!({"version":1,"lines":lines,"totals":totals}))
}
/// Explicitly sort at every object level, even if another crate enables
/// serde_json/preserve_order later. Arrays retain their documented order.
pub(crate) fn canonical(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let ordered: BTreeMap<_, _> = map.iter().collect();
            let pairs = ordered
                .into_iter()
                .map(|(k, v)| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(k).expect("string JSON"),
                        canonical(v)
                    )
                })
                .collect::<Vec<_>>();
            format!("{{{}}}", pairs.join(","))
        }
        Value::Array(values) => format!(
            "[{}]",
            values.iter().map(canonical).collect::<Vec<_>>().join(",")
        ),
        _ => value.to_string(),
    }
}
pub(crate) fn digest(value: &Value) -> String {
    Sha256::digest(canonical(value).as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
