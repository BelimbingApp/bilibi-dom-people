//! Profile targets and substantive holders through Workforce's position
//! scan (Bilimbi `Skills.position_options/2` and `position_of/4`). Private
//! to the facade; Organisation supplies the optional Workforce port.
use crate::{PositionOption, Result, SkillsError};
use bilibi_people_workforce::{PositionScan, Workforce, WorkforceError};
use bilibi_tenancy::Scope;
use jiff::civil::Date;

// Bilimbi's bounds: at most 2,000 positions, in pages of 100.
const PAGE_SIZE: usize = 100;
const MAX_PAGES: usize = 20;

/// Every exposed position today, or `PositionUnavailable` when the scan
/// fails or is not current. Keep the cursor and its high-water mark across
/// pages so concurrent additions cannot extend the scan indefinitely.
pub(crate) async fn options(
    workforce: &Workforce,
    scope: &Scope,
    company_id: i64,
) -> Result<Vec<PositionOption>> {
    let today = jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date();
    let mut scan = PositionScan {
        page_size: PAGE_SIZE,
        ..PositionScan::from_start(today)
    };
    let mut options = Vec::new();
    for _ in 0..MAX_PAGES {
        let page = workforce
            .positions(scope, company_id, &scan)
            .await
            .map_err(|_| SkillsError::PositionUnavailable)?
            .require_current()
            .map_err(|_| SkillsError::PositionUnavailable)?;
        for position in page.positions {
            options.push(PositionOption {
                id: position
                    .reference
                    .stable_id
                    .parse()
                    .map_err(|_| SkillsError::PositionUnavailable)?,
                code: position.code,
                title: position.title.unwrap_or_default(),
            });
        }
        let Some(cursor) = page.next_cursor else {
            break;
        };
        scan = scan.continued(cursor);
    }
    Ok(options)
}

/// The first position the employee substantively holds on the requested
/// day. An absent Organisation is `None`; other read failures are refused.
pub(crate) async fn position_of(
    workforce: &Workforce,
    scope: &Scope,
    company_id: i64,
    employee_id: i64,
    as_of: Date,
) -> Result<Option<i64>> {
    let target = employee_id.to_string();
    let mut scan = PositionScan {
        page_size: PAGE_SIZE,
        ..PositionScan::from_start(as_of)
    };
    for _ in 0..MAX_PAGES {
        let page = match workforce.positions(scope, company_id, &scan).await {
            Ok(read) => read.require_current()?,
            Err(WorkforceError::Unavailable) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if let Some(held) = page.positions.iter().find(|position| {
            position.assignments.iter().any(|assignment| {
                assignment.kind == "substantive"
                    && assignment.employee_reference.stable_id == target
            })
        }) {
            return held
                .reference
                .stable_id
                .parse()
                .map(Some)
                .map_err(|_| SkillsError::PositionUnavailable);
        }
        let Some(cursor) = page.next_cursor else {
            break;
        };
        scan = scan.continued(cursor);
    }
    Ok(None)
}
