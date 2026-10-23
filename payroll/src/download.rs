//! Bilimbi Payroll.Web.DocumentController; FileBody owns private delivery headers.
use crate::Payroll;
use bilibi_database::Error;
use bilibi_tenancy::Scope;
use bilibi_ui::download::{Download, DownloadFuture, DownloadRequest, FileBody};
use std::sync::Arc;

pub(crate) const SERVICE: &str = "people/payroll.downloads";
pub(crate) struct DocumentDownload {
    pub payroll: Arc<Payroll>,
}
impl Download<Scope, Error> for DocumentDownload {
    fn serve<'a>(&'a self, request: DownloadRequest<'a, Scope>) -> DownloadFuture<'a, Error> {
        Box::pin(async move {
            if request.operation != "document" {
                return Err(Error::NotFound);
            }
            let company = request
                .query
                .get("company_id")
                .and_then(|s| s.parse::<i64>().ok())
                .filter(|id| *id > 0)
                .ok_or(Error::NotFound)?;
            let id = request.path.get("id").ok_or(Error::NotFound)?;
            let document = self
                .payroll
                .read_document(request.scope, company, id)
                .await
                .map_err(|_| Error::NotFound)?;
            Ok(FileBody::attachment(
                document.bytes,
                "application/pdf",
                &format!("payroll-{}.pdf", document.metadata.id),
            ))
        })
    }
}
