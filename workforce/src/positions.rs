//! The consumer side of the `people/organisation.positions` port (Bilimbi
//! `positions/4`, which found Organisation's registered position reader at
//! run time): Workforce asks Organisation, when it is mounted, for one page
//! of a company's positions, and answers `Unavailable` otherwise. The
//! request and answer below are the documented shape (`README.md`,
//! "Positions"); Organisation keeps its own serde types for the same JSON.
use crate::{Reference, Result, WorkforceError};
use bilibi_database::Error;
use bilibi_ports::{PortClient, PortError, Ports};
use bilibi_tenancy::Scope;
use jiff::civil::Date;
use serde::{Deserialize, Serialize};

/// The port Organisation answers.
pub const POSITIONS_PORT: &str = "people/organisation.positions";
/// The default page of a scan (Bilimbi `@page_size`).
pub const POSITION_PAGE_SIZE: usize = 50;
/// The largest page a scan may ask for (Bilimbi `@max_page_size`).
pub const POSITION_PAGE_SIZE_MAX: usize = 100;

/// One page request: the day the projections are read for, the cursor of
/// the previous page (`None` starts a scan and fixes its high-water mark)
/// and the page size, 1 to [`POSITION_PAGE_SIZE_MAX`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PositionScan {
    pub as_of: Date,
    pub cursor: Option<String>,
    pub page_size: usize,
}
impl PositionScan {
    /// A scan from the start, read as of `as_of`, at the default page size.
    pub fn from_start(as_of: Date) -> Self {
        Self {
            as_of,
            cursor: None,
            page_size: POSITION_PAGE_SIZE,
        }
    }
    /// The same scan continued from a page's `next_cursor`.
    pub fn continued(&self, cursor: String) -> Self {
        Self {
            as_of: self.as_of,
            cursor: Some(cursor),
            page_size: self.page_size,
        }
    }
}

/// What crosses the port: the live company Workforce already checked, the
/// day as ISO text, the opaque cursor and the page size.
#[derive(Serialize)]
struct Request<'a> {
    company_id: i64,
    as_of: String,
    cursor: Option<&'a str>,
    page_size: usize,
}

/// One page of positions with the cursor of the next page (`None` after the
/// last) and the scan's high-water mark: the last position id the sequence
/// had issued when the scan started, the same on every page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PositionPage {
    pub positions: Vec<Position>,
    pub next_cursor: Option<String>,
    pub high_water_id: i64,
}

/// The public native position projection Organisation supplies (Bilimbi
/// `Bilimbi.People.Workforce.Position`): references for itself, its company
/// and its parent, the immutable code, the title and version in effect on
/// the day, the working holders (`assignments_incomplete` when the page's
/// holders were cut short) and whether no substantive holder occupies it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub reference: Reference,
    pub company_reference: Reference,
    pub platform_company_id: i64,
    pub workforce_company_id: i64,
    pub code: String,
    #[serde(default)]
    pub parent_reference: Option<Reference>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub version: Option<i64>,
    #[serde(default)]
    pub assignments: Vec<PositionAssignment>,
    #[serde(default)]
    pub assignments_incomplete: bool,
    pub vacant: bool,
}

/// One working holder of a position on the day: the assignment and the
/// employee it names, and its kind (`substantive` or `acting`, as
/// Organisation records it).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PositionAssignment {
    pub reference: Reference,
    pub employee_reference: Reference,
    pub kind: String,
}

/// Asks the port for one page, in Workforce's refusal words.
pub(crate) async fn scan<'a>(
    ports: &'a Ports,
    client: impl Into<PortClient<'a>>,
    scope: &'a Scope,
    company_id: i64,
    scan: &PositionScan,
) -> Result<PositionPage> {
    if scan.page_size == 0 || scan.page_size > POSITION_PAGE_SIZE_MAX {
        return Err(WorkforceError::InvalidOptions);
    }
    let request = Request {
        company_id,
        as_of: scan.as_of.to_string(),
        cursor: scan.cursor.as_deref(),
        page_size: scan.page_size,
    };
    ports
        .call::<_, PositionPage>(client, scope, POSITIONS_PORT, &request)
        .await
        .map_err(|error| match error {
            PortError::Unavailable(_) => WorkforceError::Unavailable,
            // Workforce validated the options; what the provider still
            // refuses as invalid is the cursor.
            PortError::Refused(Error::Invalid) => WorkforceError::InvalidCursor,
            PortError::Refused(error) => error.into(),
            PortError::Shape { .. } => WorkforceError::Failed(Error::Internal),
        })
}
