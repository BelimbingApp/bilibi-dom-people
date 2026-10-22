//! Bilimbi Progression.explain: compare pinned competence and released reviews.
use crate::{store, *};
use bilibi_people_performance::{PageOptions as ReviewPage, Review};
use bilibi_tenancy::Scope;
use jiff::Timestamp;
use std::collections::HashMap;

impl Progression {
    /// Explains the signed-in working employee only. Each evidence owner keeps
    /// its own capability check; unavailable evidence never becomes success.
    pub async fn explain(&self, scope: &Scope, company: i64) -> Result<Standing> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company, SELF_VIEW).await?;
        let employee = self.workforce.self_employee(scope, company).await?;
        self.workforce
            .employee(scope, company, employee.employee_id)
            .await?
            .require_current()?;
        let today = Timestamp::now()
            .to_zoned(jiff::tz::TimeZone::UTC)
            .date()
            .to_string();
        // DISTINCT ON selects each code independently; version resolves same-day corrections.
        let policies = store::governing(&**client, scope, company, &today).await?;
        if policies.is_empty() {
            return Err(ProgressionError::NoPublishedPolicy);
        }
        let scores = if policies.iter().any(|p| !p.rules.competence.is_empty()) {
            let standing = self.skills.standing(scope, company).await?;
            if standing.employee_id != employee.employee_id {
                return Err(ProgressionError::EmployeeUnavailable);
            }
            standing.scores
        } else {
            vec![]
        };
        let mut profiles = HashMap::new();
        let mut explanations = vec![];
        for policy in policies {
            let mut rules = vec![];
            for rule in &policy.rules.competence {
                // A pinned profile may now be retired. Its immutable content and
                // scale still describe the assessment this policy requires.
                if let std::collections::hash_map::Entry::Vacant(entry) =
                    profiles.entry(rule.profile_id)
                {
                    entry.insert(
                        self.skills
                            .get_profile(scope, company, rule.profile_id)
                            .await?,
                    );
                }
                let profile = &profiles[&rule.profile_id].profile;
                let score = scores.iter().find(|s| s.score.skill_id == rule.skill_id);
                let current = score.filter(|s| {
                    s.state == "current"
                        && s.assessment_profile_id == rule.profile_id
                        && s.assessment_profile_version == rule.profile_version
                        && profile.version == rule.profile_version
                        && s.assessment_scale_id == profile.scale_id
                });
                rules.push(Criterion::Skill {
                    code: rule.code.clone(),
                    status: match current {
                        None => Status::Unknown,
                        Some(s) if s.score.current_level >= rule.required_level => Status::Met,
                        _ => Status::NotMet,
                    },
                    required_level: rule.required_level,
                    observed_level: current.map(|s| s.score.current_level),
                    assessment_id: score.map(|s| s.score.assessment_id),
                });
            }
            if let Some(rule) = &policy.rules.performance {
                let latest = self
                    .latest_review(scope, company, employee.employee_id, rule)
                    .await?;
                let status = match &latest {
                    Some(review) if rule.accepted_outcomes.contains(&review.outcome) => Status::Met,
                    Some(_) => Status::NotMet,
                    None => match rule.missing_evidence {
                        MissingEvidence::Unknown => Status::Unknown,
                        MissingEvidence::NotMet => Status::NotMet,
                    },
                };
                rules.push(Criterion::Performance {
                    code: "performance".into(),
                    status,
                    review_id: latest.as_ref().map(|r| r.id),
                    review_version: latest.as_ref().map(|r| r.version),
                    supersedes_id: latest.as_ref().and_then(|r| r.supersedes_id),
                    period_start: latest.as_ref().map(|r| r.period_start.clone()),
                    period_end: latest.as_ref().map(|r| r.period_end.clone()),
                });
            }
            let status = if rules.iter().any(|r| r.status() == Status::NotMet) {
                Status::NotMet
            } else if rules.iter().any(|r| r.status() == Status::Unknown) {
                Status::Unknown
            } else {
                Status::Met
            };
            explanations.push(Explanation {
                policy,
                rules,
                status,
            });
        }
        // A relink during sibling reads must not return the former employee's explanation.
        if self
            .workforce
            .self_employee(scope, company)
            .await?
            .employee_id
            != employee.employee_id
        {
            return Err(ProgressionError::EmployeeUnavailable);
        }
        self.authorize(&**client, scope, company, SELF_VIEW).await?;
        Ok(Standing {
            employee_id: employee.employee_id,
            explanations,
        })
    }
    async fn latest_review(
        &self,
        scope: &Scope,
        company: i64,
        employee: i64,
        rule: &PerformanceRule,
    ) -> Result<Option<Review>> {
        let mut page = 1;
        let mut latest: Option<Review> = None;
        loop {
            let records = self
                .performance
                .my_records(
                    scope,
                    company,
                    ReviewPage {
                        page,
                        page_size: 100,
                    },
                )
                .await?;
            for detail in records.reviews.rows {
                let review = detail.review;
                if review.employee_id != employee {
                    return Err(ProgressionError::EmployeeUnavailable);
                }
                if rule
                    .periods
                    .iter()
                    .any(|p| review.period_start >= p.start && review.period_end <= p.end)
                    && latest.as_ref().is_none_or(|r| {
                        (&review.period_end, review.version, review.id)
                            > (&r.period_end, r.version, r.id)
                    })
                {
                    latest = Some(review);
                }
            }
            if records.reviews.page * 100 >= records.reviews.total {
                return Ok(latest);
            }
            page += 1;
        }
    }
}
