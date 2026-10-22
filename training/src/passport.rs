//! Mirrors Bilimbi Training.Passport. Latest attendance, original-revision evidence.
use crate::{passport_store as store, *};
use bilibi_artifacts::{Metadata, Reference};
use bilibi_people_workforce::{Employee, ReadResult, WorkforceError};
use serde::Serialize;
use serde_json::json;
use std::collections::HashMap;

pub const PASSPORT_MY_VIEW: &str = "people.training.passport.my.view";
pub const PASSPORT_TEAM_VIEW: &str = "people.training.passport.team.view";
pub const PASSPORT_GENERATE: &str = "people.training.passport.generate";
pub const PASSPORT_LIMIT: usize = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PassportAudience {
    SelfService,
    Team,
}
impl PassportAudience {
    pub fn capability(self) -> &'static str {
        match self {
            Self::SelfService => PASSPORT_MY_VIEW,
            Self::Team => PASSPORT_TEAM_VIEW,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SelfService => "self",
            Self::Team => "team",
        }
    }
    pub(crate) fn subject(self, employee: i64) -> String {
        format!("{}:{employee}", self.as_str())
    }
    pub(crate) fn from_subject(subject: &str) -> Result<(Self, i64)> {
        let (kind, id) = subject.split_once(':').ok_or(TrainingError::NotFound)?;
        let audience = match kind {
            "self" => Self::SelfService,
            "team" => Self::Team,
            _ => return Err(TrainingError::NotFound),
        };
        let id = id
            .parse::<i64>()
            .ok()
            .filter(|id| *id > 0)
            .ok_or(TrainingError::NotFound)?;
        Ok((audience, id))
    }
}
/// Bilimbi's page envelope. Inputs use `page` and `perPage` (25, 50, 100, 300).
#[derive(Clone, Debug, Serialize)]
pub struct TrainingPage<T> {
    pub entries: Vec<T>,
    pub page: i64,
    pub page_size: i64,
    pub total_entries: i64,
    pub total_pages: i64,
}
impl<T> TrainingPage<T> {
    pub(crate) fn empty(params: &HashMap<String, String>, total: i64) -> Self {
        let positive = |key: &str| {
            params
                .get(key)
                .and_then(|s| s.parse::<i64>().ok())
                .filter(|n| *n > 0)
        };
        let size = positive("perPage")
            .filter(|n| [25, 50, 100, 300].contains(n))
            .unwrap_or(25);
        let pages = total / size + i64::from(total % size != 0);
        Self {
            entries: Vec::new(),
            page: positive("page").unwrap_or(1).min(pages.max(1)),
            page_size: size,
            total_entries: total,
            total_pages: pages,
        }
    }
    pub(crate) fn offset(&self) -> i64 {
        (self.page - 1) * self.page_size
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct PassportEvidence {
    pub fact_id: i64,
    pub artifact_id: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct PassportRow {
    pub fact_id: i64,
    pub employee_id: i64,
    pub course: String,
    pub session: String,
    /// Whole-second UTC ISO instant.
    pub ends_at: String,
    pub status: String,
    pub revision: i32,
    pub evidence: Vec<PassportEvidence>,
}
#[derive(Clone, Debug)]
pub struct PassportDirectory {
    pub employees: Vec<Employee>,
    pub freshness: Freshness,
}
#[derive(Clone, Debug)]
pub struct Passport {
    pub employee: Employee,
    pub freshness: Freshness,
    pub page: TrainingPage<PassportRow>,
}

impl Training {
    /// Own working employee, or current direct reports (never the manager themself).
    /// Native Workforce is current; retain freshness for the self-only stale read contract.
    pub async fn passport_employees(
        &self,
        scope: &Scope,
        company: i64,
        audience: PassportAudience,
    ) -> Result<PassportDirectory> {
        let client = self.db.get().await.map_err(Error::from)?;
        let company_row =
            authorization::authorize_company(&**client, scope, company, audience.capability())
                .await?;
        if company_row.archived {
            return Err(TrainingError::CompanyUnavailable);
        }
        drop(client);
        let own = self
            .workforce
            .self_employee(scope, company)
            .await
            .map_err(|e| match e {
                WorkforceError::NotLinked => TrainingError::EmployeeUnavailable,
                other => other.into(),
            })?;
        let read = match audience {
            PassportAudience::SelfService => {
                let read = self
                    .workforce
                    .employee(scope, company, own.employee_id)
                    .await?;
                ReadResult {
                    value: read.value.map(|e| vec![e]),
                    freshness: read.freshness,
                }
            }
            PassportAudience::Team => {
                self.workforce
                    .direct_reports(scope, company, own.employee_id)
                    .await?
            }
        };
        let freshness = read.freshness.clone();
        let employees = match audience {
            PassportAudience::SelfService => read
                .value
                .ok_or_else(|| TrainingError::WorkforceNotCurrent(freshness.clone()))?,
            PassportAudience::Team => read
                .require_current()
                .map_err(|n| TrainingError::WorkforceNotCurrent(n.0))?,
        };
        Ok(PassportDirectory {
            employees,
            freshness,
        })
    }
    pub async fn passport(
        &self,
        scope: &Scope,
        company: i64,
        audience: PassportAudience,
        employee: i64,
        params: &HashMap<String, String>,
    ) -> Result<Passport> {
        let directory = self.passport_employees(scope, company, audience).await?;
        let employee = directory
            .employees
            .into_iter()
            .find(|e| e.reference.stable_id == employee.to_string())
            .ok_or(TrainingError::OutsideTeam)?;
        let employee_id = employee
            .reference
            .stable_id
            .parse::<i64>()
            .map_err(|_| TrainingError::NotFound)?;
        let client = self.db.get().await.map_err(Error::from)?;
        let page = store::passport(&**client, scope.tenant(), company, employee_id, params).await?;
        Ok(Passport {
            employee,
            freshness: directory.freshness,
            page,
        })
    }
    /// Complete snapshot, never a partial PDF. Artifacts reauthorizes Create before
    /// rendering, reservation and publication; the owner holds the employment proof.
    pub async fn generate_passport(
        &self,
        scope: &Scope,
        company: i64,
        audience: PassportAudience,
        employee: i64,
    ) -> Result<Metadata> {
        let passport = self
            .passport(scope, company, audience, employee, &HashMap::new())
            .await?;
        if passport.freshness != Freshness::Current {
            return Err(TrainingError::WorkforceNotCurrent(passport.freshness));
        }
        let mut client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company, PASSPORT_GENERATE)
            .await
            .map_err(|error| match error {
                TrainingError::Unauthorized => TrainingError::DocumentForbidden,
                other => other,
            })?;
        let rows = store::all_passport(&**client, scope.tenant(), company, employee).await?;
        if rows.len() > PASSPORT_LIMIT {
            return Err(TrainingError::PassportTooLarge);
        }
        let owner = self.document_owner();
        Ok(self.artifacts.for_owner(scope, OrganisationId::new(company)?, &owner).generate_pdf(
            &mut client,
            Reference { subject: audience.subject(employee), kind: "passport".into() },
            &json!({"employee": passport.employee.display_name, "employee_id": employee, "generated": clock::whole_second(Timestamp::now()).to_string(), "rows": rows}),
        ).await?)
    }
    /// Used by both My and Team download routes; the reference retains its audience.
    pub async fn read_passport(
        &self,
        scope: &Scope,
        company: i64,
        artifact: &str,
    ) -> Result<bilibi_artifacts::Document> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let owner = self.document_owner();
        let document = self
            .artifacts
            .for_owner(scope, OrganisationId::new(company)?, &owner)
            .read(&mut client, artifact)
            .await?;
        if document.metadata.reference.kind != "passport" {
            return Err(TrainingError::NotFound);
        }
        Ok(document)
    }
    /// A passport viewer can read evidence on any revision of their own or a
    /// current direct report's fact. The PDF and provenance stay revision-bound.
    pub async fn read_passport_evidence(
        &self,
        scope: &Scope,
        company: i64,
        artifact: &str,
    ) -> Result<bilibi_artifacts::Document> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let owner = self.document_owner();
        let document = self
            .artifacts
            .for_owner(scope, OrganisationId::new(company)?, &owner)
            .read(&mut client, artifact)
            .await?;
        if document.metadata.reference.kind != "evidence" {
            return Err(TrainingError::NotFound);
        }
        Ok(document)
    }
}

impl Passport {
    /// Bilimbi's stale/unavailable notice; current native reads need no notice.
    pub fn workforce_warning(&self) -> Option<String> {
        (self.freshness != Freshness::Current).then(|| not_current_message(&self.freshness))
    }
}
impl PassportDirectory {
    pub fn workforce_warning(&self) -> Option<String> {
        (self.freshness != Freshness::Current).then(|| not_current_message(&self.freshness))
    }
}
