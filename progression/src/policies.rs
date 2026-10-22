//! Bilimbi Progression's immutable drafts, publication and bounded history.
use crate::{store, validate, *};
use bilibi_tenancy::Scope;
use std::collections::HashMap;

impl Progression {
    pub async fn policies(
        &self,
        scope: &Scope,
        company: i64,
        options: PageOptions,
    ) -> Result<PolicyPage> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company, POLICY_VIEW)
            .await?;
        if options.page <= 0 || ![10, 25, 50, 100].contains(&options.page_size) {
            return Err(ProgressionError::InvalidOptions);
        }
        let total: i64 = client.query_one("SELECT count(*) FROM people_progression_policy_versions WHERE tenant_id=$1 AND company_id=$2", &[&scope.tenant(),&company]).await.map_err(Error::from)?.get(0);
        let page = options
            .page
            .min(((total + options.page_size - 1) / options.page_size).max(1));
        let offset = (page - 1) * options.page_size;
        let rows = store::rows(
            &**client,
            scope,
            company,
            "ORDER BY t.effective_from DESC,t.id DESC LIMIT $3 OFFSET $4",
            &[&options.page_size, &offset],
        )
        .await?;
        Ok(PolicyPage {
            rows,
            total,
            page,
            page_size: options.page_size,
        })
    }
    /// Copies every requirement from the exact published profile version under
    /// the company lock. Callers cannot supply or weaken individual thresholds.
    pub async fn draft(&self, scope: &Scope, company: i64, input: &DraftInput) -> Result<Policy> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut write = self
            .begin(&mut client, scope, company, POLICY_MANAGE, "draft")
            .await?;
        validate::draft(input)?;
        let competence = if let Some(pin) = &input.profile {
            let profile = self
                .published_profile(scope, company, pin.id, pin.version)
                .await?;
            if profile.items.is_empty() {
                return Err(ProgressionError::ProfileUnavailable);
            }
            let skills = self.skills.list_skills(scope, company).await?;
            profile
                .items
                .iter()
                .map(|item| CompetenceRule {
                    code: skills
                        .iter()
                        .find(|s| s.id == item.skill_id)
                        .map(|s| s.code.clone())
                        .unwrap_or_default(),
                    profile_id: pin.id,
                    profile_version: pin.version,
                    skill_id: item.skill_id,
                    required_level: item.required_level,
                })
                .collect()
        } else {
            vec![]
        };
        let rules = Rules {
            competence,
            performance: input.performance.clone(),
        };
        validate::rules(&rules)?;
        let row = store::insert(&mut write, input, rules).await?;
        write.tx.commit().await?;
        Ok(row)
    }
    /// A higher version may correct the same effective day. Future versions do
    /// not displace today's governing policy until their effective date.
    pub async fn publish(&self, scope: &Scope, company: i64, id: i64) -> Result<Policy> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut write = self
            .begin(&mut client, scope, company, POLICY_MANAGE, "publish")
            .await?;
        let row = store::rows(
            write.tx.sql(),
            scope,
            company,
            "AND t.id=$3 FOR UPDATE",
            &[&id],
        )
        .await?
        .pop()
        .ok_or(ProgressionError::NotFound)?;
        if row.status != "draft" {
            return Err(ProgressionError::AlreadyPublished);
        }
        validate::rules(&row.rules)?;
        let mut profiles = HashMap::new();
        for rule in &row.rules.competence {
            let pin = (rule.profile_id, rule.profile_version);
            if let std::collections::hash_map::Entry::Vacant(entry) = profiles.entry(pin) {
                entry.insert(self.published_profile(scope, company, pin.0, pin.1).await?);
            }
            if !profiles[&pin]
                .items
                .iter()
                .any(|i| i.skill_id == rule.skill_id && i.required_level == rule.required_level)
            {
                return Err(ProgressionError::ProfileUnavailable);
            }
        }
        let order = write.tx.sql().query_one("SELECT EXISTS(SELECT 1 FROM people_progression_policy_versions WHERE tenant_id=$1 AND company_id=$2 AND code=$3 AND status='published' AND version >= $4),EXISTS(SELECT 1 FROM people_progression_policy_versions WHERE tenant_id=$1 AND company_id=$2 AND code=$3 AND status='published' AND effective_from > $5::text::date)", &[&scope.tenant(),&company,&row.code,&row.version,&row.effective_from]).await.map_err(Error::from)?;
        if order.get::<_, bool>(0) {
            return Err(ProgressionError::VersionOrder);
        }
        if order.get::<_, bool>(1) {
            return Err(ProgressionError::EffectiveOrder);
        }
        let row = store::publish(&mut write, row).await?;
        write.tx.commit().await?;
        Ok(row)
    }
    async fn published_profile(
        &self,
        scope: &Scope,
        company: i64,
        id: i64,
        version: i32,
    ) -> Result<bilibi_people_skills::ProfileDetail> {
        let detail = self.skills.get_profile(scope, company, id).await?;
        if detail.profile.status != "published" || detail.profile.version != version {
            return Err(ProgressionError::ProfileUnavailable);
        }
        Ok(detail)
    }
}
