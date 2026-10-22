//! Claims hand-off CSV through the shared download transport.
use crate::{Claims, ClaimsError, MODULE, REIMBURSE, SERVICE};
use bilibi_database::Error;
use bilibi_module_registry::{
    contributions::{Access, Download, DOWNLOADS},
    Contribution, Provenance, Service,
};
use bilibi_tenancy::Scope;
use bilibi_ui::download::{self, DownloadFuture, DownloadRequest, DownloadService, FileBody};
use std::sync::Arc;
const FILES: &str = "people/claims.downloads";
struct Downloads(Arc<Claims>);
impl download::Download<Scope, Error> for Downloads {
    fn serve<'a>(&'a self, request: DownloadRequest<'a, Scope>) -> DownloadFuture<'a, Error> {
        Box::pin(async move {
            let id = |name| {
                request
                    .path
                    .get(name)
                    .and_then(|s| s.parse::<i64>().ok())
                    .filter(|id| *id > 0)
                    .ok_or(Error::NotFound)
            };
            if request.operation != "handoff_export" {
                return Err(Error::NotFound);
            }
            let export = self
                .0
                .handoff_export(request.scope, id("company_id")?, id("id")?)
                .await
                .map_err(|e| match e {
                    ClaimsError::Unauthorized => Error::Forbidden,
                    ClaimsError::NotFound => Error::NotFound,
                    ClaimsError::Failed(e) => e,
                    _ => Error::Invalid,
                })?;
            Ok(FileBody::attachment(
                export.content.into_bytes(),
                "text/csv",
                &export.filename,
            ))
        })
    }
}
macro_rules! provenance {
    () => {
        Provenance {
            module: MODULE,
            source: file!(),
            line: line!(),
        }
    };
}
inventory::submit! {Contribution::new(DOWNLOADS,"people/claims.handoff_export",provenance!(),&Download{path:"/people/claims/handoff/{company_id}/{id}",service:FILES,operation:"handoff_export",access:Access::Capability(REIMBURSE)})}
pub(crate) const SERVICE_DEFINITION: Service = Service {
    id: FILES,
    factory: |services| {
        Ok(Arc::new(DownloadService::<Scope, Error>(Arc::new(
            Downloads(services.get(SERVICE)?),
        ))))
    },
};
