//! Bilimbi Skills.Reassessments: one pending request per employee and skill.
//! Performing submits evidence into PK2; only independent finalization moves
//! the score. Both rows share the same captured mutation transaction.
use crate::{
    access, actions, assessments, catalog, policy, rows, AssessmentInput, PerformedReassessment,
    ReassessmentRequest, ReassessmentView, Result, Skills, SkillsError, Write, ASSESSMENTS_SUBMIT,
    REASSESSMENTS_EXECUTE, REASSESSMENTS_SUBMIT,
};
use bilibi_database::{Error, Result as DbResult};
use bilibi_people_workforce::authorization;
use bilibi_tenancy::Scope;
use tokio_postgres::{GenericClient, Row};

fn columns() -> String {
    format!("id,employee_id,skill_id,reason,requested_by_user_id,{},status,performed_by_user_id,{},assessment_id,{}",rows::date_text("due_on"),rows::datetime_text("performed_at"),rows::datetime_text("cancelled_at"))
}
fn request(r: &Row) -> DbResult<ReassessmentRequest> {
    Ok(ReassessmentRequest {
        id: r.get("id"),
        employee_id: r.get("employee_id"),
        skill_id: r.get("skill_id"),
        reason: r.get("reason"),
        requested_by_user_id: r.get("requested_by_user_id"),
        due_on: rows::date(r, "due_on")?,
        status: r.get("status"),
        performed_by_user_id: r.get("performed_by_user_id"),
        performed_at: rows::datetime(r, "performed_at")?,
        assessment_id: r.get("assessment_id"),
        cancelled_at: rows::datetime(r, "cancelled_at")?,
    })
}
async fn get(
    client: &impl GenericClient,
    tenant: i64,
    company: i64,
    id: i64,
    lock: bool,
) -> Result<ReassessmentRequest> {
    let suffix = if lock { " FOR UPDATE" } else { "" };
    let r=client.query_opt(&format!("SELECT {} FROM people_skill_reassessment_requests WHERE tenant_id=$1 AND company_id=$2 AND id=$3{suffix}",columns()), &[&tenant,&company,&id]).await.map_err(Error::from)?.ok_or(SkillsError::NotFound)?;
    Ok(request(&r)?)
}

impl Skills {
    /// Requests require a current score and active skill, a working employee
    /// within assessment reach, and a requester other than that employee.
    pub async fn request_reassessment(
        &self,
        scope: &Scope,
        company: i64,
        employee: i64,
        skill: i64,
        reason: &str,
    ) -> Result<ReassessmentRequest> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut w = self
            .begin(
                &mut client,
                scope,
                company,
                REASSESSMENTS_SUBMIT,
                "request_reassessment",
            )
            .await?;
        let reason = if reason.trim().is_empty() {
            return Err(SkillsError::ReasonRequired);
        } else {
            actions::text(reason, 1000, true)?.unwrap_or_default()
        };
        if employee <= 0 || skill <= 0 {
            return Err(SkillsError::InvalidAssessment);
        }
        self.workforce
            .lock_working_employee(&w.tx, scope, company, employee)
            .await?;
        self.workforce
            .employee(scope, company, employee)
            .await?
            .require_current()?;
        if self
            .workforce
            .linked_employee(w.sql(), scope, company)
            .await?
            == Some(employee)
        {
            return Err(SkillsError::SelfRequest);
        }
        if !access::reach(self, w.sql(), scope, company)
            .await?
            .contains(employee)
        {
            return Err(SkillsError::OutOfReach);
        }
        let scored=w.sql().query_opt("SELECT 1 FROM people_skill_scores WHERE tenant_id=$1 AND company_id=$2 AND employee_id=$3 AND skill_id=$4", &[&w.tenant,&company,&employee,&skill]).await.map_err(Error::from)?.is_some();
        if !scored
            || !catalog::get_skill(&w, skill)
                .await?
                .is_some_and(|s| s.active)
        {
            return Err(SkillsError::NoScore);
        }
        if w.sql().query_opt("SELECT 1 FROM people_skill_reassessment_requests WHERE tenant_id=$1 AND company_id=$2 AND employee_id=$3 AND skill_id=$4 AND status='pending'", &[&w.tenant,&company,&employee,&skill]).await.map_err(Error::from)?.is_some() {return Err(SkillsError::AlreadyOpen);}
        let policy = policy::get(&self.settings, w.sql(), scope, company).await?;
        let due = assessments::today()
            .checked_add(jiff::Span::new().days(policy.reassessment_due_days))
            .map_err(|_| SkillsError::InvalidDate)?
            .to_string();
        let (tenant, actor) = (w.tenant, w.actor);
        let row=w.tx.mutate(move |sql|Box::pin(async move {
            let r=sql.query_one(&format!("INSERT INTO people_skill_reassessment_requests (tenant_id,company_id,employee_id,skill_id,reason,requested_by_user_id,due_on,status) VALUES ($1,$2,$3,$4,$5,$6,$7::text::date,'pending') RETURNING {}",columns()), &[&tenant,&company,&employee,&skill,&reason,&actor,&due]).await?;
            let row=request(&r)?;
            Ok((row.clone(),vec![rows::created("people_skill_reassessment_request",row.id,&row)]))
        })).await?;
        w.tx.commit().await?;
        Ok(row)
    }

    /// Oldest due requests, at most 200. Performers see every pending request;
    /// requesters see only their own, even if their supervisor reach changes.
    pub async fn pending_reassessments(
        &self,
        scope: &Scope,
        company: i64,
    ) -> Result<Vec<ReassessmentView>> {
        let client = self.db.get().await.map_err(Error::from)?;
        access::authorize_any(
            &**client,
            scope,
            company,
            &[REASSESSMENTS_EXECUTE, REASSESSMENTS_SUBMIT],
        )
        .await?;
        self.current_company(scope, company).await?;
        let performer =
            authorization::allowed(&**client, scope, company, REASSESSMENTS_EXECUTE).await;
        let list=client.query(&format!("SELECT {} FROM people_skill_reassessment_requests WHERE tenant_id=$1 AND company_id=$2 AND status='pending' AND ($3 OR requested_by_user_id=$4) ORDER BY due_on,id LIMIT 200",columns()), &[&scope.tenant(),&company,&performer,&scope.user()]).await.map_err(Error::from)?.iter().map(request).collect::<DbResult<Vec<_>>>()?;
        let names = access::names(
            self,
            scope,
            company,
            &list.iter().map(|r| r.employee_id).collect::<Vec<_>>(),
        )
        .await?;
        let catalog = catalog::skills_by_ids(
            &**client,
            scope.tenant(),
            company,
            &list.iter().map(|r| r.skill_id).collect::<Vec<_>>(),
        )
        .await?;
        Ok(list
            .into_iter()
            .map(|r| ReassessmentView {
                employee_name: names
                    .get(&r.employee_id)
                    .cloned()
                    .unwrap_or_else(|| format!("Employee #{}", r.employee_id)),
                skill_name: catalog
                    .iter()
                    .find(|s| s.id == r.skill_id)
                    .map(|s| s.name.clone()),
                request: r,
            })
            .collect())
    }

    /// A pending request's requester or a performer may cancel it.
    pub async fn cancel_reassessment(
        &self,
        scope: &Scope,
        company: i64,
        id: i64,
    ) -> Result<ReassessmentRequest> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let cap = if authorization::allowed(&**client, scope, company, REASSESSMENTS_EXECUTE).await
        {
            REASSESSMENTS_EXECUTE
        } else {
            REASSESSMENTS_SUBMIT
        };
        let mut w = self
            .begin(&mut client, scope, company, cap, "cancel_reassessment")
            .await?;
        let old = get(w.sql(), w.tenant, company, id, true).await?;
        if old.status != "pending" {
            return Err(SkillsError::NotPending);
        }
        if old.requested_by_user_id != scope.user()
            && !authorization::allowed(w.sql(), scope, company, REASSESSMENTS_EXECUTE).await
        {
            return Err(SkillsError::NotFound);
        }
        let row = finish(&mut w, old, None).await?;
        w.tx.commit().await?;
        Ok(row)
    }

    /// Requires execute and assessment-submit, with company-wide execution
    /// reach but never self-assessment. Subject, skill and key are fixed by
    /// the request; input values for those fields and supersession are ignored.
    pub async fn perform_reassessment(
        &self,
        scope: &Scope,
        company: i64,
        id: i64,
        input: &AssessmentInput,
    ) -> Result<PerformedReassessment> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut w = self
            .begin(
                &mut client,
                scope,
                company,
                REASSESSMENTS_EXECUTE,
                "perform_reassessment",
            )
            .await?;
        authorization::authorize(w.sql(), scope, company, ASSESSMENTS_SUBMIT).await?;
        let peek = get(w.sql(), w.tenant, company, id, false).await?;
        if peek.status != "pending" {
            return Err(SkillsError::NotPending);
        }
        self.workforce
            .lock_working_employee(&w.tx, scope, company, peek.employee_id)
            .await?;
        self.workforce
            .employee(scope, company, peek.employee_id)
            .await?
            .require_current()?;
        if self
            .workforce
            .linked_employee(w.sql(), scope, company)
            .await?
            == Some(peek.employee_id)
        {
            return Err(SkillsError::SelfAssessment);
        }
        let old = get(w.sql(), w.tenant, company, id, true).await?;
        if old.status != "pending" {
            return Err(SkillsError::NotPending);
        }
        let input = assessments::normalize(&AssessmentInput {
            employee_id: old.employee_id,
            skill_id: old.skill_id,
            request_key: format!("reassessment:{}", old.id),
            supersedes_assessment_id: None,
            ..input.clone()
        })?;
        let assessment = match assessments::replay(&w, &input).await? {
            Some(a) => a,
            None => assessments::insert(self, &mut w, scope, &input).await?,
        };
        let request = finish(&mut w, old, Some(assessment.id)).await?;
        w.tx.commit().await?;
        Ok(PerformedReassessment {
            request,
            assessment,
        })
    }
}

async fn finish(
    w: &mut Write<'_>,
    old: ReassessmentRequest,
    assessment: Option<i64>,
) -> Result<ReassessmentRequest> {
    let (tenant, company, actor) = (w.tenant, w.company_id, w.actor);
    Ok(w.tx.mutate(move |sql|Box::pin(async move {
        let set=if assessment.is_some() {"status='performed',performed_by_user_id=$4,performed_at=(now() AT TIME ZONE 'UTC'),assessment_id=$5"} else {"status='cancelled',cancelled_by_user_id=$4,cancelled_at=(now() AT TIME ZONE 'UTC'),assessment_id=$5"};
        let r=sql.query_one(&format!("UPDATE people_skill_reassessment_requests SET {set},updated_at=(now() AT TIME ZONE 'UTC') WHERE tenant_id=$1 AND company_id=$2 AND id=$3 RETURNING {}",columns()), &[&tenant,&company,&old.id,&actor,&assessment]).await?;
        let row=request(&r)?;
        let mut after=serde_json::to_value(&row).map_err(|_|Error::Internal)?;
        if assessment.is_none() {after["cancelled_by_user_id"]=actor.into();}
        Ok((row.clone(),vec![rows::changed("people_skill_reassessment_request",row.id,&old,&after)]))
    })).await?)
}
