//! Bilimbi Performance review creation, independent release and employee response.
use crate::{store, tables::Table as T, validate as v, *};
use bilibi_database::Error;
use bilibi_tenancy::Scope;
use jiff::Timestamp;
use serde_json::json;

impl Performance {
    pub async fn draft_review(
        &self,
        scope: &Scope,
        company: i64,
        input: &ReviewInput,
    ) -> Result<Review> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut w = self
            .begin(&mut client, scope, company, REVIEWS_SUBMIT, "draft_review")
            .await?;
        let saved = self.create_review(&mut w, scope, input, None).await?;
        w.tx.commit().await?;
        Ok(saved)
    }
    pub async fn correct_review(
        &self,
        scope: &Scope,
        company: i64,
        id: i64,
        input: &ReviewCorrection,
    ) -> Result<Review> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut w = self
            .begin(
                &mut client,
                scope,
                company,
                REVIEWS_SUBMIT,
                "correct_review",
            )
            .await?;
        let prior: Review = store::get(w.tx.sql(), scope, company, T::Review, id).await?;
        if prior.actor_user_id != scope.user() || prior.status != "released" {
            return Err(PerformanceError::NotAuthor);
        }
        if store::successor(&w, T::Review, id).await? {
            return Err(PerformanceError::AlreadyCorrected);
        }
        let reason = v::reason(&input.change_reason)?;
        let next = ReviewInput {
            employee_id: prior.employee_id,
            description_id: input.description_id,
            period_start: prior.period_start.clone(),
            period_end: prior.period_end.clone(),
            cutoff_at: input
                .cutoff_at
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(&prior.cutoff_at)
                .to_owned(),
            outcome: input.outcome.clone(),
            rationale: input.rationale.clone(),
            observation_ids: input.observation_ids.clone(),
            target_ids: input.target_ids.clone(),
        };
        let saved = self
            .create_review(&mut w, scope, &next, Some((&prior, reason)))
            .await?;
        w.tx.commit().await?;
        Ok(saved)
    }
    async fn create_review(
        &self,
        w: &mut Write<'_>,
        scope: &Scope,
        input: &ReviewInput,
        prior: Option<(&Review, String)>,
    ) -> Result<Review> {
        let mut value = v::clean(
            input,
            &[
                "period_start",
                "period_end",
                "cutoff_at",
                "outcome",
                "rationale",
            ],
        )?;
        self.require_report(w, scope, input.employee_id, REVIEWS_SUBMIT)
            .await?;
        let start = v::text(&value, "period_start");
        let end = v::text(&value, "period_end");
        v::period(&start, &end)?;
        let cutoff = v::cutoff(&v::text(&value, "cutoff_at"))?;
        if cutoff.to_zoned(jiff::tz::TimeZone::UTC).date() < v::date(&end)? {
            return Err(PerformanceError::InvalidDates);
        }
        value["cutoff_at"] = json!(cutoff.to_string());
        let description: Description = store::get(
            w.tx.sql(),
            scope,
            w.company,
            T::Description,
            input.description_id,
        )
        .await?;
        if description.status != "published"
            || v::date(&description.effective_from)? > v::date(&start)?
            || description
                .effective_to
                .as_deref()
                .map(v::date)
                .transpose()?
                .is_some_and(|last| last.to_string() < end)
        {
            return Err(PerformanceError::DescriptionUnavailable);
        }
        self.validate_assignment(
            w.tx.sql(),
            scope,
            w.company,
            input.employee_id,
            &start,
            &end,
            &description,
        )
        .await?;
        value["status"] = json!("draft");
        value["version"] = json!(1);
        if let Some((prior, reason)) = prior {
            if cutoff < v::cutoff(&prior.cutoff_at)? {
                return Err(PerformanceError::InvalidDates);
            }
            value["version"] = json!(prior
                .version
                .checked_add(1)
                .ok_or(PerformanceError::InvariantRefused)?);
            value["supersedes_id"] = json!(prior.id);
            value["change_reason"] = json!(reason);
        }
        let observations = v::ids(&input.observation_ids)?;
        let targets = v::ids(&input.target_ids)?;
        if observations.is_empty() || targets.is_empty() {
            return Err(PerformanceError::EvidenceRequired);
        }
        for &id in &observations {
            let o: Observation =
                store::get(w.tx.sql(), scope, w.company, T::Observation, id).await?;
            if o.employee_id != input.employee_id
                || v::date(&o.window_start)? < v::date(&start)?
                || v::date(&o.window_end)? > v::date(&end)?
                || v::cutoff(&o.inserted_at)? > cutoff
            {
                return Err(PerformanceError::EvidenceOutsideWindow);
            }
        }
        for &id in &targets {
            let t: Target = store::get(w.tx.sql(), scope, w.company, T::Target, id).await?;
            if t.employee_id != input.employee_id
                || t.status != "published"
                || t.confidential
                || t.period_start != start
                || t.period_end != end
                || t.published_at
                    .as_deref()
                    .map(v::cutoff)
                    .transpose()?
                    .is_none_or(|t| t > cutoff)
            {
                return Err(PerformanceError::TargetUnavailable);
            }
        }
        let saved: Review = store::insert(w, T::Review, value).await?;
        for id in observations {
            let _: ReviewObservation = store::insert(
                w,
                T::ReviewObservation,
                json!({"review_id": saved.id, "observation_id": id}),
            )
            .await?;
        }
        for id in targets {
            let _: ReviewTarget = store::insert(
                w,
                T::ReviewTarget,
                json!({"review_id": saved.id, "target_id": id}),
            )
            .await?;
        }
        Ok(saved)
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn validate_assignment(
        &self,
        client: &(impl tokio_postgres::GenericClient + Sync),
        scope: &Scope,
        company: i64,
        employee: i64,
        start: &str,
        end: &str,
        description: &Description,
    ) -> Result<()> {
        self.workforce
            .employee(client, scope, company, employee)
            .await?
            .require_current()?;
        for day in [v::date(start)?, v::date(end)?] {
            let position = self
                .position(client, scope, company, description.position_id, day)
                .await?;
            if position.version != Some(description.position_version)
                || position.assignments_incomplete
                || !position
                    .assignments
                    .iter()
                    .any(|a| a.employee_reference.stable_id == employee.to_string())
            {
                return Err(PerformanceError::AssignmentUnavailable);
            }
        }
        Ok(())
    }
    pub async fn release_review(&self, scope: &Scope, company: i64, id: i64) -> Result<Review> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut w = self
            .begin(
                &mut client,
                scope,
                company,
                REVIEWS_APPROVE,
                "release_review",
            )
            .await?;
        let prior: Review = store::get(w.tx.sql(), scope, company, T::Review, id).await?;
        if prior.status != "draft" {
            return Err(PerformanceError::NotPending);
        }
        self.independent(
            &w,
            scope,
            prior.employee_id,
            prior.actor_user_id,
            REVIEWS_APPROVE,
        )
        .await?;
        self.workforce
            .lock_working_employee(&w.tx, scope, company, prior.employee_id)
            .await?;
        if v::cutoff(&prior.cutoff_at)? > Timestamp::now() {
            return Err(PerformanceError::FutureCutoff);
        }
        let description: Description = store::get(
            w.tx.sql(),
            scope,
            company,
            T::Description,
            prior.description_id,
        )
        .await?;
        self.validate_assignment(
            w.tx.sql(),
            scope,
            company,
            prior.employee_id,
            &prior.period_start,
            &prior.period_end,
            &description,
        )
        .await?;
        let pins = w.tx.sql().query_one(
            "SELECT EXISTS(SELECT 1 FROM people_performance_review_evidence WHERE tenant_id = $1 AND company_id = $2 AND review_id = $3) AND EXISTS(SELECT 1 FROM people_performance_review_targets WHERE tenant_id = $1 AND company_id = $2 AND review_id = $3)", &[&scope.tenant(), &company, &id],
        ).await.map_err(Error::from)?.get::<_, bool>(0);
        if !pins {
            return Err(PerformanceError::EvidenceRequired);
        }
        let saved = store::transition(
            &mut w,
            T::Review,
            id,
            prior,
            store::Transition::ReleaseReview,
        )
        .await?;
        w.tx.commit().await?;
        Ok(saved)
    }
    /// Resolves the account's current link under the employment lock on every call.
    pub async fn respond(
        &self,
        scope: &Scope,
        company: i64,
        id: i64,
        response: &str,
    ) -> Result<Response> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut w = self
            .begin(&mut client, scope, company, SELF_VIEW, "respond")
            .await?;
        let own = self
            .workforce
            .lock_self_employee(&w.tx, scope, company, SELF_VIEW)
            .await?;
        let review: Review = store::get(w.tx.sql(), scope, company, T::Review, id).await?;
        if review.employee_id != own.employee_id || review.status != "released" {
            return Err(PerformanceError::NotFound);
        }
        let mut value = v::clean(&json!({"response": response}), &["response"])?;
        if w.tx.sql().query_opt("SELECT 1 FROM people_performance_responses WHERE tenant_id = $1 AND company_id = $2 AND review_id = $3 AND actor_user_id = $4", &[&scope.tenant(), &company, &id, &scope.user()]).await.map_err(Error::from)?.is_some() { return Err(PerformanceError::InvariantRefused); }
        value["review_id"] = json!(id);
        value["employee_id"] = json!(own.employee_id);
        let saved = store::insert(&mut w, T::Response, value).await?;
        w.tx.commit().await?;
        Ok(saved)
    }
}
