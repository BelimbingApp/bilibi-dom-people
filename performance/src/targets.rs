//! Bilimbi Performance: direct-report proposals, independent review and communication.
use crate::{store, tables::Table as T, validate as v, *};
use bilibi_database::Error;
use bilibi_tenancy::Scope;
use serde_json::json;

impl Performance {
    pub async fn propose_target(
        &self,
        scope: &Scope,
        company: i64,
        input: &TargetInput,
    ) -> Result<Target> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut w = self
            .begin(&mut client, scope, company, KPIS_SUBMIT, "propose_target")
            .await?;
        let mut value = v::clean(
            input,
            &["target", "period_start", "period_end", "effective_from"],
        )?;
        self.require_report(&w, scope, input.employee_id, KPIS_SUBMIT)
            .await?;
        let definition: Definition = store::get(
            w.tx.sql(),
            scope,
            company,
            T::Definition,
            input.definition_id,
        )
        .await?;
        let start = v::text(&value, "period_start");
        let end = v::text(&value, "period_end");
        let first = v::text(&value, "effective_from");
        v::period(&start, &end)?;
        v::period(&start, &first)?;
        v::period(&first, &end)?;
        value["definition_version"] = json!(definition.version);
        value["status"] = json!("proposed");
        value["version"] = json!(1);
        let saved = store::insert(&mut w, T::Target, value).await?;
        w.tx.commit().await?;
        Ok(saved)
    }
    pub async fn amend_target(
        &self,
        scope: &Scope,
        company: i64,
        id: i64,
        input: &TargetAmendment,
    ) -> Result<Target> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut w = self
            .begin(&mut client, scope, company, KPIS_SUBMIT, "amend_target")
            .await?;
        let prior: Target = store::get(w.tx.sql(), scope, company, T::Target, id).await?;
        self.require_report(&w, scope, prior.employee_id, KPIS_SUBMIT)
            .await?;
        if prior.status != "published" {
            return Err(PerformanceError::NotReleased);
        }
        if store::successor(&w, T::Target, id).await? {
            return Err(PerformanceError::AlreadyCorrected);
        }
        let mut value = v::clean(input, &["target", "effective_from", "change_reason"])?;
        let first = v::text(&value, "effective_from");
        if v::date(&first)? <= v::date(&prior.effective_from)?
            || v::date(&first)? > v::date(&prior.period_end)?
        {
            return Err(PerformanceError::InvalidDates);
        }
        value["definition_id"] = json!(prior.definition_id);
        value["definition_version"] = json!(prior.definition_version);
        value["employee_id"] = json!(prior.employee_id);
        value["period_start"] = json!(prior.period_start);
        value["period_end"] = json!(prior.period_end);
        value["confidential"] = json!(prior.confidential);
        value["version"] = json!(prior
            .version
            .checked_add(1)
            .ok_or(PerformanceError::InvariantRefused)?);
        value["supersedes_id"] = json!(prior.id);
        value["status"] = json!("proposed");
        let saved = store::insert(&mut w, T::Target, value).await?;
        w.tx.commit().await?;
        Ok(saved)
    }
    pub async fn review_target(
        &self,
        scope: &Scope,
        company: i64,
        id: i64,
        note: &str,
    ) -> Result<Target> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut w = self
            .begin(&mut client, scope, company, KPIS_REVIEW, "review_target")
            .await?;
        let prior: Target = store::get(w.tx.sql(), scope, company, T::Target, id).await?;
        if prior.status != "proposed" {
            return Err(PerformanceError::NotPending);
        }
        self.independent(
            &w,
            scope,
            prior.employee_id,
            prior.actor_user_id,
            KPIS_REVIEW,
        )
        .await?;
        let note = v::reason(note)?;
        let saved = store::transition(
            &mut w,
            T::Target,
            id,
            prior,
            store::Transition::ReviewTarget(note),
        )
        .await?;
        w.tx.commit().await?;
        Ok(saved)
    }
    pub async fn publish_target(&self, scope: &Scope, company: i64, id: i64) -> Result<Target> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut w = self
            .begin(&mut client, scope, company, KPIS_APPROVE, "publish_target")
            .await?;
        let prior: Target = store::get(w.tx.sql(), scope, company, T::Target, id).await?;
        if prior.status != "reviewed" || prior.confidential {
            return Err(PerformanceError::NotPublishable);
        }
        self.independent(
            &w,
            scope,
            prior.employee_id,
            prior.actor_user_id,
            KPIS_APPROVE,
        )
        .await?;
        self.workforce
            .lock_working_employee(&w.tx, scope, company, prior.employee_id)
            .await?;
        let saved = store::transition(
            &mut w,
            T::Target,
            id,
            prior,
            store::Transition::PublishTarget,
        )
        .await?;
        w.tx.commit().await?;
        Ok(saved)
    }
    pub async fn record_observation(
        &self,
        scope: &Scope,
        company: i64,
        input: &ObservationInput,
    ) -> Result<Observation> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut w = self
            .begin(
                &mut client,
                scope,
                company,
                REVIEWS_SUBMIT,
                "record_observation",
            )
            .await?;
        let value = v::clean(
            input,
            &[
                "window_start",
                "window_end",
                "evidence",
                "source_reference",
                "source_version",
            ],
        )?;
        self.require_report(&w, scope, input.employee_id, REVIEWS_SUBMIT)
            .await?;
        v::period(
            &v::text(&value, "window_start"),
            &v::text(&value, "window_end"),
        )?;
        let saved = store::insert(&mut w, T::Observation, value).await?;
        w.tx.commit().await?;
        Ok(saved)
    }
    pub async fn correct_observation(
        &self,
        scope: &Scope,
        company: i64,
        id: i64,
        input: &ObservationCorrection,
    ) -> Result<Observation> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut w = self
            .begin(
                &mut client,
                scope,
                company,
                REVIEWS_SUBMIT,
                "correct_observation",
            )
            .await?;
        let prior: Observation = store::get(w.tx.sql(), scope, company, T::Observation, id).await?;
        if prior.actor_user_id != scope.user() {
            return Err(PerformanceError::NotAuthor);
        }
        self.require_report(&w, scope, prior.employee_id, REVIEWS_SUBMIT)
            .await?;
        if store::successor(&w, T::Observation, id).await? {
            return Err(PerformanceError::AlreadyCorrected);
        }
        let mut value = v::clean(input, &["evidence", "source_version", "change_reason"])?;
        value["employee_id"] = json!(prior.employee_id);
        value["window_start"] = json!(prior.window_start);
        value["window_end"] = json!(prior.window_end);
        value["source_reference"] = json!(prior.source_reference);
        value["supersedes_id"] = json!(prior.id);
        let saved = store::insert(&mut w, T::Observation, value).await?;
        w.tx.commit().await?;
        Ok(saved)
    }
}
