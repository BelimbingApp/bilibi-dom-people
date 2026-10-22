//! Bilimbi Performance actor-specific reads. Subject filters run before LIMIT.
use crate::{store, tables::Table as T, *};
use bilibi_database::Error;
use bilibi_people_workforce::authorization;
use bilibi_tenancy::Scope;
use tokio_postgres::{types::ToSql, GenericClient};

impl Performance {
    pub async fn reviews(
        &self,
        scope: &Scope,
        company: i64,
        options: PageOptions,
    ) -> Result<Page<Review>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company, VIEW).await?;
        let own = self
            .workforce
            .linked_employee(&**client, scope, company)
            .await?;
        review_page(&**client, scope, company, options,
            "AND t.actor_user_id = $3 AND (t.status = 'released' OR $4::bigint IS NULL OR t.employee_id <> $4)", &[&scope.user(), &own]).await
    }
    pub async fn review(&self, scope: &Scope, company: i64, id: i64) -> Result<ReviewDetail> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company, VIEW).await?;
        let row: Review = store::get(&**client, scope, company, T::Review, id).await?;
        if row.actor_user_id != scope.user()
            && !authorization::allowed(&**client, scope, company, REVIEWS_APPROVE).await
        {
            return Err(PerformanceError::NotFound);
        }
        if row.status != "released"
            && self
                .workforce
                .linked_employee(&**client, scope, company)
                .await?
                == Some(row.employee_id)
        {
            return Err(PerformanceError::NotFound);
        }
        detail(&**client, scope, company, row).await
    }
    pub async fn my_records(
        &self,
        scope: &Scope,
        company: i64,
        options: PageOptions,
    ) -> Result<MyRecords> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company, SELF_VIEW).await?;
        let own = self.workforce.self_employee(scope, company).await?;
        let page = review_page(
            &**client,
            scope,
            company,
            options,
            "AND t.employee_id = $3 AND t.status = 'released'",
            &[&own.employee_id],
        )
        .await?;
        let mut rows = Vec::new();
        for row in page.rows {
            rows.push(detail(&**client, scope, company, row).await?);
        }
        let targets = store::query(&**client, scope, company, T::Target,
            "AND t.employee_id = $3 AND t.status = 'published' AND NOT t.confidential ORDER BY t.id DESC LIMIT 100", &[&own.employee_id]).await?;
        Ok(MyRecords {
            reviews: Page {
                rows,
                total: page.total,
                page: page.page,
                page_size: page.page_size,
            },
            targets,
        })
    }
    pub async fn planning_records(&self, scope: &Scope, company: i64) -> Result<PlanningRecords> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company, VIEW).await?;
        let own = self
            .workforce
            .linked_employee(&**client, scope, company)
            .await?;
        let definitions = if authorization::allowed(&**client, scope, company, KPIS_SUBMIT).await {
            store::query(
                &**client,
                scope,
                company,
                T::Definition,
                "ORDER BY t.id DESC LIMIT 100",
                &[],
            )
            .await?
        } else {
            vec![]
        };
        let descriptions =
            if authorization::allowed(&**client, scope, company, DESCRIPTIONS_MANAGE).await {
                store::query(
                    &**client,
                    scope,
                    company,
                    T::Description,
                    "ORDER BY t.id DESC LIMIT 100",
                    &[],
                )
                .await?
            } else {
                store::query(
                    &**client,
                    scope,
                    company,
                    T::Description,
                    "AND t.status = 'published' ORDER BY t.id DESC LIMIT 100",
                    &[],
                )
                .await?
            };
        let targets = if authorization::allowed(&**client, scope, company, KPIS_REVIEW).await
            || authorization::allowed(&**client, scope, company, KPIS_APPROVE).await
        {
            store::query(
                &**client,
                scope,
                company,
                T::Target,
                "AND ($3::bigint IS NULL OR t.employee_id <> $3) ORDER BY t.id DESC LIMIT 100",
                &[&own],
            )
            .await?
        } else if authorization::allowed(&**client, scope, company, KPIS_SUBMIT).await {
            store::query(&**client, scope, company, T::Target, "AND ($3::bigint IS NULL OR t.employee_id <> $3) AND t.actor_user_id = $4 ORDER BY t.id DESC LIMIT 100", &[&own, &scope.user()]).await?
        } else {
            vec![]
        };
        let drafts = if authorization::allowed(&**client, scope, company, REVIEWS_APPROVE).await {
            store::query(&**client, scope, company, T::Review, "AND ($3::bigint IS NULL OR t.employee_id <> $3) AND t.status = 'draft' ORDER BY t.id DESC LIMIT 100", &[&own]).await?
        } else {
            vec![]
        };
        let observations = store::query(&**client, scope, company, T::Observation,
            "AND ($3::bigint IS NULL OR t.employee_id <> $3) AND t.actor_user_id = $4 ORDER BY t.id DESC LIMIT 100", &[&own, &scope.user()]).await?;
        let prior_reviews = store::query(&**client, scope, company, T::Review,
            "AND ($3::bigint IS NULL OR t.employee_id <> $3) AND t.actor_user_id = $4 AND t.status = 'released' ORDER BY t.id DESC LIMIT 100", &[&own, &scope.user()]).await?;
        Ok(PlanningRecords {
            definitions,
            descriptions,
            targets,
            observations,
            drafts,
            prior_reviews,
        })
    }
    pub async fn planning_choices(&self, scope: &Scope, company: i64) -> Result<PlanningChoices> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company, VIEW).await?;
        let people = self
            .workforce
            .employees(scope, company)
            .await?
            .require_current()?;
        let own = match self.workforce.self_employee(scope, company).await {
            Ok(own) => Some(own.employee_id.to_string()),
            Err(bilibi_people_workforce::WorkforceError::NotLinked) => None,
            Err(error) => return Err(error.into()),
        };
        let id = |text: &str| {
            text.parse::<i64>()
                .map_err(|_| PerformanceError::Failed(Error::Internal))
        };
        let mut employees = Vec::new();
        let mut names = std::collections::BTreeMap::new();
        for employee in people {
            let employee_id = id(&employee.reference.stable_id)?;
            if own.as_ref().is_some_and(|own| {
                employee
                    .supervisor_reference
                    .as_ref()
                    .is_some_and(|s| s.stable_id == *own)
            }) {
                employees.push(EmployeeChoice {
                    id: employee_id,
                    name: employee.display_name.clone(),
                });
            }
            names.insert(employee_id, employee.display_name);
        }
        let report_ids: Vec<i64> = employees.iter().map(|e| e.id).collect();
        let targets = if authorization::allowed(&**client, scope, company, KPIS_SUBMIT).await
            || authorization::allowed(&**client, scope, company, REVIEWS_SUBMIT).await
        {
            store::query(&**client, scope, company, T::Target,
                "AND t.status = 'published' AND NOT t.confidential AND t.employee_id = ANY($3::bigint[]) ORDER BY t.id DESC LIMIT 100", &[&report_ids]).await?
        } else {
            vec![]
        };
        let descriptions = store::query(
            &**client,
            scope,
            company,
            T::Description,
            "AND t.status = 'published' ORDER BY t.id DESC LIMIT 100",
            &[],
        )
        .await?;
        let today = jiff::Timestamp::now()
            .to_zoned(jiff::tz::TimeZone::UTC)
            .date();
        let positions = self
            .organisation
            .positions(scope, company, today, 1, 100)
            .await
            .map_err(|_| PerformanceError::PositionUnavailable)?
            .into_iter()
            .map(|p| {
                Ok(PositionChoice {
                    id: id(&p.reference.stable_id)?,
                    name: p.title.unwrap_or(p.code),
                    version: p.version,
                })
            })
            .collect::<Result<_>>()?;
        let profiles = self
            .skills
            .list_profiles(scope, company)
            .await
            .map_err(|_| PerformanceError::ProfileUnavailable)?
            .into_iter()
            .filter(|p| p.status == "published")
            .map(|p| ProfileChoice {
                id: p.id,
                code: p.code,
                name: p.name,
                version: p.version,
            })
            .collect();
        Ok(PlanningChoices {
            employees,
            descriptions,
            targets,
            positions,
            profiles,
            names,
        })
    }
}
async fn review_page(
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company: i64,
    options: PageOptions,
    filter: &str,
    args: &[&(dyn ToSql + Sync)],
) -> Result<Page<Review>> {
    if options.page < 1 || ![10, 25, 50, 100].contains(&options.page_size) {
        return Err(PerformanceError::InvalidOptions);
    }
    let tenant = scope.tenant();
    let mut parameters: Vec<&(dyn ToSql + Sync)> = vec![&tenant, &company];
    parameters.extend_from_slice(args);
    let total: i64 = client.query_one(&format!("SELECT count(*) FROM people_performance_appraisals t WHERE t.tenant_id = $1 AND t.company_id = $2 {filter}"), &parameters).await.map_err(Error::from)?.get(0);
    let page = options
        .page
        .min(((total + options.page_size - 1) / options.page_size).max(1));
    let limit = options.page_size;
    let offset = (page - 1) * limit;
    let n = args.len() + 3;
    let tail = format!("{filter} ORDER BY t.id DESC LIMIT ${n} OFFSET ${}", n + 1);
    let mut args = args.to_vec();
    args.extend([&limit as &(dyn ToSql + Sync), &offset]);
    let rows = store::query(client, scope, company, T::Review, &tail, &args).await?;
    Ok(Page {
        rows,
        total,
        page,
        page_size: limit,
    })
}
async fn detail(
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company: i64,
    review: Review,
) -> Result<ReviewDetail> {
    let observations = store::query(client, scope, company, T::Observation,
        "AND EXISTS(SELECT 1 FROM people_performance_review_evidence p WHERE p.tenant_id = t.tenant_id AND p.company_id = t.company_id AND p.review_id = $3 AND p.observation_id = t.id) ORDER BY t.id", &[&review.id]).await?;
    let targets = store::query(client, scope, company, T::Target,
        "AND EXISTS(SELECT 1 FROM people_performance_review_targets p WHERE p.tenant_id = t.tenant_id AND p.company_id = t.company_id AND p.review_id = $3 AND p.target_id = t.id) ORDER BY t.id", &[&review.id]).await?;
    let responses = store::query(
        client,
        scope,
        company,
        T::Response,
        "AND t.review_id = $3 ORDER BY t.id",
        &[&review.id],
    )
    .await?;
    Ok(ReviewDetail {
        review,
        observations,
        targets,
        responses,
    })
}
