//! The assessment and standing part of the Skills facade (Bilimbi Skills).
use crate::{
    access, assessments, standing, AssessableEmployee, Assessment, AssessmentDecision,
    AssessmentFilters, AssessmentInput, AssessmentQueue, AssessmentView, Coverage, Result,
    ReviewDecision, ScoreView, Skills, SkillsError, Standing, ASSESSMENTS_APPROVE,
    ASSESSMENTS_MANAGE, ASSESSMENTS_REVIEW, ASSESSMENTS_SUBMIT, ASSESSMENTS_VIEW,
    REASSESSMENTS_SUBMIT, SELF_VIEW,
};
use bilibi_database::Error;
use bilibi_people_workforce::authorization;
use bilibi_tenancy::Scope;
use jiff::civil::Date;

impl Skills {
    /// Submits evidence for a working employee in the actor's reach, never
    /// themselves. A request key replays the same five input facts as Bilimbi;
    /// an altered subject, skill, level, evidence or date is KeyConflict.
    pub async fn submit_assessment(
        &self,
        scope: &Scope,
        company_id: i64,
        input: &AssessmentInput,
    ) -> Result<Assessment> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let input = assessments::normalize(input)?;
        let mut w = self
            .begin(
                &mut client,
                scope,
                company_id,
                ASSESSMENTS_SUBMIT,
                "submit_assessment",
            )
            .await?;
        // Company -> employment -> assessment. This also fences account
        // relinking (which takes the company lock) while we prove this is not self.
        self.workforce
            .lock_working_employee(&w.tx, scope, company_id, input.employee_id)
            .await?;
        self.workforce
            .employee(scope, company_id, input.employee_id)
            .await?
            .require_current()?;
        if self
            .workforce
            .linked_employee(w.sql(), scope, company_id)
            .await?
            == Some(input.employee_id)
        {
            return Err(SkillsError::SelfAssessment);
        }
        let reach = access::reach(self, w.sql(), scope, company_id).await?;
        if !reach.contains(input.employee_id) {
            return Err(SkillsError::OutOfReach);
        }
        if let Some(row) = assessments::replay(&w, &input).await? {
            w.tx.rollback().await?;
            return Ok(row);
        }
        let row = assessments::insert(self, &mut w, scope, &input).await?;
        w.tx.commit().await?;
        Ok(row)
    }

    /// Verifies or returns pending work within reach. Both the assessor and
    /// subject are refused; returning needs a note. Corrections are new facts.
    pub async fn review_assessment(
        &self,
        scope: &Scope,
        company_id: i64,
        assessment_id: i64,
        decision: ReviewDecision,
        note: Option<&str>,
    ) -> Result<Assessment> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut w = self
            .begin(
                &mut client,
                scope,
                company_id,
                ASSESSMENTS_REVIEW,
                "review_assessment",
            )
            .await?;
        let note = assessments::note(note, decision)?;
        let subject = assessments::get(w.sql(), w.tenant, company_id, assessment_id).await?;
        self.workforce
            .lock_employee_subject(&w.tx, scope, company_id, subject.employee_id)
            .await?;
        let row = assessments::lock(&w, assessment_id).await?;
        if row.status != "pending_review" {
            return Err(SkillsError::NotPending);
        }
        let reach = access::reach(self, w.sql(), scope, company_id).await?;
        if !reach.contains(row.employee_id) {
            return Err(SkillsError::OutOfReach);
        }
        access::independent(
            self,
            w.sql(),
            scope,
            company_id,
            row.employee_id,
            row.assessor_user_id,
        )
        .await?;
        let kind = match decision {
            ReviewDecision::Verify => "verified",
            ReviewDecision::Return => "returned",
        };
        let row = assessments::decide(&mut w, row, kind, note).await?;
        w.tx.commit().await?;
        Ok(row)
    }

    /// Finalizes verified work with the independent approve capability.
    /// Bilimbi deliberately grants finalizers company-wide reach; they may
    /// also have reviewed it, but may never be its assessor or subject.
    pub async fn finalize_assessment(
        &self,
        scope: &Scope,
        company_id: i64,
        assessment_id: i64,
    ) -> Result<Assessment> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut w = self
            .begin(
                &mut client,
                scope,
                company_id,
                ASSESSMENTS_APPROVE,
                "finalize_assessment",
            )
            .await?;
        let subject = assessments::get(w.sql(), w.tenant, company_id, assessment_id).await?;
        self.workforce
            .lock_employee_subject(&w.tx, scope, company_id, subject.employee_id)
            .await?;
        let row = assessments::lock(&w, assessment_id).await?;
        if row.status != "verified" {
            return Err(SkillsError::NotVerified);
        }
        access::independent(
            self,
            w.sql(),
            scope,
            company_id,
            row.employee_id,
            row.assessor_user_id,
        )
        .await?;
        let row = assessments::decide(&mut w, row, "finalized", None).await?;
        assessments::refresh_score(&mut w, &row).await?;
        w.tx.commit().await?;
        Ok(row)
    }

    /// Working employees reachable for assessment or reassessment, by name,
    /// excluding the actor even when they hold assessments.manage.
    pub async fn assessable(
        &self,
        scope: &Scope,
        company_id: i64,
    ) -> Result<Vec<AssessableEmployee>> {
        let client = self.db.get().await.map_err(Error::from)?;
        access::authorize_any(
            &**client,
            scope,
            company_id,
            &[ASSESSMENTS_SUBMIT, REASSESSMENTS_SUBMIT],
        )
        .await?;
        self.current_company(scope, company_id).await?;
        let reach = access::reach(self, &**client, scope, company_id).await?;
        let own = self
            .workforce
            .linked_employee(&**client, scope, company_id)
            .await?;
        let employees = self
            .workforce
            .employees(scope, company_id)
            .await?
            .require_current()?;
        let mut options = Vec::new();
        for employee in employees {
            let id = access::employee_id(&employee)?;
            if reach.contains(id) && own != Some(id) {
                options.push(AssessableEmployee {
                    id,
                    name: format!("{} ({})", employee.display_name, employee.employee_number),
                });
            }
        }
        options.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(options)
    }

    /// The newest 200 assessments within reach, optionally filtered.
    pub async fn list_assessments(
        &self,
        scope: &Scope,
        company_id: i64,
        filters: &AssessmentFilters,
    ) -> Result<Vec<AssessmentView>> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, ASSESSMENTS_VIEW).await?;
        self.current_company(scope, company_id).await?;
        let reach = access::reach(self, &**client, scope, company_id).await?;
        let rows =
            assessments::list(&**client, scope.tenant(), company_id, &reach, filters).await?;
        assessments::present(self, &**client, scope, company_id, rows).await
    }

    /// Oldest independent review work in reach, company-wide finalization
    /// work, and the actor's returned work that has no correction. Each starts
    /// with at most 200 candidates before independence filtering, as Bilimbi
    /// does; a capability is rechecked for each decision bucket.
    pub async fn assessment_queue(
        &self,
        scope: &Scope,
        company_id: i64,
    ) -> Result<AssessmentQueue> {
        let client = self.db.get().await.map_err(Error::from)?;
        access::authorize_any(
            &**client,
            scope,
            company_id,
            &[
                ASSESSMENTS_VIEW,
                ASSESSMENTS_SUBMIT,
                ASSESSMENTS_REVIEW,
                ASSESSMENTS_APPROVE,
            ],
        )
        .await?;
        self.current_company(scope, company_id).await?;
        let reach = access::reach(self, &**client, scope, company_id).await?;
        let own = self
            .workforce
            .linked_employee(&**client, scope, company_id)
            .await?;
        let review =
            if authorization::allowed(&**client, scope, company_id, ASSESSMENTS_REVIEW).await {
                assessments::queue_rows(
                    &**client,
                    scope.tenant(),
                    company_id,
                    reach.ids(),
                    scope.user(),
                    own,
                    "pending_review",
                )
                .await?
            } else {
                vec![]
            };
        let finalize =
            if authorization::allowed(&**client, scope, company_id, ASSESSMENTS_APPROVE).await {
                assessments::queue_rows(
                    &**client,
                    scope.tenant(),
                    company_id,
                    None,
                    scope.user(),
                    own,
                    "verified",
                )
                .await?
            } else {
                vec![]
            };
        let returned = assessments::queue_rows(
            &**client,
            scope.tenant(),
            company_id,
            None,
            scope.user(),
            own,
            "returned",
        )
        .await?;
        Ok(AssessmentQueue {
            review: assessments::present(self, &**client, scope, company_id, review).await?,
            finalize: assessments::present(self, &**client, scope, company_id, finalize).await?,
            returned: assessments::present(self, &**client, scope, company_id, returned).await?,
        })
    }

    /// The append-only trail of an assessment within the reader's reach.
    pub async fn assessment_decisions(
        &self,
        scope: &Scope,
        company_id: i64,
        assessment_id: i64,
    ) -> Result<Vec<AssessmentDecision>> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, ASSESSMENTS_VIEW).await?;
        self.current_company(scope, company_id).await?;
        let reach = access::reach(self, &**client, scope, company_id).await?;
        let row = assessments::get(&**client, scope.tenant(), company_id, assessment_id).await?;
        if !reach.contains(row.employee_id) {
            return Err(SkillsError::NotFound);
        }
        assessments::decisions(&**client, scope.tenant(), company_id, assessment_id).await
    }

    /// Finalized scores with gaps within reach, mandatory and highest
    /// priority first; at most 500.
    pub async fn gaps(&self, scope: &Scope, company_id: i64) -> Result<Vec<ScoreView>> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, ASSESSMENTS_VIEW).await?;
        self.current_company(scope, company_id).await?;
        let reach = access::reach(self, &**client, scope, company_id).await?;
        standing::gaps(self, &**client, scope, company_id, &reach).await
    }

    /// The signed-in working employee's scores, visible actions and pending
    /// reassessments. The employment link is resolved afresh on every call.
    pub async fn standing(&self, scope: &Scope, company_id: i64) -> Result<Standing> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, SELF_VIEW).await?;
        self.current_company(scope, company_id).await?;
        let me = self
            .workforce
            .authorize_self(scope, company_id, SELF_VIEW)
            .await?;
        standing::own(self, &**client, scope, company_id, me.employee_id).await
    }

    /// Active critical skills and current competent working holders at the
    /// company's backup minimum; assessments.view plus assessments.manage.
    pub async fn coverage(
        &self,
        scope: &Scope,
        company_id: i64,
        as_of: Date,
    ) -> Result<Vec<Coverage>> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorization::authorize(&**client, scope, company_id, ASSESSMENTS_VIEW).await?;
        self.current_company(scope, company_id).await?;
        if !authorization::allowed(&**client, scope, company_id, ASSESSMENTS_MANAGE).await {
            return Err(SkillsError::OutOfReach);
        }
        standing::coverage(self, &**client, scope, company_id, as_of).await
    }
}
