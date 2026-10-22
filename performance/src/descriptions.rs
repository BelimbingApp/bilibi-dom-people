//! Bilimbi Performance descriptions and governed KPI definitions.
use crate::{store, tables::Table as T, validate as v, *};
use bilibi_database::Error;
use bilibi_tenancy::Scope;
use serde_json::{json, Value};

impl Performance {
    /// Appends a complete immutable draft with exact position/profile versions.
    pub async fn draft_description(
        &self,
        scope: &Scope,
        company: i64,
        input: &DescriptionInput,
    ) -> Result<Description> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut w = self
            .begin(
                &mut client,
                scope,
                company,
                DESCRIPTIONS_MANAGE,
                "draft_description",
            )
            .await?;
        let mut value = v::clean(
            input,
            &[
                "code",
                "purpose",
                "responsibilities",
                "duties",
                "authority",
                "qualifications",
                "effective_from",
            ],
        )?;
        v::positive("version", input.version.into())?;
        v::positive("position_version", input.position_version.into())?;
        v::positive("position_id", input.position_id)?;
        let first = v::text(&value, "effective_from");
        let last = value["effective_to"].as_str().filter(|s| !s.is_empty());
        v::period(&first, last.unwrap_or(&first))?;
        value["effective_to"] = last.map_or(Value::Null, |s| json!(s));
        value["competency_links"] = json!({"profiles": input.competency_links});
        self.validate_description(
            scope,
            company,
            input.position_id,
            input.position_version,
            &first,
            &value["competency_links"],
        )
        .await?;
        value["status"] = json!("draft");
        let saved = store::insert(&mut w, T::Description, value).await?;
        w.tx.commit().await?;
        Ok(saved)
    }
    /// Publishes only the pinned versions and an inclusive, nonoverlapping interval.
    pub async fn publish_description(
        &self,
        scope: &Scope,
        company: i64,
        id: i64,
    ) -> Result<Description> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut w = self
            .begin(
                &mut client,
                scope,
                company,
                DESCRIPTIONS_MANAGE,
                "publish_description",
            )
            .await?;
        let prior: Description = store::get(w.tx.sql(), scope, company, T::Description, id).await?;
        if prior.status != "draft" {
            return Err(PerformanceError::NotPending);
        }
        self.validate_description(
            scope,
            company,
            prior.position_id,
            prior.position_version,
            &prior.effective_from,
            &prior.competency_links,
        )
        .await?;
        let overlap = w.tx.sql().query_opt(
            "SELECT 1 FROM people_performance_descriptions WHERE tenant_id = $1 AND company_id = $2 AND position_id = $3 AND status = 'published' AND daterange(effective_from, effective_to, '[]') && daterange($4::text::date, $5::text::date, '[]') LIMIT 1",
            &[&scope.tenant(), &company, &prior.position_id, &prior.effective_from, &prior.effective_to],
        ).await.map_err(Error::from)?.is_some();
        if overlap {
            return Err(PerformanceError::OverlappingDescription);
        }
        let saved = store::transition(
            &mut w,
            T::Description,
            id,
            prior,
            store::Transition::PublishDescription,
        )
        .await?;
        w.tx.commit().await?;
        Ok(saved)
    }
    pub(crate) async fn validate_description(
        &self,
        scope: &Scope,
        company: i64,
        position: i64,
        version: i32,
        first: &str,
        links: &Value,
    ) -> Result<()> {
        let position = self
            .position(scope, company, position, v::date(first)?)
            .await?;
        if position.version != Some(version) {
            return Err(PerformanceError::PositionVersionUnavailable);
        }
        let profiles = links
            .get("profiles")
            .and_then(Value::as_array)
            .filter(|p| !p.is_empty())
            .ok_or(PerformanceError::ProfileUnavailable)?;
        for link in profiles {
            let fields = link
                .as_object()
                .ok_or(PerformanceError::ProfileUnavailable)?;
            if fields.len() != 2 || !fields.contains_key("id") || !fields.contains_key("version") {
                return Err(PerformanceError::ProfileUnavailable);
            }
            let id = link["id"]
                .as_i64()
                .filter(|i| *i > 0)
                .ok_or(PerformanceError::ProfileUnavailable)?;
            let version = link["version"]
                .as_i64()
                .filter(|i| *i > 0)
                .ok_or(PerformanceError::ProfileUnavailable)?;
            let profile =
                self.skills
                    .get_profile(scope, company, id)
                    .await
                    .map_err(|e| match e {
                        bilibi_people_skills::SkillsError::Failed(error) => {
                            PerformanceError::from(error)
                        }
                        _ => PerformanceError::ProfileUnavailable,
                    })?;
            if profile.profile.status != "published"
                || i64::from(profile.profile.version) != version
            {
                return Err(PerformanceError::ProfileUnavailable);
            }
        }
        Ok(())
    }
    /// Records provenance and interpretation; calculation_version is never executed.
    pub async fn define_kpi(
        &self,
        scope: &Scope,
        company: i64,
        input: &DefinitionInput,
    ) -> Result<Definition> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut w = self
            .begin(&mut client, scope, company, KPIS_SUBMIT, "define_kpi")
            .await?;
        let value = v::clean(
            input,
            &[
                "code",
                "name",
                "purpose",
                "unit",
                "measure",
                "source_reference",
                "calculation_version",
                "direction",
                "interpretation",
            ],
        )?;
        v::positive("version", input.version.into())?;
        if !["higher", "lower", "band", "rubric"].contains(&v::text(&value, "direction").as_str()) {
            return Err(PerformanceError::Invalid(vec![FieldError::new(
                "direction",
                "is invalid",
            )]));
        }
        if !(0..=8).contains(&input.precision) {
            return Err(PerformanceError::Invalid(vec![FieldError::new(
                "precision",
                if input.precision < 0 {
                    "must be greater than or equal to 0"
                } else {
                    "must be less than or equal to 8"
                },
            )]));
        }
        if value["direction"] == "rubric" && value["rubric"].as_str().is_none_or(str::is_empty) {
            return Err(PerformanceError::RubricRequired);
        }
        let saved = store::insert(&mut w, T::Definition, value).await?;
        w.tx.commit().await?;
        Ok(saved)
    }
}
