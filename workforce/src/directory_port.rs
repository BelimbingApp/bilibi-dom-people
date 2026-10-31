//! People connector's answer-now seam. Mirrors the native read adapter in
//! Bilimbi, using the Workforce facade and Organisation's optional port.
use crate::{EmployeeScan, PositionScan, Workforce, WorkforceError};
use bilibi_database::Error;
use bilibi_ports::{answer, on_client, request, PortClient, PortFuture, PortProvider, Value};
use bilibi_tenancy::Scope;
use jiff::{civil::Date, Timestamp};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::{Arc, RwLock};

pub const DIRECTORY_PORT: &str = "people/workforce.directory";
static FACADE: RwLock<Option<Arc<Workforce>>> = RwLock::new(None);
pub(crate) fn install(facade: Arc<Workforce>) {
    *FACADE.write().unwrap_or_else(|p| p.into_inner()) = Some(facade);
}

#[derive(Deserialize)]
#[serde(tag = "read", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Company {
        company_id: i64,
    },
    Employees {
        company_id: i64,
        #[serde(default)]
        cursor: Option<String>,
        #[serde(default = "employee_size")]
        page_size: usize,
    },
    Positions {
        company_id: i64,
        as_of: String,
        #[serde(default)]
        cursor: Option<String>,
        #[serde(default = "position_size")]
        page_size: usize,
    },
}
fn employee_size() -> usize {
    250
}
fn position_size() -> usize {
    50
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PositionCursor {
    tenant: i64,
    company: i64,
    as_of: String,
    size: usize,
    watermark: String,
    inner: String,
}

pub(crate) struct DirectoryPort;
impl PortProvider for DirectoryPort {
    fn call<'a>(&'a self, client: PortClient<'a>, scope: &'a Scope, body: Value) -> PortFuture<'a> {
        Box::pin(async move {
            let request: Request = request(body)?;
            let facade = FACADE
                .read()
                .unwrap_or_else(|p| p.into_inner())
                .clone()
                .ok_or(Error::Internal)?;
            match request {
                Request::Company { company_id } => {
                    let read = on_client!(client, |c| facade.company(c, scope, company_id).await)
                        .map_err(refused)?;
                    Ok(json!({"value": read.value, "freshness": "current"}))
                }
                Request::Employees {
                    company_id,
                    cursor,
                    page_size,
                } => {
                    let scan = EmployeeScan { cursor, page_size };
                    let read = on_client!(client, |c| facade
                        .employee_page(c, scope, company_id, &scan)
                        .await)
                    .map_err(refused)?;
                    let mut page = answer(&read.value.ok_or(Error::Internal)?)?;
                    page["freshness"] = json!("current");
                    Ok(page)
                }
                Request::Positions {
                    company_id,
                    as_of,
                    cursor,
                    page_size,
                } => {
                    // The same live-company proof as every Workforce read,
                    // before even deciding whether Organisation is mounted.
                    on_client!(client, |c| facade.company(c, scope, company_id).await)
                        .map_err(refused)?;
                    let day: Date = as_of.parse().map_err(|_| Error::Invalid)?;
                    let (inner, watermark) = if let Some(cursor) = cursor {
                        if cursor.len() > 8192 {
                            return Err(Error::Invalid);
                        }
                        let c: PositionCursor =
                            serde_json::from_str(&cursor).map_err(|_| Error::Invalid)?;
                        if c.tenant != scope.tenant()
                            || c.company != company_id
                            || c.as_of != as_of
                            || c.size != page_size
                            || c.watermark.parse::<Timestamp>().is_err()
                        {
                            return Err(Error::Invalid);
                        }
                        (Some(c.inner), c.watermark)
                    } else {
                        (None, Timestamp::now().to_string())
                    };
                    let scan = PositionScan {
                        as_of: day,
                        cursor: inner,
                        page_size,
                    };
                    match crate::positions::scan(&facade.ports, client, scope, company_id, &scan)
                        .await
                    {
                        Ok(mut page) => {
                            page.next_cursor = page
                                .next_cursor
                                .map(|inner| {
                                    serde_json::to_string(&PositionCursor {
                                        tenant: scope.tenant(),
                                        company: company_id,
                                        as_of,
                                        size: page_size,
                                        watermark: watermark.clone(),
                                        inner,
                                    })
                                })
                                .transpose()
                                .map_err(|_| Error::Internal)?;
                            let mut body = answer(&page)?;
                            body["freshness"] = json!("current");
                            body["snapshot"] = json!(true);
                            body["watermark"] = json!(watermark);
                            Ok(body)
                        }
                        Err(WorkforceError::Unavailable) => {
                            Ok(json!({"positions": [], "freshness": "unavailable",
                            "reason": "organisation_unavailable", "next_cursor": null, "high_water_id": null,
                            "snapshot": false, "watermark": watermark}))
                        }
                        Err(e) => Err(refused(e)),
                    }
                }
            }
        })
    }
}
fn refused(error: WorkforceError) -> Error {
    match error {
        WorkforceError::Unauthorized => Error::Forbidden,
        WorkforceError::NotFound | WorkforceError::CompanyArchived => Error::NotFound,
        WorkforceError::Failed(error) => error,
        _ => Error::Invalid,
    }
}
