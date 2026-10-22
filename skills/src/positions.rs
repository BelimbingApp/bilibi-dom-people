//! The positions a profile may target and the position an employee holds,
//! read through Workforce's position scan (Bilimbi `Workforce.positions/4`,
//! served by Organisation through the `people/organisation.positions` port
//! when it is mounted). Private to the facade.
//!
//! **Seam (PK1).** Workforce's `positions` scan lands with the employment
//! slice (port plan row PW1), which this slice does not depend on, so this
//! module answers today exactly what Bilimbi answers while no Organisation
//! read is registered: the Workforce read is unavailable, so a profile can
//! target only the company ([`options`] is `PositionUnavailable`) and no
//! employee holds a position ([`position_of`] is `None`). PK4 (the catalog
//! page, which offers the position targets) replaces the bodies below with
//! the Workforce scan (pages of 100, at most 20 pages, `require_current`
//! on each page, a substantive assignment naming the employee), keeping the
//! signatures. Nothing else in the facade changes.
use crate::{PositionOption, Result, SkillsError};
use bilibi_people_workforce::Workforce;
use bilibi_tenancy::Scope;
use jiff::civil::Date;

/// Bilimbi `position_options/2`: every position the Workforce scan exposes
/// for the company today, up to 2,000, or `PositionUnavailable` while the
/// scan is unavailable.
pub(crate) async fn options(
    _workforce: &Workforce,
    _scope: &Scope,
    _company_id: i64,
) -> Result<Vec<PositionOption>> {
    Err(SkillsError::PositionUnavailable)
}

/// Bilimbi `position_of/4`: the position the employee substantively holds
/// on the day, or `None` when they hold none or the scan is unavailable.
pub(crate) async fn position_of(
    _workforce: &Workforce,
    _scope: &Scope,
    _company_id: i64,
    _employee_id: i64,
    _as_of: Date,
) -> Result<Option<i64>> {
    Ok(None)
}
