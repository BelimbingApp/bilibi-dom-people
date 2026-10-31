//! Mirrors Bilimbi `Training.Governance`: requests, team plans and budgets.
//! Public operations live on Training; validation and persistence stay private.
use crate::{governance_store as store, governance_store::Table, *};
use bilibi_database::MutationTransaction;
use bilibi_module_registry::{Contribution, Provenance};
use bilibi_settings::{Definition, Editor, ScopeKind, Setting, Validator, ValueType, DEFINITIONS};
use rust_decimal::Decimal;
use serde_json::{json, Value};
use std::{collections::BTreeMap, str::FromStr};

pub const CURRENCIES_KEY: &str = "people.training.currencies";
pub const REQUESTS_SUBMIT: &str = "people.training.requests.submit";
pub const REQUESTS_RECOMMEND: &str = "people.training.requests.recommend";
pub const REQUESTS_REVIEW: &str = "people.training.requests.review";
pub const REQUESTS_APPROVE: &str = "people.training.requests.approve";
pub const REQUESTS_VIEW: &str = "people.training.requests.view";
pub const PLANS_SUBMIT: &str = "people.training.plans.submit";
pub const PLANS_APPROVE: &str = "people.training.plans.approve";
pub const PLANS_VIEW: &str = "people.training.plans.view";
pub const BUDGETS_VIEW: &str = "people.training.budgets.view";
pub const BUDGETS_MANAGE: &str = "people.training.budgets.manage";

const PROVENANCE: Provenance = Provenance {
    module: MODULE,
    source: file!(),
    line: line!(),
};
inventory::submit! {
    Contribution::new(bilibi_authz::VOCABULARY,"people/training.governance_vocabulary",PROVENANCE,&bilibi_authz::VocabularyContribution {
        domains:&["people"], verbs:&["submit","recommend","review","approve"],
        capabilities:&[
            bilibi_authz::CapabilityDefinition::tenant(REQUESTS_SUBMIT),bilibi_authz::CapabilityDefinition::tenant(REQUESTS_RECOMMEND),
            bilibi_authz::CapabilityDefinition::tenant(REQUESTS_REVIEW),bilibi_authz::CapabilityDefinition::tenant(REQUESTS_APPROVE),
            bilibi_authz::CapabilityDefinition::tenant(REQUESTS_VIEW),bilibi_authz::CapabilityDefinition::tenant(PLANS_SUBMIT),
            bilibi_authz::CapabilityDefinition::tenant(PLANS_APPROVE),bilibi_authz::CapabilityDefinition::tenant(PLANS_VIEW),
            bilibi_authz::CapabilityDefinition::tenant(BUDGETS_VIEW),bilibi_authz::CapabilityDefinition::tenant(BUDGETS_MANAGE),
        ], roles:&[],system_principals:&[],
    })
}
fn currencies_valid(value: &Value) -> bool {
    value.as_array().is_some_and(|values| {
        values
            .iter()
            .all(|v| v.as_str().is_some_and(currency_shaped))
    })
}
fn currency_shaped(code: &str) -> bool {
    code.len() == 3 && code.bytes().all(|c| c.is_ascii_uppercase())
}
inventory::submit! {
    Contribution::new(DEFINITIONS,CURRENCIES_KEY,PROVENANCE,&Setting::Definition(Definition {
        value_type:ValueType::Array,scopes:&[ScopeKind::Company],default:||json!([]),nullable:false,encrypted:false,minimum:None,maximum:None,
        validator:Some(Validator{accepts:currencies_valid,message:"Enter currency codes as three uppercase letters, separated by commas."}),
        editor:Some(Editor{group:"People",label:"Learning currencies",help:"Currency codes accepted for this company's learning budgets and requests.",capability:Some(BUDGETS_MANAGE),reveal_duration_ms:5_000}),
    }))
}

fn values<T: serde::Serialize>(input: &T) -> Result<Value> {
    serde_json::to_value(input).map_err(|_| TrainingError::Failed(Error::Internal))
}
fn text(value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 4000 {
        return Err(TrainingError::InvalidLearningValues);
    }
    Ok(value.to_owned())
}
fn fields(data: &mut Value, names: &[&str]) -> Result<()> {
    for name in names {
        data[*name] = json!(text(
            data[*name]
                .as_str()
                .ok_or(TrainingError::InvalidLearningValues)?
        )?);
    }
    Ok(())
}
fn reason(value: &str) -> Result<String> {
    if value.trim().is_empty() || value.len() > 4000 {
        return Err(TrainingError::ReasonRequired);
    }
    Ok(value.trim().to_owned())
}
fn date(value: &str) -> Result<Date> {
    let parsed = value
        .parse::<Date>()
        .map_err(|_| TrainingError::InvalidLearningValues)?;
    if parsed.to_string() != value {
        return Err(TrainingError::InvalidLearningValues);
    }
    Ok(parsed)
}
fn period(from: &str, to: &str) -> Result<()> {
    if date(to)? < date(from)? {
        return Err(TrainingError::InvalidPeriod);
    }
    Ok(())
}
fn money(value: &str) -> Result<Decimal> {
    let amount = Decimal::from_str(value).map_err(|_| TrainingError::InvalidLearningValues)?;
    if amount < Decimal::ZERO
        || amount.scale() > 4
        || amount >= Decimal::from(100_000_000_000_000i64)
    {
        return Err(TrainingError::InvalidLearningValues);
    }
    Ok(amount)
}
fn request_cap(audience: LearningAudience) -> &'static str {
    match audience {
        LearningAudience::SelfService => REQUESTS_SUBMIT,
        LearningAudience::Team => REQUESTS_RECOMMEND,
        LearningAudience::Hr => REQUESTS_VIEW,
    }
}
fn plan_cap(audience: LearningAudience) -> Result<&'static str> {
    match audience {
        LearningAudience::SelfService => Err(TrainingError::Unauthorized),
        LearningAudience::Team => Ok(PLANS_SUBMIT),
        LearningAudience::Hr => Ok(PLANS_VIEW),
    }
}
fn step(status: &str, action: &str) -> Result<(&'static str, &'static str, &'static str)> {
    match (status, action) {
        ("draft", "submit") => Ok(("draft", "pending_hod", REQUESTS_SUBMIT)),
        ("pending_hod", "recommend") => Ok(("pending_hod", "pending_hr", REQUESTS_RECOMMEND)),
        ("pending_hr", "review") => Ok(("pending_hr", "pending_approval", REQUESTS_REVIEW)),
        ("pending_approval", "approve") => Ok(("pending_approval", "approved", REQUESTS_APPROVE)),
        ("pending_hod", "reject") => Ok(("pending_hod", "rejected", REQUESTS_RECOMMEND)),
        ("pending_hr", "reject") => Ok(("pending_hr", "rejected", REQUESTS_REVIEW)),
        ("pending_approval", "reject") => Ok(("pending_approval", "rejected", REQUESTS_APPROVE)),
        ("draft", "cancel") => Ok(("draft", "cancelled", REQUESTS_SUBMIT)),
        ("pending_hod", "cancel") => Ok(("pending_hod", "cancelled", REQUESTS_SUBMIT)),
        ("pending_hr", "cancel") => Ok(("pending_hr", "cancelled", REQUESTS_SUBMIT)),
        ("pending_approval", "cancel") => Ok(("pending_approval", "cancelled", REQUESTS_SUBMIT)),
        _ => Err(TrainingError::InvalidTransition),
    }
}

impl Training {
    /// Enabled currencies, in operator order (`budgets.view`); empty by default.
    pub async fn learning_currencies(&self, scope: &Scope, company: i64) -> Result<Vec<String>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company, BUDGETS_VIEW)
            .await?;
        self.governance_currencies(&**client, scope, company).await
    }
    /// Replaces the allowed codes (`budgets.manage`); duplicate codes collapse.
    pub async fn put_learning_currencies(
        &self,
        scope: &Scope,
        company: i64,
        codes: &[String],
    ) -> Result<Vec<String>> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company, BUDGETS_MANAGE)
            .await?;
        if !codes.iter().all(|c| currency_shaped(c)) {
            return Err(TrainingError::InvalidCurrencies);
        }
        let mut unique = Vec::new();
        for code in codes {
            if !unique.contains(code) {
                unique.push(code.clone());
            }
        }
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.training.put_learning_currencies"),
            )
            .await?;
        writable(tx.sql(), scope, company).await?;
        self.authorize(tx.sql(), scope, company, BUDGETS_MANAGE)
            .await?;
        self.settings
            .put(
                &mut tx,
                scope,
                Target::Company(OrganisationId::new(company)?),
                CURRENCIES_KEY,
                json!(unique),
            )
            .await?;
        tx.commit().await?;
        Ok(unique)
    }
    async fn governance_currencies(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company: i64,
    ) -> Result<Vec<String>> {
        let resolved = self
            .settings
            .get(
                client,
                scope,
                Target::Company(OrganisationId::new(company)?),
                CURRENCIES_KEY,
            )
            .await?;
        store::decode(resolved.value)
    }
    async fn currency_enabled(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company: i64,
        currency: &str,
    ) -> Result<()> {
        if !self
            .governance_currencies(client, scope, company)
            .await?
            .iter()
            .any(|c| c == currency)
        {
            return Err(TrainingError::CurrencyUnavailable);
        }
        Ok(())
    }
    /// Creates an immutable effective budget (`budgets.manage`).
    pub async fn create_budget_policy(
        &self,
        scope: &Scope,
        company: i64,
        input: &NewBudgetPolicy,
    ) -> Result<BudgetPolicy> {
        self.put_budget_policy(scope, company, None, input).await
    }
    /// Corrects the active policy without rewriting it. Its currency is retained.
    pub async fn supersede_budget_policy(
        &self,
        scope: &Scope,
        company: i64,
        id: i64,
        input: &NewBudgetPolicy,
    ) -> Result<BudgetPolicy> {
        self.put_budget_policy(scope, company, Some(id), input)
            .await
    }
    async fn put_budget_policy(
        &self,
        scope: &Scope,
        company: i64,
        prior_id: Option<i64>,
        input: &NewBudgetPolicy,
    ) -> Result<BudgetPolicy> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company, BUDGETS_MANAGE)
            .await?;
        let mut data = values(input)?;
        fields(&mut data, &["reason"])?;
        period(&input.effective_from, &input.effective_to)?;
        let amount = money(&input.amount)?;
        data["amount"] = json!(amount.to_string());
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.training.put_budget_policy"),
            )
            .await?;
        writable(tx.sql(), scope, company).await?;
        self.authorize(tx.sql(), scope, company, BUDGETS_MANAGE)
            .await?;
        store::lock(tx.sql(), scope, company).await?;
        let currency = if let Some(id) = prior_id {
            let prior: BudgetPolicy =
                store::fetch(tx.sql(), scope, company, Table::Budget, id).await?;
            let successors: Vec<BudgetPolicy> = store::list(
                tx.sql(),
                scope,
                company,
                Table::Budget,
                "AND r.supersedes_id=$3",
                &[&id],
            )
            .await?;
            if !successors.is_empty() {
                return Err(TrainingError::SupersededBudget);
            }
            let outside=tx.sql().query_one("SELECT EXISTS(SELECT 1 FROM people_training_needs WHERE tenant_id=$1 AND company_id=$2 AND status='approved' AND currency=$3 AND proposed_on BETWEEN $4::text::date AND $5::text::date AND proposed_on NOT BETWEEN $6::text::date AND $7::text::date)",&[&scope.tenant(),&company,&prior.currency,&prior.effective_from,&prior.effective_to,&input.effective_from,&input.effective_to]).await.map_err(Error::from)?.get::<_,bool>(0);
            if outside {
                return Err(TrainingError::CommitmentsOutsidePeriod);
            }
            prior.currency
        } else {
            text(&input.currency)?
        };
        self.currency_enabled(tx.sql(), scope, company, &currency)
            .await?;
        let overlap=tx.sql().query_one("SELECT EXISTS(SELECT 1 FROM people_training_budget_policies r WHERE r.tenant_id=$1 AND r.company_id=$2 AND r.currency=$3 AND r.effective_from<=$5::text::date AND r.effective_to>=$4::text::date AND ($6::bigint IS NULL OR r.id<>$6) AND NOT EXISTS(SELECT 1 FROM people_training_budget_policies s WHERE s.tenant_id=$1 AND s.company_id=$2 AND s.supersedes_id=r.id))",&[&scope.tenant(),&company,&currency,&input.effective_from,&input.effective_to,&prior_id]).await.map_err(Error::from)?.get::<_,bool>(0);
        if overlap {
            return Err(TrainingError::OverlappingPolicy);
        }
        let committed = spent(
            tx.sql(),
            scope,
            company,
            &currency,
            &input.effective_from,
            &input.effective_to,
        )
        .await?;
        if committed > amount {
            return Err(TrainingError::BudgetBelowCommitments);
        }
        data["currency"] = json!(currency);
        data["supersedes_id"] = json!(prior_id);
        let result = store::insert(&mut tx, scope, company, Table::Budget, data).await?;
        tx.commit().await?;
        Ok(result)
    }
    /// All policy versions, with successor, committed and remaining amounts.
    pub async fn learning_budgets(
        &self,
        scope: &Scope,
        company: i64,
    ) -> Result<Vec<LearningBudget>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company, BUDGETS_VIEW)
            .await?;
        // One SQL snapshot keeps amount and totals consistent with concurrent approvals.
        let rows=client.query("SELECT to_jsonb(p) || jsonb_build_object('amount',p.amount::text,'superseded_by',s.id,'committed',c.amount::text,'remaining',(p.amount-c.amount)::text) AS value FROM people_training_budget_policies p LEFT JOIN people_training_budget_policies s ON s.tenant_id=p.tenant_id AND s.company_id=p.company_id AND s.supersedes_id=p.id CROSS JOIN LATERAL (SELECT COALESCE(sum(r.approved_cost),0.0000) AS amount FROM people_training_needs r WHERE r.tenant_id=p.tenant_id AND r.company_id=p.company_id AND r.currency=p.currency AND r.status='approved' AND r.proposed_on BETWEEN p.effective_from AND p.effective_to) c WHERE p.tenant_id=$1 AND p.company_id=$2 ORDER BY p.effective_from DESC,p.id DESC",&[&scope.tenant(),&company]).await.map_err(Error::from)?;
        rows.iter().map(|r| store::decode(r.get("value"))).collect()
    }
    /// Creates the current signed-in employee's draft and its permanent create decision.
    pub async fn create_learning_request(
        &self,
        scope: &Scope,
        company: i64,
        input: &NewLearningRequest,
    ) -> Result<LearningRequest> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company, REQUESTS_SUBMIT)
            .await?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.training.create_learning_request"),
            )
            .await?;
        let row = self
            .create_learning_request_in(&mut tx, scope, company, input)
            .await?;
        tx.commit().await?;
        Ok(row)
    }
    pub(super) async fn create_learning_request_in(
        &self,
        tx: &mut MutationTransaction<'_>,
        scope: &Scope,
        company: i64,
        input: &NewLearningRequest,
    ) -> Result<LearningRequest> {
        let mut data = values(input)?;
        fields(
            &mut data,
            &["need", "objective", "expected_result", "currency"],
        )?;
        date(&input.proposed_on)?;
        let cost = money(&input.estimated_cost)?;
        data["estimated_cost"] = json!(cost.to_string());
        writable(tx.sql(), scope, company).await?;
        self.authorize(tx.sql(), scope, company, REQUESTS_SUBMIT)
            .await?;
        let me = self
            .learning_self(tx, scope, company, REQUESTS_SUBMIT)
            .await?;
        if let Some(id) = input.course_id {
            let course:Option<bool>=tx.sql().query_opt("SELECT active FROM people_training_courses WHERE tenant_id=$1 AND company_id=$2 AND id=$3",&[&scope.tenant(),&company,&id]).await.map_err(Error::from)?.map(|r|r.get(0));
            if course != Some(true) {
                return Err(TrainingError::CourseUnavailable);
            }
        }
        self.currency_enabled(
            tx.sql(),
            scope,
            company,
            data["currency"]
                .as_str()
                .ok_or(TrainingError::InvalidLearningValues)?,
        )
        .await?;
        data["employee_id"] = json!(me);
        data["status"] = json!("draft");
        let row: LearningRequest = store::insert(tx, scope, company, Table::Request, data).await?;
        request_decision(
            tx, scope, company, row.id, "create", "new", "draft", "Created",
        )
        .await?;
        Ok(row)
    }
    /// Own drafts and decisions; submitted direct-report requests; or HR's submitted register.
    pub async fn learning_requests(
        &self,
        scope: &Scope,
        company: i64,
        audience: LearningAudience,
    ) -> Result<Vec<LearningRequest>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company, request_cap(audience))
            .await?;
        drop(client);
        let ids = self.learning_audience(scope, company, audience).await?;
        let client = self.db.get().await.map_err(Error::from)?;
        store::list(&**client,scope,company,Table::Request,"AND ($3::bigint[] IS NULL OR r.employee_id=ANY($3)) AND ($4 OR EXISTS(SELECT 1 FROM people_training_need_decisions d WHERE d.tenant_id=r.tenant_id AND d.company_id=r.company_id AND d.request_id=r.id AND d.action='submit')) ORDER BY r.id DESC",&[&ids,&(audience==LearningAudience::SelfService)]).await
    }
    /// Reads one request through the same audience boundary as the list.
    pub async fn learning_request(
        &self,
        scope: &Scope,
        company: i64,
        id: i64,
        audience: LearningAudience,
    ) -> Result<LearningRequest> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company, request_cap(audience))
            .await?;
        drop(client);
        let ids = self.learning_audience(scope, company, audience).await?;
        let client = self.db.get().await.map_err(Error::from)?;
        let mut rows:Vec<LearningRequest>=store::list(&**client,scope,company,Table::Request,"AND r.id=$3 AND ($4::bigint[] IS NULL OR r.employee_id=ANY($4)) AND ($5 OR EXISTS(SELECT 1 FROM people_training_need_decisions d WHERE d.tenant_id=r.tenant_id AND d.company_id=r.company_id AND d.request_id=r.id AND d.action='submit'))",&[&id,&ids,&(audience==LearningAudience::SelfService)]).await?;
        rows.pop().ok_or(TrainingError::NotFound)
    }
    /// Advances or refuses a request under its current state and budget lock.
    pub async fn decide_learning_request(
        &self,
        scope: &Scope,
        company: i64,
        id: i64,
        action: &str,
        explanation: &str,
    ) -> Result<LearningRequest> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut authorized = false;
        for cap in [
            REQUESTS_SUBMIT,
            REQUESTS_RECOMMEND,
            REQUESTS_REVIEW,
            REQUESTS_APPROVE,
        ] {
            if self.authorize(&**client, scope, company, cap).await.is_ok() {
                authorized = true;
                break;
            }
        }
        if !authorized {
            return Err(TrainingError::Unauthorized);
        }
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.training.decide_learning_request"),
            )
            .await?;
        let updated = self
            .decide_learning_request_in(&mut tx, scope, company, id, action, explanation)
            .await?;
        tx.commit().await?;
        Ok(updated)
    }
    pub(super) async fn decide_learning_request_in(
        &self,
        tx: &mut MutationTransaction<'_>,
        scope: &Scope,
        company: i64,
        id: i64,
        action: &str,
        explanation: &str,
    ) -> Result<LearningRequest> {
        let explanation = reason(explanation)?;
        writable(tx.sql(), scope, company).await?;
        store::lock(tx.sql(), scope, company).await?;
        let row: LearningRequest =
            store::fetch(tx.sql(), scope, company, Table::Request, id).await?;
        let (from, to, cap) = step(&row.status, action)?;
        self.authorize(tx.sql(), scope, company, cap).await?;
        if cap == REQUESTS_SUBMIT {
            let me = self.learning_self(tx, scope, company, cap).await?;
            if me != row.employee_id {
                return Err(TrainingError::NotOwner);
            }
        } else if cap == REQUESTS_RECOMMEND {
            let me = self.learning_self(tx, scope, company, cap).await?;
            if !self
                .learning_team(scope, company, me)
                .await?
                .contains(&row.employee_id)
            {
                return Err(TrainingError::OutsideTeam);
            }
        } else {
            self.learning_independent(tx, scope, company, row.actor_user_id, row.employee_id, cap)
                .await?;
        }
        self.learning_current(tx, scope, company, row.employee_id)
            .await?;
        let budget = if to == "approved" {
            self.currency_enabled(tx.sql(), scope, company, &row.currency)
                .await?;
            let policies:Vec<BudgetPolicy>=store::list(tx.sql(),scope,company,Table::Budget,"AND r.currency=$3 AND $4::text::date BETWEEN r.effective_from AND r.effective_to AND NOT EXISTS(SELECT 1 FROM people_training_budget_policies s WHERE s.tenant_id=r.tenant_id AND s.company_id=r.company_id AND s.supersedes_id=r.id)",&[&row.currency,&row.proposed_on]).await?;
            let policy = policies.first().ok_or(TrainingError::BudgetUnavailable)?;
            let committed = spent(
                tx.sql(),
                scope,
                company,
                &row.currency,
                &policy.effective_from,
                &policy.effective_to,
            )
            .await?;
            if committed + money(&row.estimated_cost)? > money(&policy.amount)? {
                return Err(TrainingError::BudgetExceeded);
            }
            Some((policy.id, row.estimated_cost.clone()))
        } else {
            None
        };
        let updated = store::transition(tx, scope, company, Table::Request, id, to, budget).await?;
        request_decision(tx, scope, company, id, action, from, to, &explanation).await?;
        Ok(updated)
    }
    /// Creates a manager-owned draft with 1–300 immutable items (`plans.submit`).
    pub async fn create_learning_plan(
        &self,
        scope: &Scope,
        company: i64,
        input: &NewLearningPlan,
        items: &[NewLearningPlanItem],
    ) -> Result<LearningPlan> {
        self.put_learning_plan(scope, company, None, input, items)
            .await
    }
    /// Creates a draft amendment; approving it supersedes the approved predecessor.
    pub async fn amend_learning_plan(
        &self,
        scope: &Scope,
        company: i64,
        id: i64,
        input: &NewLearningPlan,
        items: &[NewLearningPlanItem],
    ) -> Result<LearningPlan> {
        self.put_learning_plan(scope, company, Some(id), input, items)
            .await
    }
    async fn put_learning_plan(
        &self,
        scope: &Scope,
        company: i64,
        prior_id: Option<i64>,
        input: &NewLearningPlan,
        items: &[NewLearningPlanItem],
    ) -> Result<LearningPlan> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company, PLANS_SUBMIT)
            .await?;
        let mut data = values(input)?;
        fields(&mut data, &["objectives", "reason"])?;
        period(&input.period_start, &input.period_end)?;
        if items.is_empty() || items.len() > 300 {
            return Err(TrainingError::ItemsRequired);
        }
        let mut valid_items = Vec::with_capacity(items.len());
        for item in items {
            let mut data = values(item)?;
            fields(
                &mut data,
                &[
                    "need",
                    "expected_result",
                    "target_cohort",
                    "responsible_owner",
                    "intended_timing",
                    "evaluation_approach",
                ],
            )?;
            valid_items.push(data);
        }
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.training.put_learning_plan"),
            )
            .await?;
        writable(tx.sql(), scope, company).await?;
        self.authorize(tx.sql(), scope, company, PLANS_SUBMIT)
            .await?;
        let me = self
            .learning_self(&tx, scope, company, PLANS_SUBMIT)
            .await?;
        let team = self.learning_team(scope, company, me).await?;
        if team.is_empty() {
            return Err(TrainingError::OutsideTeam);
        }
        store::lock(tx.sql(), scope, company).await?;
        let (key, version) = if let Some(id) = prior_id {
            let prior: LearningPlan =
                store::fetch(tx.sql(), scope, company, Table::Plan, id).await?;
            if prior.actor_user_id != scope.user() || prior.manager_employee_id != me {
                return Err(TrainingError::NotOwner);
            }
            if prior.status != "approved" {
                return Err(TrainingError::InvalidTransition);
            }
            let highest:i32=tx.sql().query_one("SELECT max(version) FROM people_training_team_plans WHERE tenant_id=$1 AND company_id=$2 AND plan_key=$3",&[&scope.tenant(),&company,&prior.plan_key]).await.map_err(Error::from)?.get(0);
            (
                prior.plan_key,
                highest
                    .checked_add(1)
                    .ok_or(TrainingError::InvalidLearningValues)?,
            )
        } else {
            (
                tx.sql()
                    .query_one("SELECT gen_random_uuid()::text", &[])
                    .await
                    .map_err(Error::from)?
                    .get::<_, String>(0),
                1,
            )
        };
        for item in items {
            if let Some(id) = item.request_id {
                let request: LearningRequest =
                    store::fetch(tx.sql(), scope, company, Table::Request, id).await?;
                if !team.contains(&request.employee_id) {
                    return Err(TrainingError::OutsideTeam);
                }
                if request.status != "approved" {
                    return Err(TrainingError::RequestUnapproved);
                }
            }
        }
        data["manager_employee_id"] = json!(me);
        data["version"] = json!(version);
        data["plan_key"] = json!(key);
        data["status"] = json!("draft");
        data["prior_plan_id"] = json!(prior_id);
        let mut row: LearningPlan =
            store::insert(&mut tx, scope, company, Table::Plan, data).await?;
        for mut item in valid_items {
            item["plan_id"] = json!(row.id);
            row.items
                .push(store::insert(&mut tx, scope, company, Table::Item, item).await?);
        }
        plan_decision(
            &mut tx,
            scope,
            company,
            row.id,
            if prior_id.is_some() {
                "amend"
            } else {
                "create"
            },
            "new",
            "draft",
            &row.reason,
        )
        .await?;
        tx.commit().await?;
        Ok(row)
    }
    /// Own plans including drafts, or HR's submitted plans (`plans.view`).
    pub async fn learning_plans(
        &self,
        scope: &Scope,
        company: i64,
        audience: LearningAudience,
    ) -> Result<Vec<LearningPlan>> {
        let cap = plan_cap(audience)?;
        let client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company, cap).await?;
        let manager = if audience == LearningAudience::Team {
            Some(
                self.workforce
                    .self_employee(scope, company)
                    .await
                    .map_err(learning_employee_error)?
                    .employee_id,
            )
        } else {
            None
        };
        let mut rows:Vec<LearningPlan>=store::list(&**client,scope,company,Table::Plan,"AND ($3::bigint IS NULL OR r.manager_employee_id=$3) AND ($4 OR EXISTS(SELECT 1 FROM people_training_plan_decisions d WHERE d.tenant_id=r.tenant_id AND d.company_id=r.company_id AND d.plan_id=r.id AND d.action='submit')) ORDER BY r.id DESC",&[&manager,&(audience==LearningAudience::Team)]).await?;
        let ids: Vec<_> = rows.iter().map(|r| r.id).collect();
        let items: Vec<LearningPlanItem> = store::list(
            &**client,
            scope,
            company,
            Table::Item,
            "AND r.plan_id=ANY($3) ORDER BY r.id",
            &[&ids],
        )
        .await?;
        let mut by_plan: BTreeMap<i64, Vec<LearningPlanItem>> = BTreeMap::new();
        for item in items {
            by_plan.entry(item.plan_id).or_default().push(item);
        }
        for row in &mut rows {
            row.items = by_plan.remove(&row.id).unwrap_or_default();
        }
        Ok(rows)
    }
    /// Submit/cancel as the owner, or independently approve/reject a submitted plan.
    pub async fn decide_learning_plan(
        &self,
        scope: &Scope,
        company: i64,
        id: i64,
        action: &str,
        explanation: &str,
    ) -> Result<LearningPlan> {
        let cap = if matches!(action, "submit" | "cancel") {
            PLANS_SUBMIT
        } else {
            PLANS_APPROVE
        };
        let mut client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company, cap).await?;
        let explanation = reason(explanation)?;
        let (from, to) = match action {
            "submit" => ("draft", "submitted"),
            "approve" => ("submitted", "approved"),
            "reject" => ("submitted", "rejected"),
            "cancel" => ("draft", "cancelled"),
            _ => return Err(TrainingError::InvalidTransition),
        };
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.training.decide_learning_plan"),
            )
            .await?;
        writable(tx.sql(), scope, company).await?;
        self.authorize(tx.sql(), scope, company, cap).await?;
        store::lock(tx.sql(), scope, company).await?;
        let row: LearningPlan = store::fetch(tx.sql(), scope, company, Table::Plan, id).await?;
        if row.status != from {
            return Err(TrainingError::InvalidTransition);
        }
        if cap == PLANS_SUBMIT {
            let me = self.learning_self(&tx, scope, company, cap).await?;
            if row.actor_user_id != scope.user() || row.manager_employee_id != me {
                return Err(TrainingError::NotOwner);
            }
            if self.learning_team(scope, company, me).await?.is_empty() {
                return Err(TrainingError::OutsideTeam);
            }
        } else {
            self.learning_independent(
                &tx,
                scope,
                company,
                row.actor_user_id,
                row.manager_employee_id,
                cap,
            )
            .await?;
        }
        self.learning_current(&tx, scope, company, row.manager_employee_id)
            .await?;
        if to == "approved" {
            if let Some(prior_id) = row.prior_plan_id {
                let prior: LearningPlan =
                    store::fetch(tx.sql(), scope, company, Table::Plan, prior_id).await?;
                if prior.status != "approved" {
                    return Err(TrainingError::SupersededPlan);
                }
                store::transition::<LearningPlan>(
                    &mut tx,
                    scope,
                    company,
                    Table::Plan,
                    prior_id,
                    "superseded",
                    None,
                )
                .await?;
                plan_decision(
                    &mut tx,
                    scope,
                    company,
                    prior_id,
                    "supersede",
                    "approved",
                    "superseded",
                    &explanation,
                )
                .await?;
            }
        }
        let mut row: LearningPlan =
            store::transition(&mut tx, scope, company, Table::Plan, id, to, None).await?;
        plan_decision(&mut tx, scope, company, id, action, from, to, &explanation).await?;
        row.items = store::list(
            tx.sql(),
            scope,
            company,
            Table::Item,
            "AND r.plan_id=$3 ORDER BY r.id",
            &[&id],
        )
        .await?;
        tx.commit().await?;
        Ok(row)
    }
    /// Decisions for requested IDs filtered by the same audience as the register.
    pub async fn learning_histories(
        &self,
        scope: &Scope,
        company: i64,
        kind: LearningKind,
        ids: &[i64],
        audience: LearningAudience,
    ) -> Result<LearningHistories> {
        let visible: Vec<i64> = match kind {
            LearningKind::Request => self
                .learning_requests(scope, company, audience)
                .await?
                .iter()
                .map(|r| r.id)
                .filter(|id| ids.contains(id))
                .collect(),
            LearningKind::Plan => self
                .learning_plans(scope, company, audience)
                .await?
                .iter()
                .map(|r| r.id)
                .filter(|id| ids.contains(id))
                .collect(),
        };
        let client = self.db.get().await.map_err(Error::from)?;
        let cap = match kind {
            LearningKind::Request => request_cap(audience),
            LearningKind::Plan => plan_cap(audience)?,
        };
        self.authorize(&**client, scope, company, cap).await?;
        let (table, suffix) = match kind {
            LearningKind::Request => (
                Table::RequestDecision,
                "AND r.request_id=ANY($3) ORDER BY r.id",
            ),
            LearningKind::Plan => (Table::PlanDecision, "AND r.plan_id=ANY($3) ORDER BY r.id"),
        };
        let decisions: Vec<LearningDecision> =
            store::list(&**client, scope, company, table, suffix, &[&visible]).await?;
        let mut grouped = LearningHistories::new();
        for decision in decisions {
            let id = decision
                .request_id
                .or(decision.plan_id)
                .ok_or(TrainingError::NotFound)?;
            grouped.entry(id).or_default().push(decision);
        }
        Ok(grouped)
    }
    async fn learning_self(
        &self,
        tx: &MutationTransaction<'_>,
        scope: &Scope,
        company: i64,
        cap: &str,
    ) -> Result<i64> {
        Ok(self
            .workforce
            .lock_self_employee(tx, scope, company, cap)
            .await
            .map_err(learning_employee_error)?
            .employee_id)
    }
    async fn learning_team(&self, scope: &Scope, company: i64, manager: i64) -> Result<Vec<i64>> {
        let read = self
            .workforce
            .direct_reports(scope, company, manager)
            .await
            .map_err(learning_employee_error)?;
        Ok(read
            .require_current()
            .map_err(|e| TrainingError::WorkforceNotCurrent(e.0))?
            .iter()
            .filter_map(|e| e.reference.stable_id.parse().ok())
            .collect())
    }
    async fn learning_audience(
        &self,
        scope: &Scope,
        company: i64,
        audience: LearningAudience,
    ) -> Result<Option<Vec<i64>>> {
        if audience == LearningAudience::Hr {
            return Ok(None);
        }
        let me = self
            .workforce
            .self_employee(scope, company)
            .await
            .map_err(learning_employee_error)?
            .employee_id;
        if audience == LearningAudience::SelfService {
            Ok(Some(vec![me]))
        } else {
            Ok(Some(self.learning_team(scope, company, me).await?))
        }
    }
    async fn learning_current(
        &self,
        tx: &MutationTransaction<'_>,
        scope: &Scope,
        company: i64,
        employee: i64,
    ) -> Result<()> {
        self.workforce
            .lock_working_employee(tx, scope, company, employee)
            .await
            .map_err(learning_employee_error)?;
        Ok(())
    }
    async fn learning_independent(
        &self,
        tx: &MutationTransaction<'_>,
        scope: &Scope,
        company: i64,
        author: i64,
        subject: i64,
        cap: &str,
    ) -> Result<()> {
        if author == scope.user()
            || self
                .workforce
                .owns_employee(tx, scope, company, subject, cap)
                .await
                .map_err(learning_employee_error)?
        {
            return Err(TrainingError::SelfApproval);
        }
        Ok(())
    }
}
fn learning_employee_error(error: WorkforceError) -> TrainingError {
    match error {
        WorkforceError::NotLinked | WorkforceError::NotFound => TrainingError::EmployeeUnavailable,
        other => other.into(),
    }
}
async fn spent(
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company: i64,
    currency: &str,
    from: &str,
    to: &str,
) -> Result<Decimal> {
    Ok(client.query_one("SELECT COALESCE(sum(approved_cost),0) FROM people_training_needs WHERE tenant_id=$1 AND company_id=$2 AND currency=$3 AND status='approved' AND proposed_on BETWEEN $4::text::date AND $5::text::date",&[&scope.tenant(),&company,&currency,&from,&to]).await.map_err(Error::from)?.get(0))
}
#[allow(clippy::too_many_arguments)]
async fn request_decision(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company: i64,
    id: i64,
    action: &str,
    from: &str,
    to: &str,
    reason: &str,
) -> Result<()> {
    store::decision(
        tx,
        scope,
        company,
        Table::RequestDecision,
        json!({"request_id":id,"action":action,"from_status":from,"to_status":to,"reason":reason}),
    )
    .await
}
#[allow(clippy::too_many_arguments)]
async fn plan_decision(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company: i64,
    id: i64,
    action: &str,
    from: &str,
    to: &str,
    reason: &str,
) -> Result<()> {
    store::decision(
        tx,
        scope,
        company,
        Table::PlanDecision,
        json!({"plan_id":id,"action":action,"from_status":from,"to_status":to,"reason":reason}),
    )
    .await
}

pub(super) fn request_step(status: &str, action: &str) -> Result<&'static str> {
    step(status, action).map(|(_, _, cap)| cap)
}
