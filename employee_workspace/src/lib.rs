//! People EmployeeWorkspace, mirroring Bilimbi's EmployeeWorkspace facade.
mod contributions;

pub const MODULE: &str = "people/employee_workspace";
pub const SERVICE: &str = "people/employee_workspace.employee_workspace";
pub const VIEW: &str = "people.employees.view";
pub const MANAGE: &str = "people.employees.manage";
pub const REVIEW: &str = "people.employees.review";
pub const CAPABILITIES: [&str; 3] = [VIEW, MANAGE, REVIEW];

mod store;
mod web;
use bilibi_database::{Db, Error, MutationBoundary};
use bilibi_people_workforce::{authorization, DirectoryEmployee, Workforce, WorkforceError};
use bilibi_tenancy::Scope;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub type Result<T> = std::result::Result<T, WorkspaceError>;
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error(transparent)]
    Workforce(#[from] WorkforceError),
    #[error("This request has already been reviewed.")]
    AlreadyReviewed,
    #[error("This record has changed. Refresh it and try again.")]
    Stale,
    #[error("Invalid review")]
    InvalidReview,
}
impl From<Error> for WorkspaceError {
    fn from(error: Error) -> Self {
        WorkforceError::from(error).into()
    }
}
impl From<bilibi_company::WriteError> for WorkspaceError {
    fn from(error: bilibi_company::WriteError) -> Self {
        WorkforceError::from(error).into()
    }
}

/// People's work facts, separate from Core-owned identity and contacts.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WorkProfile {
    pub work_location: Option<String>,
    pub work_arrangement: Option<String>,
    pub notes: Option<String>,
}
/// A stored People fact and its optimistic concurrency fence. An absent row
/// starts at version zero; its first write creates version one.
#[derive(Clone, Debug)]
pub struct FactRecord<T> {
    pub id: Option<i64>,
    pub version: i64,
    pub value: T,
}
/// Portal eligibility records a People decision; it does not create a login.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Access {
    pub portal_enabled: bool,
    pub reason: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChangeRequest {
    pub id: i64,
    pub field: String,
    pub proposed_value: String,
    pub reason: Option<String>,
    pub status: String,
    pub requested_by_actor_id: i64,
    pub reviewed_by_actor_id: Option<i64>,
    /// UTC, stored as Bilimbi's timestamp without time zone.
    pub reviewed_at: Option<String>,
    pub inserted_at: String,
}
#[derive(Clone, Debug)]
pub struct NewChangeRequest {
    pub field: String,
    pub proposed_value: String,
    pub reason: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedView {
    pub id: i64,
    pub name: String,
    pub search: Option<String>,
    pub status: Option<String>,
}
#[derive(Clone, Debug, Default)]
pub struct NewSavedView {
    pub name: String,
    pub search: Option<String>,
    pub status: Option<String>,
}
pub const CHANGE_FIELDS: [&str; 4] = ["full_name", "short_name", "email", "mobile_number"];

/// The sole boundary for Employee Workspace reads and writes. Every operation
/// authorizes against the current grant and explicit company through Workforce.
pub struct EmployeeWorkspace {
    db: Db,
    mutations: Arc<MutationBoundary>,
    workforce: Arc<Workforce>,
}
impl EmployeeWorkspace {
    pub(crate) fn new(db: Db, mutations: Arc<MutationBoundary>, workforce: Arc<Workforce>) -> Self {
        Self {
            db,
            mutations,
            workforce,
        }
    }
    async fn authorize(&self, scope: &Scope, company_id: i64, capability: &str) -> Result<()> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, capability).await?;
        Ok(())
    }
    pub async fn employees(
        &self,
        scope: &Scope,
        company_id: i64,
    ) -> Result<Vec<DirectoryEmployee>> {
        let client = self.db.get().await.map_err(bilibi_database::Error::from)?;
        authorization::authorize(&**client, scope, company_id, VIEW).await?;
        self.workforce
            .directory_employees(&**client, scope, company_id)
            .await?
            .require_current()
            .map_err(|_| WorkforceError::Unavailable.into())
    }
    pub async fn employee(
        &self,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
    ) -> Result<DirectoryEmployee> {
        self.employees(scope, company_id)
            .await?
            .into_iter()
            .find(|e| e.id == employee_id)
            .ok_or_else(|| WorkforceError::NotFound.into())
    }
    pub async fn work_profile(
        &self,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
    ) -> Result<Option<WorkProfile>> {
        self.employee(scope, company_id, employee_id).await?;
        let client = self.db.get().await.map_err(Error::from)?;
        store::profile(&**client, scope, company_id, employee_id).await
    }
    pub async fn access(
        &self,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
    ) -> Result<Option<Access>> {
        self.employee(scope, company_id, employee_id).await?;
        let client = self.db.get().await.map_err(Error::from)?;
        store::access(&**client, scope, company_id, employee_id).await
    }
    pub async fn change_requests(
        &self,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
    ) -> Result<Vec<ChangeRequest>> {
        self.employee(scope, company_id, employee_id).await?;
        let client = self.db.get().await.map_err(Error::from)?;
        store::requests(&**client, scope, company_id, employee_id).await
    }
    pub async fn work_profile_record(
        &self,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
    ) -> Result<FactRecord<WorkProfile>> {
        self.employee(scope, company_id, employee_id).await?;
        let client = self.db.get().await.map_err(Error::from)?;
        store::profile_record(&**client, scope, company_id, employee_id).await
    }
    pub async fn access_record(
        &self,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
    ) -> Result<FactRecord<Access>> {
        self.employee(scope, company_id, employee_id).await?;
        let client = self.db.get().await.map_err(Error::from)?;
        store::access_record(&**client, scope, company_id, employee_id).await
    }
    /// Changes one inline fact under the employment lock. Checking the stored
    /// version and merging here prevents a stale page overwriting another edit.
    #[allow(clippy::too_many_arguments)]
    pub async fn update_work_profile(
        &self,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
        version: i64,
        field: &str,
        value: &str,
    ) -> Result<WorkProfile> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.employee_workspace.put_work_profile"),
            )
            .await?;
        authorization::authorize(tx.sql(), scope, company_id, MANAGE).await?;
        self.workforce
            .lock_employee(&tx, scope, company_id, employee_id)
            .await?;
        authorization::authorize(tx.sql(), scope, company_id, MANAGE).await?;
        let current = store::profile_record(tx.sql(), scope, company_id, employee_id).await?;
        if current.version != version {
            return Err(WorkspaceError::Stale);
        }
        let mut profile = current.value;
        match field {
            "work_location" => profile.work_location = Some(value.into()),
            "work_arrangement" => profile.work_arrangement = Some(value.into()),
            "notes" => profile.notes = Some(value.into()),
            _ => {
                return Err(WorkforceError::Invalid(vec![
                    bilibi_people_workforce::FieldError::new("field", "is invalid"),
                ])
                .into())
            }
        }
        let profile = store::validate_profile(&profile)?;
        let result = store::put_profile(&mut tx, scope, company_id, employee_id, profile).await?;
        tx.commit().await?;
        Ok(result)
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn update_access(
        &self,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
        version: i64,
        field: &str,
        value: &str,
    ) -> Result<Access> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.employee_workspace.put_access"),
            )
            .await?;
        authorization::authorize(tx.sql(), scope, company_id, MANAGE).await?;
        self.workforce
            .lock_employee(&tx, scope, company_id, employee_id)
            .await?;
        authorization::authorize(tx.sql(), scope, company_id, MANAGE).await?;
        let current = store::access_record(tx.sql(), scope, company_id, employee_id).await?;
        if current.version != version {
            return Err(WorkspaceError::Stale);
        }
        let mut access = current.value;
        match field {
            "portal_enabled" if ["true", "false"].contains(&value) => {
                access.portal_enabled = value == "true"
            }
            "reason" => access.reason = Some(value.into()),
            _ => {
                return Err(WorkforceError::Invalid(vec![
                    bilibi_people_workforce::FieldError::new("field", "is invalid"),
                ])
                .into())
            }
        }
        let access = store::validate_access(&access)?;
        let result = store::put_access(&mut tx, scope, company_id, employee_id, access).await?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn put_work_profile(
        &self,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
        profile: &WorkProfile,
    ) -> Result<WorkProfile> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.employee_workspace.put_work_profile"),
            )
            .await?;
        authorization::authorize(tx.sql(), scope, company_id, MANAGE).await?;
        let profile = store::validate_profile(profile)?;
        self.workforce
            .lock_employee(&tx, scope, company_id, employee_id)
            .await?;
        authorization::authorize(tx.sql(), scope, company_id, MANAGE).await?;
        let result = store::put_profile(&mut tx, scope, company_id, employee_id, profile).await?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn put_access(
        &self,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
        access: &Access,
    ) -> Result<Access> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.employee_workspace.put_access"),
            )
            .await?;
        authorization::authorize(tx.sql(), scope, company_id, MANAGE).await?;
        let access = store::validate_access(access)?;
        self.workforce
            .lock_employee(&tx, scope, company_id, employee_id)
            .await?;
        authorization::authorize(tx.sql(), scope, company_id, MANAGE).await?;
        let result = store::put_access(&mut tx, scope, company_id, employee_id, access).await?;
        tx.commit().await?;
        Ok(result)
    }
    /// Records a proposal only; approval does not change Core identity facts.
    pub async fn request_change(
        &self,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
        request: &NewChangeRequest,
    ) -> Result<ChangeRequest> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.employee_workspace.request_change"),
            )
            .await?;
        authorization::authorize(tx.sql(), scope, company_id, MANAGE).await?;
        let request = store::validate_request(request)?;
        self.workforce
            .lock_employee(&tx, scope, company_id, employee_id)
            .await?;
        authorization::authorize(tx.sql(), scope, company_id, MANAGE).await?;
        let result =
            store::insert_request(&mut tx, scope, company_id, employee_id, request).await?;
        tx.commit().await?;
        Ok(result)
    }
    /// Reviews a pending request once, recording the reviewer from the scope.
    /// Bilimbi allows the requester to review; no independent-review rule applies.
    pub async fn review_change(
        &self,
        scope: &Scope,
        company_id: i64,
        employee_id: i64,
        request_id: i64,
        status: &str,
    ) -> Result<ChangeRequest> {
        if request_id <= 0 || !["approved", "rejected"].contains(&status) {
            return Err(WorkspaceError::InvalidReview);
        }
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.employee_workspace.review_change"),
            )
            .await?;
        authorization::authorize(tx.sql(), scope, company_id, REVIEW).await?;
        self.workforce
            .lock_employee(&tx, scope, company_id, employee_id)
            .await?;
        authorization::authorize(tx.sql(), scope, company_id, REVIEW).await?;
        let result =
            store::review_request(&mut tx, scope, company_id, employee_id, request_id, status)
                .await?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn saved_views(&self, scope: &Scope, company_id: i64) -> Result<Vec<SavedView>> {
        self.authorize(scope, company_id, VIEW).await?;
        let client = self.db.get().await.map_err(Error::from)?;
        store::views(&**client, scope, company_id).await
    }
    pub async fn save_view(
        &self,
        scope: &Scope,
        company_id: i64,
        view: &NewSavedView,
    ) -> Result<SavedView> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .mutations
            .begin_view_state(
                &mut client,
                scope.mutation_context("people.employee_workspace.save_view"),
            )
            .await?;
        authorization::authorize(tx.sql(), scope, company_id, VIEW).await?;
        let view = store::validate_view(view)?;
        bilibi_company::require_writable(tx.sql(), scope, company_id, true).await?;
        authorization::authorize(tx.sql(), scope, company_id, VIEW).await?;
        let result = store::insert_view(&mut tx, scope, company_id, view).await?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn delete_view(&self, scope: &Scope, company_id: i64, view_id: i64) -> Result<()> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .mutations
            .begin_view_state(
                &mut client,
                scope.mutation_context("people.employee_workspace.delete_view"),
            )
            .await?;
        authorization::authorize(tx.sql(), scope, company_id, VIEW).await?;
        bilibi_company::require_writable(tx.sql(), scope, company_id, true).await?;
        authorization::authorize(tx.sql(), scope, company_id, VIEW).await?;
        store::delete_view(&mut tx, scope, company_id, view_id).await?;
        tx.commit().await?;
        Ok(())
    }
}
