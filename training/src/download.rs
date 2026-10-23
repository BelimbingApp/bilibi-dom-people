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
            let company_id = request
                .path
                .get("company_id")
                .and_then(|s| s.parse::<i64>().ok())
                .filter(|id| *id > 0)
                .ok_or(Error::NotFound)?;
            let artifact_id = request.path.get("id").ok_or(Error::NotFound)?;
            let passport = match request.operation {
                "my_document" | "my_evidence" => Some(crate::PassportAudience::SelfService),
                "team_document" | "team_evidence" => Some(crate::PassportAudience::Team),
                "evidence" => None,
                _ => return Err(Error::NotFound),
            };
            if let Some(audience) = passport {
                let client = self.training.db.get().await?;
                bilibi_people_workforce::authorization::authorize(
                    &**client,
                    request.scope,
                    company_id,
                    audience.capability(),
                )
                .await
                .map_err(|_| Error::NotFound)?;
            }
            let (document, name) = match request.operation {
                "my_document" | "team_document" => (
                    self.training
                        .read_passport(request.scope, company_id, artifact_id)
                        .await,
                    "passport.pdf",
                ),
                "my_evidence" | "team_evidence" => (
                    self.training
                        .read_passport_evidence(request.scope, company_id, artifact_id)
                        .await,
                    "evidence.pdf",
                ),
                _ => (
                    self.training
                        .read_evidence(request.scope, company_id, artifact_id)
                        .await,
                    "evidence.pdf",
                ),
            };
            let document = document.map_err(|_| Error::NotFound)?;
            Ok(FileBody::attachment(
                document.bytes,
                "application/pdf",
                name,
            ))
        })
    }
}
