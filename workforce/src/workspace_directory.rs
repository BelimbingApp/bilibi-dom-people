//! Bilimbi Core Employee summaries plus Workforce.with_employment_facts:
//! directory facts for People workbenches, through the Workforce owner.
use crate::{
    authorization, employments, live_company, Employee, ReadResult, Result, Workforce,
    WorkforceError, EMPLOYEE_STATUSES,
};
use bilibi_database::{Error, MutationTransaction};
use bilibi_tenancy::Scope;
use serde::{Deserialize, Serialize};

/// Public directory facts, distinct from the private employment record.
#[derive(Clone, Debug, Serialize)]
pub struct DirectoryEmployee {
    pub id: i64,
    pub employee: Employee,
    pub status: String,
    pub employment_type: String,
}

/// An active company-defined employment type, for the employment picker.
#[derive(Clone, Debug, Deserialize)]
pub struct EmploymentType {
    pub code: String,
    pub label: String,
}

impl Workforce {
    /// All statuses for the People directory. As with `employees_at_any_status`,
    /// these are company-scoped summaries; private dates and identity ids stay out.
    pub async fn directory_employees(
        &self,
        scope: &Scope,
        company_id: i64,
    ) -> Result<ReadResult<Vec<DirectoryEmployee>>> {
        let client = self.db.get().await.map_err(Error::from)?;
        let company = live_company(&**client, scope, company_id).await?;
        let working = self
            .company_working_statuses(&**client, scope, &company)
            .await?;
        let statuses = EMPLOYEE_STATUSES
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        let rows =
            employments::in_statuses(&**client, scope.tenant(), company_id, &statuses).await?;
        let facts = rows
            .iter()
            .map(|row| {
                (
                    row.employment.id,
                    row.employment.status.clone(),
                    row.employment.employment_type.clone(),
                )
            })
            .collect::<Vec<_>>();
        let employees = self
            .project(&**client, scope, company_id, &working, rows)
            .await?;
        Ok(ReadResult::current(
            employees
                .into_iter()
                .zip(facts)
                .map(
                    |(employee, (id, status, employment_type))| DirectoryEmployee {
                        id,
                        employee,
                        status,
                        employment_type,
                    },
                )
                .collect(),
        ))
    }

    /// Locks the named employee under the company's writable lock for a
    /// sibling's transaction. The caller must authorize its own capability;
    /// this proof grants no write and exposes no private employment facts.
    pub async fn lock_employee(
        &self,
        tx: &MutationTransaction<'_>,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
    ) -> Result<()> {
        bilibi_company::require_writable(tx.sql(), scope, company_id, false).await?;
        let employee = employments::find(tx.sql(), scope.tenant(), company_id, employee_id)
            .await?
            .ok_or(WorkforceError::NotFound)?
            .employment;
        {
            // The owner proof locks tenant, company, person and affiliation
            // before the employment lock, matching create_employment's order.
            let proof = self
                .affiliations
                .lock_for_employment_at(
                    tx,
                    scope,
                    bilibi_party_ids::AffiliationId::new(employee.affiliation_id)?,
                    bilibi_party_ids::OrganisationId::new(company_id)?,
                )
                .await?;
            if proof.identity().person().get() != employee.person_id {
                return Err(WorkforceError::AffiliationMismatch);
            }
        }
        bilibi_company::require_writable(tx.sql(), scope, company_id, true).await?;
        employments::lock(tx.sql(), scope.tenant(), company_id, employee_id)
            .await?
            .ok_or(WorkforceError::NotFound)?;
        Ok(())
    }

    /// Active `employment_type` reference entries for the employment picker.
    /// No dependency from Employee Workspace
    /// to Reference Data: Workforce owns the employment input contract.
    pub async fn employment_types(
        &self,
        scope: &Scope,
        company_id: i64,
    ) -> Result<Vec<EmploymentType>> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, crate::EMPLOYMENT_MANAGE).await?;
        match self
            .ports
            .call(
                &client,
                scope,
                "people/reference_data.employment_types",
                &serde_json::json!({"company_id":company_id}),
            )
            .await
        {
            Ok(types) => Ok(types),
            Err(bilibi_ports::PortError::Unavailable(_)) => Err(WorkforceError::Unavailable),
            Err(bilibi_ports::PortError::Refused(error)) => Err(error.into()),
            Err(bilibi_ports::PortError::Shape { .. }) => {
                Err(WorkforceError::Failed(Error::Internal))
            }
        }
    }
}
