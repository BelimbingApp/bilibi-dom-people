//! Bounded native directory pages (Bilimbi NativePeopleAdapter's immutable
//! employee key and per-pass watermark). SQL stays behind the Workforce facade.
use crate::{employments, live_company, Employee, ReadResult, Result, Workforce, WorkforceError};
use bilibi_database::Error;
use bilibi_tenancy::Scope;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use tokio_postgres::GenericClient;

pub const EMPLOYEE_PAGE_SIZE_MAX: usize = 1_000;

/// `None` starts a pass. Continue with its next cursor and the same page size.
#[derive(Clone, Debug)]
pub struct EmployeeScan {
    pub cursor: Option<String>,
    pub page_size: usize,
}
impl Default for EmployeeScan {
    fn default() -> Self {
        Self {
            cursor: None,
            page_size: 250,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct EmployeePage {
    pub employees: Vec<Employee>,
    pub next_cursor: Option<String>,
    pub high_water_id: i64,
    /// RFC 3339 UTC observation time, fixed for every page of this pass.
    pub watermark: String,
    /// A complete enumeration when all pages have been read, not a change feed.
    pub snapshot: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    v: u8,
    tenant: i64,
    company: i64,
    size: usize,
    statuses: Vec<String>,
    after: i64,
    high_water: i64,
    watermark: String,
}

impl Workforce {
    pub async fn employee_page(
        &self,
        scope: &Scope,
        company_id: i64,
        scan: &EmployeeScan,
    ) -> Result<ReadResult<EmployeePage>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.employee_page_on(&**client, scope, company_id, scan)
            .await
    }

    /// The same page on a caller's client, including Core names and contacts.
    pub async fn employee_page_on(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company_id: i64,
        scan: &EmployeeScan,
    ) -> Result<ReadResult<EmployeePage>> {
        let company = live_company(client, scope, company_id).await?;
        if scan.page_size == 0 || scan.page_size > EMPLOYEE_PAGE_SIZE_MAX {
            return Err(WorkforceError::InvalidOptions);
        }
        let statuses = self
            .company_working_statuses(client, scope, &company)
            .await?;
        let mut cursor = if let Some(encoded) = &scan.cursor {
            if encoded.len() > 2048 {
                return Err(WorkforceError::InvalidCursor);
            }
            let c: Cursor =
                serde_json::from_str(encoded).map_err(|_| WorkforceError::InvalidCursor)?;
            if c.v != 1
                || c.tenant != scope.tenant()
                || c.company != company_id
                || c.size != scan.page_size
                || c.statuses != statuses
                || c.after <= 0
                || c.high_water < c.after
                || c.watermark.parse::<Timestamp>().is_err()
            {
                return Err(WorkforceError::InvalidCursor);
            }
            c
        } else {
            let high_water = client.query_one(
                "SELECT COALESCE(max(id),0) FROM people_workforce_employments WHERE tenant_id=$1 AND company_id=$2",
                &[&scope.tenant(), &company_id],
            ).await.map_err(Error::from)?.get(0);
            Cursor {
                v: 1,
                tenant: scope.tenant(),
                company: company_id,
                size: scan.page_size,
                statuses: statuses.clone(),
                after: 0,
                high_water,
                watermark: Timestamp::now().to_string(),
            }
        };
        let mut rows = employments::scan(
            client,
            scope.tenant(),
            company_id,
            &statuses,
            cursor.after,
            cursor.high_water,
            (scan.page_size + 1) as i64,
        )
        .await?;
        let more = rows.len() > scan.page_size;
        rows.truncate(scan.page_size);
        let next_cursor = if more {
            cursor.after = rows.last().ok_or(Error::Internal)?.employment.id;
            Some(serde_json::to_string(&cursor).map_err(|_| Error::Internal)?)
        } else {
            None
        };
        let employees = self
            .project(client, scope, company_id, &statuses, rows)
            .await?;
        Ok(ReadResult::current(EmployeePage {
            employees,
            next_cursor,
            high_water_id: cursor.high_water,
            watermark: cursor.watermark,
            snapshot: true,
        }))
    }
}
