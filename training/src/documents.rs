//! Bilimbi Training.DocumentOwner and Participation's evidence operations.
//! Keep this concrete owner type stable: Artifacts persists its Rust name.
use crate::{
    participation, writable, Evidence, Result, Training, TrainingError, EVIDENCE_MANAGE,
    RECORDS_VIEW, RETENTION_MANAGE,
};
use bilibi_artifacts::{self as artifacts, Operation, Owner, Reference};
use bilibi_database::{Change, Error};
use bilibi_party_ids::OrganisationId;
use bilibi_people_workforce::authorization;
use bilibi_tenancy::{ProofClient, ProofFuture, Scope};
use serde_json::json;

pub(crate) struct DocumentOwner;
impl Owner for DocumentOwner {
    fn owner_id(&self) -> &'static str {
        "people/training"
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
            let capability = match operation {
                Operation::Read => RECORDS_VIEW,
                Operation::Create => EVIDENCE_MANAGE,
                Operation::Delete | Operation::Purge => RETENTION_MANAGE,
                Operation::Retain => return Err(Error::Forbidden),
            };
            let company =
                authorization::authorize_company_proof(client, scope, company.get(), capability)
                    .await
                    .map_err(|error| match error {
                        bilibi_people_workforce::WorkforceError::Failed(error) => error,
                        _ => Error::Forbidden,
                    })?;
            if company.archived {
                return Err(Error::Forbidden);
            }
            if operation == Operation::Purge && reference.is_none() {
                return Ok(());
            }
            let reference = reference
                .filter(|r| r.kind == "evidence")
                .ok_or(Error::Forbidden)?;
            let fact_id = reference
                .subject
                .parse::<i64>()
                .ok()
                .filter(|id| *id > 0)
                .ok_or(Error::NotFound)?;
            client.query_opt("SELECT id FROM people_training_attendance_facts WHERE tenant_id=$1 AND company_id=$2 AND id=$3", &[&scope.tenant(), &company.id.get(), &fact_id]).await?.ok_or(Error::NotFound)?;
            Ok(())
        })
    }
}

impl From<artifacts::Error> for TrainingError {
    fn from(error: artifacts::Error) -> Self {
        match error {
            artifacts::Error::Database(error) => error.into(),
            artifacts::Error::NotFound => Self::NotFound,
            artifacts::Error::InvalidPdf | artifacts::Error::InvalidDocument => Self::InvalidPdf,
            artifacts::Error::Policy => Self::ArtifactPolicy,
            other => Self::ArtifactFailed(other),
        }
    }
}

impl Training {
    /// Header-gated PDF upload. Artifacts commits its reservation and
    /// publication outside the owner transaction; its Create callback runs
    /// again before publication. Linking provenance then reauthorizes under
    /// the company lock. A failed link leaves a tracked artifact for expiry
    /// and recovery; never perform untracked filesystem cleanup.
    pub async fn attach(
        &self,
        scope: &Scope,
        company_id: i64,
        fact_id: i64,
        bytes: &[u8],
    ) -> Result<Evidence> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company_id, EVIDENCE_MANAGE)
            .await?;
        require_fact(&**client, scope, company_id, fact_id).await?;
        if !bytes.starts_with(b"%PDF-") {
            return Err(TrainingError::InvalidPdf);
        }
        let company = OrganisationId::new(company_id)?;
        let document = self
            .artifacts
            .for_owner(scope, company, &DocumentOwner)
            .put(
                &mut client,
                Reference {
                    subject: fact_id.to_string(),
                    kind: "evidence".into(),
                },
                bytes,
                "application/pdf",
            )
            .await?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.training.attach"),
            )
            .await?;
        writable(tx.sql(), scope, company_id).await?;
        let actor = self
            .authorize(tx.sql(), scope, company_id, EVIDENCE_MANAGE)
            .await?;
        require_fact(tx.sql(), scope, company_id, fact_id).await?;
        let tenant = scope.tenant();
        let saved = tx.mutate(move |sql| Box::pin(async move {
            let row = sql.query_one(&format!("INSERT INTO people_training_evidence (tenant_id, company_id, fact_id, artifact_id, actor_user_id, impersonator_id) VALUES ($1,$2,$3,$4::text::uuid,$5,$6) RETURNING {}", participation::EVIDENCE_COLUMNS), &[&tenant, &company_id, &fact_id, &document.id, &actor.user, &actor.impersonator]).await?;
            let saved = participation::evidence(&row);
            let change = Change { kind: "people_training_evidence".into(), id: saved.id, old: json!({}), new: json!(saved) };
            Ok((saved, vec![change]))
        })).await?;
        tx.commit().await?;
        Ok(saved)
    }

    /// Durable provenance remains listed after Base purges expired bytes.
    pub async fn evidence(
        &self,
        scope: &Scope,
        company_id: i64,
        fact_id: i64,
    ) -> Result<Vec<Evidence>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company_id, RECORDS_VIEW)
            .await?;
        if fact_id <= 0 {
            return Err(TrainingError::NotFound);
        }
        Ok(client.query(&format!("SELECT {} FROM people_training_evidence WHERE tenant_id=$1 AND company_id=$2 AND fact_id=$3 ORDER BY id", participation::EVIDENCE_COLUMNS), &[&scope.tenant(), &company_id, &fact_id]).await.map_err(Error::from)?.iter().map(participation::evidence).collect())
    }

    /// Every read re-proves the actor, company, fact and artifact lifecycle.
    pub async fn read_evidence(
        &self,
        scope: &Scope,
        company_id: i64,
        artifact_id: &str,
    ) -> Result<artifacts::Document> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company_id, RECORDS_VIEW)
            .await?;
        Ok(self
            .artifacts
            .for_owner(scope, OrganisationId::new(company_id)?, &DocumentOwner)
            .read(&mut client, artifact_id)
            .await?)
    }
    pub async fn purge(&self, scope: &Scope, company_id: i64) -> Result<artifacts::Recovery> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company_id, RETENTION_MANAGE)
            .await?;
        Ok(self
            .artifacts
            .for_owner(scope, OrganisationId::new(company_id)?, &DocumentOwner)
            .purge_expired(&mut client)
            .await?)
    }
    pub async fn purge_holds(
        &self,
        scope: &Scope,
        company_id: i64,
    ) -> Result<Vec<artifacts::PurgeHold>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company_id, RETENTION_MANAGE)
            .await?;
        Ok(self
            .artifacts
            .for_owner(scope, OrganisationId::new(company_id)?, &DocumentOwner)
            .list_purge_holds(&client)
            .await?)
    }
    pub async fn retry_purge(
        &self,
        scope: &Scope,
        company_id: i64,
        artifact_id: &str,
    ) -> Result<()> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company_id, RETENTION_MANAGE)
            .await?;
        Ok(self
            .artifacts
            .for_owner(scope, OrganisationId::new(company_id)?, &DocumentOwner)
            .retry_purge(&mut client, artifact_id)
            .await?)
    }
}
async fn require_fact(
    client: &impl tokio_postgres::GenericClient,
    scope: &Scope,
    company_id: i64,
    fact_id: i64,
) -> Result<()> {
    client.query_opt("SELECT id FROM people_training_attendance_facts WHERE tenant_id=$1 AND company_id=$2 AND id=$3", &[&scope.tenant(), &company_id, &fact_id]).await.map_err(Error::from)?.ok_or(TrainingError::NotFound)?;
    Ok(())
}
