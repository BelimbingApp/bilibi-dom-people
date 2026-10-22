//! Bilimbi Training.Web.EvidenceController; shared delivery headers are in FileBody.
use crate::Training;
use bilibi_database::Error;
use bilibi_tenancy::Scope;
use bilibi_ui::download::{Download, DownloadFuture, DownloadRequest, FileBody};
use std::sync::Arc;
pub(crate) const SERVICE: &str = "people/training.downloads";
pub(crate) struct EvidenceDownload {
    pub training: Arc<Training>,
}
impl Download<Scope, Error> for EvidenceDownload {
    fn serve<'a>(&'a self, request: DownloadRequest<'a, Scope>) -> DownloadFuture<'a, Error> {
        Box::pin(async move {
            if request.operation != "evidence" {
                return Err(Error::NotFound);
            }
            let company_id = request
                .path
                .get("company_id")
                .and_then(|s| s.parse::<i64>().ok())
                .filter(|id| *id > 0)
                .ok_or(Error::NotFound)?;
            let artifact_id = request.path.get("id").ok_or(Error::NotFound)?;
            let document = self
                .training
                .read_evidence(request.scope, company_id, artifact_id)
                .await
                .map_err(|_| Error::NotFound)?;
            Ok(FileBody::attachment(
                document.bytes,
                "application/pdf",
                "evidence.pdf",
            ))
        })
    }
}
