//! Bilimbi Payroll.DocumentOwner: re-read approved frozen data before rendering.
//! Keep the owner ID and this concrete type's name stable: Artifacts persists both.
use crate::{
    store::{self, Kind},
    Document, Payroll, PayrollError, Result, MANAGE, VIEW,
};
use bilibi_artifacts::{
    self as artifacts, pdf, Operation, Owner, PdfOwner, Reference, RenderFuture,
};
use bilibi_database::{Db, Error};
use bilibi_party_ids::OrganisationId;
use bilibi_people_workforce::authorization;
use bilibi_tenancy::{ProofClient, ProofFuture, Scope};
use serde::Deserialize;
use serde_json::{json, Value};

pub(crate) struct DocumentOwner {
    db: Db,
}
impl Owner for DocumentOwner {
    fn owner_id(&self) -> &'static str {
        "people/payroll"
    }
    fn authorize<'a>(
        &'a self,
        client: &'a dyn ProofClient,
        scope: &'a Scope,
        company: OrganisationId,
        operation: Operation,
        reference: Option<&'a Reference>,
    ) -> ProofFuture<'a, ()> {
        Box::pin(async move {
            if scope.impersonator().is_some() {
                return Err(Error::Forbidden);
            }
            let capability = if operation == Operation::Read {
                VIEW
            } else {
                MANAGE
            };
            let summary =
                authorization::authorize_company_proof(client, scope, company.get(), capability)
                    .await
                    .map_err(|error| match error {
                        bilibi_people_workforce::WorkforceError::Failed(error) => error,
                        _ => Error::Forbidden,
                    })?;
            if summary.archived {
                return Err(Error::Forbidden);
            }
            if operation == Operation::Purge && reference.is_none() {
                return Ok(());
            }
            let run = reference.and_then(reference_run).ok_or(Error::NotFound)?;
            client.query_opt("SELECT id FROM people_payroll_decisions WHERE tenant_id=$1 AND company_id=$2 AND run_id=$3 AND outcome='approved'", &[&scope.tenant(), &company.get(), &run])
                .await?.ok_or(Error::NotFound)?;
            Ok(())
        })
    }
}
impl PdfOwner for DocumentOwner {
    fn render_pdf<'a>(
        &'a self,
        scope: &'a Scope,
        company: OrganisationId,
        reference: &'a Reference,
        data: &'a Value,
    ) -> RenderFuture<'a> {
        Box::pin(async move {
            let employee = match data.get("employee_id") {
                None | Some(Value::Null) => None,
                Some(v) => Some(
                    v.as_i64()
                        .filter(|id| *id > 0)
                        .ok_or(artifacts::Error::InvalidDocument)?,
                ),
            };
            // Ignore caller-supplied currency, digest, lines and totals. Only
            // the employee selection is input; all printed facts are re-read.
            let frozen = self
                .frozen(scope, company, reference, employee)
                .await
                .map_err(artifact_error)?;
            let mut blocks = vec![
                pdf::Block::Text(format!("Currency: {}", frozen.currency)),
                pdf::Block::Text(format!("Calculation: {}", frozen.digest)),
            ];
            for line in frozen.result.lines {
                blocks.push(pdf::Block::Text(format!(
                    "Employee {} / item {} / {}: {} x {} = {}",
                    line.employee_id,
                    line.item_id,
                    line.direction,
                    line.units,
                    line.rate,
                    line.amount
                )));
            }
            for total in frozen.result.totals {
                blocks.push(pdf::Block::Text(format!(
                    "Employee {}: earnings {}, deductions {}, net {}, employer {}",
                    total.employee_id, total.earning, total.deduction, total.net, total.employer
                )));
            }
            pdf::render(&pdf::Document {
                title: format!("Payroll {} - run {}", reference.kind, reference.subject),
                metadata: pdf::Metadata::default(),
                blocks,
            })
        })
    }
}
fn reference_run(reference: &Reference) -> Option<i64> {
    if !matches!(reference.kind.as_str(), "report" | "payslip") {
        return None;
    }
    reference.subject.parse::<i64>().ok().filter(|id| *id > 0)
}
fn artifact_error(error: PayrollError) -> artifacts::Error {
    match error {
        PayrollError::NotFound => artifacts::Error::NotFound,
        PayrollError::InvalidDocument => artifacts::Error::InvalidDocument,
        PayrollError::Failed(error) => artifacts::Error::Database(error),
        _ => artifacts::Error::Database(Error::Forbidden),
    }
}
#[derive(Deserialize)]
struct Line {
    employee_id: i64,
    item_id: i64,
    direction: String,
    units: String,
    rate: String,
    amount: String,
}
#[derive(Deserialize)]
struct Total {
    employee_id: i64,
    earning: String,
    deduction: String,
    employer: String,
    net: String,
}
#[derive(Deserialize)]
struct FrozenResult {
    lines: Vec<Line>,
    totals: Vec<Total>,
}
struct Frozen {
    currency: String,
    digest: String,
    result: FrozenResult,
}
impl DocumentOwner {
    async fn frozen(
        &self,
        scope: &Scope,
        company: OrganisationId,
        reference: &Reference,
        employee: Option<i64>,
    ) -> Result<Frozen> {
        let c = self.db.get().await.map_err(Error::from)?;
        // As in Bilimbi document_data, generating also needs view permission.
        self.authorize(&**c, scope, company, Operation::Read, Some(reference))
            .await?;
        let run = reference_run(reference).ok_or(PayrollError::NotFound)?;
        // One statement reads a consistent view of the immutable approved result.
        let row = c.query_opt("SELECT r.currency,c.digest,c.snapshot->'result' FROM people_payroll_setup_snapshots r JOIN people_payroll_calculations c ON c.tenant_id=r.tenant_id AND c.company_id=r.company_id AND c.run_id=r.id JOIN people_payroll_decisions d ON d.tenant_id=r.tenant_id AND d.company_id=r.company_id AND d.run_id=r.id WHERE r.tenant_id=$1 AND r.company_id=$2 AND r.id=$3 AND d.outcome='approved'", &[&scope.tenant(), &company.get(), &run]).await?.ok_or(PayrollError::NotFound)?;
        let mut result: FrozenResult =
            serde_json::from_value(row.get(2)).map_err(|_| PayrollError::InvalidDocument)?;
        match (reference.kind.as_str(), employee) {
            ("report", None) => {}
            ("payslip", Some(id)) if result.totals.iter().any(|t| t.employee_id == id) => {
                result.lines.retain(|l| l.employee_id == id);
                result.totals.retain(|t| t.employee_id == id);
            }
            _ => return Err(PayrollError::InvalidDocument),
        }
        Ok(Frozen {
            currency: row.get(0),
            digest: row.get(1),
            result,
        })
    }
}
impl From<artifacts::Error> for PayrollError {
    fn from(error: artifacts::Error) -> Self {
        match error {
            artifacts::Error::Database(error) => error.into(),
            artifacts::Error::Sql(error) => error.into(),
            artifacts::Error::NotFound => Self::NotFound,
            artifacts::Error::InvalidDocument | artifacts::Error::InvalidPdf => {
                Self::InvalidDocument
            }
            artifacts::Error::Policy => Self::ArtifactPolicy,
            other => Self::ArtifactFailed(other),
        }
    }
}
impl Payroll {
    /// Artifacts reserves and publishes outside the business transaction. A
    /// failed provenance link leaves tracked bytes for expiry/recovery.
    pub async fn generate_document(
        &self,
        scope: &Scope,
        company: i64,
        run: i64,
        kind: &str,
        employee: Option<i64>,
    ) -> Result<Document> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**c, scope, company, MANAGE).await?;
        let company_id = OrganisationId::new(company).map_err(|_| PayrollError::NotFound)?;
        let owner = DocumentOwner {
            db: self.db.clone(),
        };
        let reference = Reference {
            subject: run.to_string(),
            kind: kind.into(),
        };
        // Refuse invalid selections before reserving or writing any bytes.
        owner
            .frozen(scope, company_id, &reference, employee)
            .await?;
        let metadata = self
            .artifacts
            .for_owner(scope, company_id, &owner)
            .generate_pdf(&mut c, reference, &json!({"employee_id": employee}))
            .await?;
        let mut tx = self
            .begin(
                &mut c,
                scope,
                company,
                MANAGE,
                "people.payroll.generate_document",
            )
            .await?;
        store::run_for_update(tx.sql(), scope, company, run).await?;
        if tx.sql().query_opt("SELECT id FROM people_payroll_decisions WHERE tenant_id=$1 AND company_id=$2 AND run_id=$3 AND outcome='approved'", &[&scope.tenant(), &company, &run]).await?.is_none() { return Err(PayrollError::NotFound); }
        let document = store::insert(
            &mut tx,
            scope,
            company,
            Kind::Document,
            json!({"run_id": run,"employee_id": employee,"artifact_id": metadata.id,"kind": kind}),
        )
        .await?;
        tx.commit().await?;
        Ok(document)
    }
    /// Every read re-proves current authority and the approved run. Artifacts
    /// also checks expiry, lifecycle, private file mode, length and SHA-256.
    pub async fn read_document(
        &self,
        scope: &Scope,
        company: i64,
        artifact_id: &str,
    ) -> Result<artifacts::Document> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**c, scope, company, VIEW).await?;
        Ok(self
            .artifacts
            .for_owner(
                scope,
                OrganisationId::new(company).map_err(|_| PayrollError::NotFound)?,
                &DocumentOwner {
                    db: self.db.clone(),
                },
            )
            .read(&mut c, artifact_id)
            .await?)
    }
    pub async fn purge(&self, scope: &Scope, company: i64) -> Result<artifacts::Recovery> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**c, scope, company, MANAGE).await?;
        Ok(self
            .artifacts
            .for_owner(
                scope,
                OrganisationId::new(company).map_err(|_| PayrollError::NotFound)?,
                &DocumentOwner {
                    db: self.db.clone(),
                },
            )
            .purge_expired(&mut c)
            .await?)
    }
    pub async fn purge_holds(
        &self,
        scope: &Scope,
        company: i64,
    ) -> Result<Vec<artifacts::PurgeHold>> {
        let c = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**c, scope, company, MANAGE).await?;
        Ok(self
            .artifacts
            .for_owner(
                scope,
                OrganisationId::new(company).map_err(|_| PayrollError::NotFound)?,
                &DocumentOwner {
                    db: self.db.clone(),
                },
            )
            .list_purge_holds(&c)
            .await?)
    }
    pub async fn retry_purge(&self, scope: &Scope, company: i64, artifact_id: &str) -> Result<()> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**c, scope, company, MANAGE).await?;
        Ok(self
            .artifacts
            .for_owner(
                scope,
                OrganisationId::new(company).map_err(|_| PayrollError::NotFound)?,
                &DocumentOwner {
                    db: self.db.clone(),
                },
            )
            .retry_purge(&mut c, artifact_id)
            .await?)
    }
}
