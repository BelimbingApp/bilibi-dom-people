//! Mirrors Bilimbi Training.Evaluation: operator maintenance and frozen summaries.
use crate::{evaluation_store as store, evaluation_store::Table, *};
use bilibi_module_registry::{Contribution, Provenance};
use bilibi_people_workforce::Employee;
use bilibi_settings::{Definition, Editor, ScopeKind, Setting, Validator, ValueType, DEFINITIONS};
use jiff::ToSpan;
use rust_decimal::{Decimal, RoundingStrategy};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const EFFECTIVENESS_VIEW: &str = "people.training.effectiveness.view";
pub const EVALUATION_SUBMIT: &str = "people.training.evaluation.submit";
pub const EFFECTIVENESS_ANSWER: &str = "people.training.effectiveness.answer";
pub const EFFECTIVENESS_SUMMARY_VIEW: &str = "people.training.effectiveness.summary.view";
pub const EVALUATION_POLICY_MANAGE: &str = "people.training.evaluation.policy.manage";
pub const EVALUATION_REMINDERS_MANAGE: &str = "people.training.evaluation.reminders.manage";
const KEYS: [&str; 6] = [
    "people.training.evaluation.evaluation_days",
    "people.training.evaluation.checkpoints",
    "people.training.evaluation.reminder_days",
    "people.training.evaluation.minimum_cohort",
    "people.training.evaluation.report_months",
    "people.training.evaluation.report_grace_days",
];
const PROVENANCE: Provenance = Provenance {
    module: MODULE,
    source: file!(),
    line: line!(),
};
inventory::submit! {
    Contribution::new(bilibi_authz::VOCABULARY,"people/training.evaluation_vocabulary",PROVENANCE,&bilibi_authz::VocabularyContribution {
        domains:&["people"],verbs:&["answer"],capabilities:&[
            bilibi_authz::CapabilityDefinition::tenant(EFFECTIVENESS_VIEW),bilibi_authz::CapabilityDefinition::tenant(EVALUATION_SUBMIT),
            bilibi_authz::CapabilityDefinition::tenant(EFFECTIVENESS_ANSWER),bilibi_authz::CapabilityDefinition::tenant(EFFECTIVENESS_SUMMARY_VIEW),
            bilibi_authz::CapabilityDefinition::tenant(EVALUATION_POLICY_MANAGE),bilibi_authz::CapabilityDefinition::tenant(EVALUATION_REMINDERS_MANAGE),
        ],roles:&[],system_principals:&[],
    })
}
fn checkpoints_valid(days: &[i32]) -> bool {
    (1..=24).contains(&days.len())
        && days.iter().all(|d| (1..=3650).contains(d))
        && days.iter().collect::<BTreeSet<_>>().len() == days.len()
}
fn checkpoints_setting(v: &Value) -> bool {
    v.is_null()
        || serde_json::from_value::<Vec<i32>>(v.clone()).is_ok_and(|d| checkpoints_valid(&d))
}
fn months_setting(v: &Value) -> bool {
    v.is_null()
        || v.as_i64()
            .is_some_and(|m| (1..=12).contains(&m) && 12 % m == 0)
}
macro_rules! setting {
    ($key:literal,$kind:ident,$default:expr,$min:expr,$max:expr,$validator:expr,$label:literal,$help:literal) => {
        inventory::submit! { Contribution::new(DEFINITIONS,concat!("people.training.evaluation.",$key),PROVENANCE,&Setting::Definition(Definition {
            value_type:ValueType::$kind,scopes:&[ScopeKind::Company],default:||$default,nullable:true,encrypted:false,
            minimum:$min,maximum:$max,validator:$validator,
            editor:Some(Editor{group:"People",label:$label,help:$help,capability:Some(EVALUATION_POLICY_MANAGE),reveal_duration_ms:5_000}),
        })) }
    };
}
setting!(
    "evaluation_days",
    Integer,
    Value::Null,
    Some(0.0),
    Some(3650.0),
    None,
    "Evaluation due days",
    "Days after the session ends when employee evaluation is due."
);
setting!(
    "reminder_days",
    Integer,
    Value::Null,
    Some(0.0),
    Some(3650.0),
    None,
    "Evaluation reminder lead",
    "Days before a review is due when its reminder becomes available."
);
setting!(
    "minimum_cohort",
    Integer,
    Value::Null,
    Some(2.0),
    Some(1000.0),
    None,
    "Effectiveness disclosure minimum",
    "Minimum distinct employees and answered employees for a summary score."
);
setting!("report_months",Integer,json!(3),Some(1.0),Some(12.0),Some(Validator{accepts:months_setting,message:"Choose a reporting period that divides 12 months."}),"Effectiveness reporting period","Months per fixed summary period; 12 must divide evenly by it. 3 is a calendar quarter. A changed length applies from the day after the last frozen period; Effectiveness shows that date.");
setting!(
    "report_grace_days",
    Integer,
    Value::Null,
    Some(0.0),
    Some(3650.0),
    None,
    "Effectiveness answer grace",
    "Days after a reporting period ends before its summary is frozen permanently."
);
setting!(
    "checkpoints",
    Array,
    Value::Null,
    None,
    None,
    Some(Validator {
        accepts: checkpoints_setting,
        message: "Set 1 to 24 distinct positive checkpoint day offsets, at most 3650."
    }),
    "Effectiveness checkpoints",
    "Distinct positive day offsets after a session, captured by each published policy."
);

fn reason_valid(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 4000
}
fn criteria_valid(criteria: &[EvaluationCriterion]) -> bool {
    (1..=50).contains(&criteria.len())
        && criteria.iter().all(|c| {
            let mut code = c.code.bytes();
            code.next().is_some_and(|b| b.is_ascii_lowercase())
                && c.code.len() <= 80
                && code.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                && reason_valid(&c.label)
                && c.minimum >= -10000
                && c.maximum <= 10000
                && c.minimum <= c.maximum
        })
        && criteria
            .iter()
            .map(|c| &c.code)
            .collect::<BTreeSet<_>>()
            .len()
            == criteria.len()
}
fn date(s: &str) -> Result<Date> {
    let parsed = s
        .parse::<Date>()
        .map_err(|_| TrainingError::InvalidEvaluationPeriod)?;
    if parsed.to_string() != s {
        return Err(TrainingError::InvalidEvaluationPeriod);
    }
    Ok(parsed)
}
fn shift_days(day: Date, days: i32) -> Result<Date> {
    day.checked_add(days.days())
        .map_err(|_| TrainingError::InvalidEvaluationPeriod)
}
fn task_caps(kind: EvaluationKind) -> [&'static str; 2] {
    match kind {
        EvaluationKind::Evaluation => [REQUESTS_SUBMIT, EVALUATION_SUBMIT],
        EvaluationKind::Effectiveness => [EFFECTIVENESS_VIEW, EFFECTIVENESS_ANSWER],
    }
}
impl Training {
    /// Six company settings; unset values stay null, except report_months defaults to 3.
    pub async fn evaluation_settings(
        &self,
        scope: &Scope,
        company: i64,
    ) -> Result<EvaluationSettings> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company, EVALUATION_POLICY_MANAGE)
            .await?;
        self.evaluation_settings_on(&**client, scope, company).await
    }
    async fn evaluation_settings_on(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company: i64,
    ) -> Result<EvaluationSettings> {
        let resolved = self
            .settings
            .resolve_many(
                client,
                scope,
                Target::Company(OrganisationId::new(company)?),
                &KEYS,
            )
            .await?;
        let values = KEYS
            .into_iter()
            .map(|key| {
                (
                    key.rsplit('.').next().unwrap_or(key).to_owned(),
                    resolved[key].value.clone(),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        governance_store::decode(Value::Object(values))
    }
    /// Validates all six values before any write; changes are audited atomically.
    pub async fn put_evaluation_settings(
        &self,
        scope: &Scope,
        company: i64,
        settings: &EvaluationSettings,
    ) -> Result<EvaluationSettings> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company, EVALUATION_POLICY_MANAGE)
            .await?;
        let days_valid = |d: Option<i32>| d.is_some_and(|d| (0..=3650).contains(&d));
        if !days_valid(settings.evaluation_days)
            || !days_valid(settings.reminder_days)
            || !days_valid(settings.report_grace_days)
            || !settings
                .minimum_cohort
                .is_some_and(|m| (2..=1000).contains(&m))
            || !settings
                .report_months
                .is_some_and(|m| (1..=12).contains(&m) && 12 % m == 0)
            || !settings
                .checkpoints
                .as_ref()
                .is_some_and(|d| checkpoints_valid(d))
        {
            return Err(TrainingError::InvalidEvaluationSettings);
        }
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.training.put_evaluation_settings"),
            )
            .await?;
        writable(tx.sql(), scope, company).await?;
        self.authorize(tx.sql(), scope, company, EVALUATION_POLICY_MANAGE)
            .await?;
        let values =
            serde_json::to_value(settings).map_err(|_| TrainingError::Failed(Error::Internal))?;
        for key in KEYS {
            let suffix = key.rsplit('.').next().unwrap_or(key);
            self.settings
                .put(
                    &mut tx,
                    scope,
                    Target::Company(OrganisationId::new(company)?),
                    key,
                    values[suffix].clone(),
                )
                .await?;
        }
        tx.commit().await?;
        Ok(settings.clone())
    }
    pub async fn evaluation_policies(
        &self,
        scope: &Scope,
        company: i64,
    ) -> Result<Vec<EvaluationPolicy>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company, EVALUATION_POLICY_MANAGE)
            .await?;
        store::rows(
            &**client,
            scope,
            company,
            Table::Policy,
            "ORDER BY r.version DESC",
            &[],
        )
        .await
    }
    /// Captures current due, reminder and checkpoint settings in an immutable policy.
    pub async fn publish_evaluation_policy(
        &self,
        scope: &Scope,
        company: i64,
        new: &NewEvaluationPolicy,
    ) -> Result<EvaluationPolicy> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company, EVALUATION_POLICY_MANAGE)
            .await?;
        let effective_from = date(&new.effective_from)?;
        let effective_to = date(&new.effective_to)?;
        if effective_to < effective_from {
            return Err(TrainingError::InvalidEvaluationPeriod);
        }
        if !reason_valid(&new.reason) {
            return Err(TrainingError::EvaluationReasonRequired);
        }
        if !criteria_valid(&new.criteria) || !criteria_valid(&new.effectiveness_criteria) {
            return Err(TrainingError::InvalidCriteria);
        }
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.training.publish_evaluation_policy"),
            )
            .await?;
        writable(tx.sql(), scope, company).await?;
        self.authorize(tx.sql(), scope, company, EVALUATION_POLICY_MANAGE)
            .await?;
        let settings = self
            .evaluation_settings_on(tx.sql(), scope, company)
            .await?;
        let (Some(due), Some(lead)) = (settings.evaluation_days, settings.reminder_days) else {
            return Err(TrainingError::EvaluationPolicyNotConfigured);
        };
        let checkpoints = settings
            .checkpoints
            .filter(|d| checkpoints_valid(d))
            .ok_or(TrainingError::InvalidCheckpoints)?;
        let policies: Vec<EvaluationPolicy> = store::rows(
            tx.sql(),
            scope,
            company,
            Table::Policy,
            "ORDER BY r.version DESC",
            &[],
        )
        .await?;
        if policies.iter().any(|p| {
            date(&p.effective_from).is_ok_and(|d| d <= effective_to)
                && date(&p.effective_to).is_ok_and(|d| d >= effective_from)
        }) {
            return Err(TrainingError::EvaluationPolicyOverlap);
        }
        let version = policies.first().map_or(1, |p| p.version + 1);
        let policy=store::insert(&mut tx,scope,company,Table::Policy,json!({"version":version,"effective_from":new.effective_from,"effective_to":new.effective_to,"criteria":{"items":new.criteria},"effectiveness_criteria":{"items":new.effectiveness_criteria},"evaluation_days":due,"checkpoints":{"days":checkpoints},"reminder_days":lead,"reason":new.reason.trim()})).await?;
        tx.commit().await?;
        Ok(policy)
    }
    async fn evaluation_employees(
        &self,
        scope: &Scope,
        company: i64,
    ) -> Result<BTreeMap<i64, Employee>> {
        let employees = self
            .workforce
            .employees(scope, company)
            .await?
            .require_current()
            .map_err(|e| TrainingError::WorkforceNotCurrent(e.0))?;
        employees
            .into_iter()
            .map(|e| {
                e.reference
                    .stable_id
                    .parse::<i64>()
                    .map(|id| (id, e))
                    .map_err(|_| TrainingError::Failed(Error::Internal))
            })
            .collect()
    }
    /// Once per session, learner, kind and checkpoint; confirmed corrections keep obligations.
    pub async fn prepare_evaluation_reviews(
        &self,
        scope: &Scope,
        company: i64,
        session_id: i64,
        now: Timestamp,
    ) -> Result<PreparedEvaluationReviews> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company, EVALUATION_REMINDERS_MANAGE)
            .await?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.training.prepare_evaluation_reviews"),
            )
            .await?;
        writable(tx.sql(), scope, company).await?;
        self.authorize(tx.sql(), scope, company, EVALUATION_REMINDERS_MANAGE)
            .await?;
        let session = tx.sql().query_opt("SELECT to_char(ends_at,'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS ends_at,time_zone FROM people_training_session_runs WHERE tenant_id=$1 AND company_id=$2 AND id=$3 FOR UPDATE", &[&scope.tenant(),&company,&session_id]).await.map_err(Error::from)?.ok_or(TrainingError::SessionUnavailable)?;
        let ends_at = session
            .get::<_, String>("ends_at")
            .parse::<Timestamp>()
            .map_err(|_| TrainingError::Failed(Error::Internal))?;
        if now < ends_at {
            return Err(TrainingError::SessionNotFinished);
        }
        let zone = jiff::tz::TimeZone::get(&session.get::<_, String>("time_zone"))
            .map_err(|_| TrainingError::InvalidTimeZoneOrTime)?;
        let finished = ends_at.to_zoned(zone).date();
        let finished_text = finished.to_string();
        let policy: EvaluationPolicy = store::rows(
            tx.sql(),
            scope,
            company,
            Table::Policy,
            "AND r.effective_from <= $3::text::date AND r.effective_to >= $3::text::date",
            &[&finished_text],
        )
        .await?
        .pop()
        .ok_or(TrainingError::EvaluationPolicyUnavailable)?;
        let employees = self.evaluation_employees(scope, company).await?;
        let facts=tx.sql().query("SELECT id,employee_id FROM (SELECT DISTINCT ON (employee_id) id,employee_id,status FROM people_training_attendance_facts WHERE tenant_id=$1 AND company_id=$2 AND session_id=$3 ORDER BY employee_id,revision DESC) latest WHERE status='confirmed'",&[&scope.tenant(),&company,&session_id]).await.map_err(Error::from)?;
        let mut result = PreparedEvaluationReviews {
            reviews: vec![],
            unknown: 0,
        };
        let mut changed = false;
        for fact in facts {
            let employee: i64 = fact.get("employee_id");
            if !employees.contains_key(&employee) {
                result.unknown += 1;
                continue;
            }
            let fact_id: i64 = fact.get("id");
            let obligations =
                std::iter::once((EvaluationKind::Evaluation, 0, policy.evaluation_days)).chain(
                    policy
                        .checkpoints
                        .days
                        .iter()
                        .map(|d| (EvaluationKind::Effectiveness, *d, *d)),
                );
            for (kind, days, offset) in obligations {
                let existing:Option<EvaluationReview>=store::rows(tx.sql(),scope,company,Table::Review,"AND r.kind=$3 AND r.checkpoint_days=$4 AND EXISTS (SELECT 1 FROM people_training_attendance_facts f WHERE f.id=r.fact_id AND f.tenant_id=r.tenant_id AND f.company_id=r.company_id AND f.session_id=$5 AND f.employee_id=$6)",&[&kind.as_str(),&days,&session_id,&employee]).await?.pop();
                let review = if let Some(review) = existing {
                    review
                } else {
                    changed = true;
                    store::insert(&mut tx,scope,company,Table::Review,json!({"policy_id":policy.id,"fact_id":fact_id,"kind":kind,"checkpoint_days":days,"due_on":shift_days(finished,offset)?.to_string()})).await?
                };
                result.reviews.push(review);
            }
        }
        // A no-op returns without committing an empty mutation.
        if changed {
            tx.commit().await?;
        }
        Ok(result)
    }
    async fn evaluation_task_auth(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company: i64,
        kind: EvaluationKind,
    ) -> Result<()> {
        for cap in task_caps(kind) {
            self.authorize(client, scope, company, cap).await?;
        }
        Ok(())
    }
    /// Own evaluations, or effectiveness reviews for current direct reports.
    pub async fn evaluation_reviews(
        &self,
        scope: &Scope,
        company: i64,
        kind: EvaluationKind,
    ) -> Result<Vec<EvaluationTask>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.evaluation_task_auth(&**client, scope, company, kind)
            .await?;
        let me = self
            .workforce
            .self_employee(scope, company)
            .await
            .map_err(evaluation_employee_error)?
            .employee_id;
        let employees = self.evaluation_employees(scope, company).await?;
        let ids = match kind {
            EvaluationKind::Evaluation => vec![me],
            EvaluationKind::Effectiveness => employees
                .iter()
                .filter(|(id, e)| **id != me && supervisor(e) == Some(me))
                .map(|(id, _)| *id)
                .collect(),
        };
        store::tasks(&**client, scope, company, Some(kind), Some(&ids), None).await
    }
    /// Exact policy codes, bounded integers or explicit null. Answers are permanent.
    /// The signed-in employment is re-proved under lock on every write.
    pub async fn answer_evaluation(
        &self,
        scope: &Scope,
        company: i64,
        review_id: i64,
        values: &Value,
        reason: &str,
    ) -> Result<EvaluationAnswer> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        match self
            .evaluation_task_auth(&**client, scope, company, EvaluationKind::Evaluation)
            .await
        {
            Ok(()) => {}
            Err(TrainingError::Unauthorized) => {
                self.evaluation_task_auth(&**client, scope, company, EvaluationKind::Effectiveness)
                    .await?
            }
            Err(error) => return Err(error),
        }
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.training.answer_evaluation"),
            )
            .await?;
        writable(tx.sql(), scope, company).await?;
        let review: EvaluationReview =
            store::fetch(tx.sql(), scope, company, Table::Review, review_id).await?;
        self.evaluation_task_auth(tx.sql(), scope, company, review.kind)
            .await?;
        let me = self
            .workforce
            .lock_self_employee(&tx, scope, company, task_caps(review.kind)[1])
            .await
            .map_err(evaluation_employee_error)?
            .employee_id;
        let task = store::tasks(
            tx.sql(),
            scope,
            company,
            Some(review.kind),
            None,
            Some(review_id),
        )
        .await?
        .into_iter()
        .find(|t| t.review.id == review_id)
        .ok_or(TrainingError::AttendanceUnavailable)?;
        let employees = self.evaluation_employees(scope, company).await?;
        let subject = employees
            .get(&task.employee_id)
            .ok_or(TrainingError::AttendanceEmployeeUnavailable)?;
        let permitted = match review.kind {
            EvaluationKind::Evaluation => me == task.employee_id,
            EvaluationKind::Effectiveness => {
                me != task.employee_id && supervisor(subject) == Some(me)
            }
        };
        if !permitted {
            return Err(TrainingError::OutsideDirectTeam);
        }
        if task.answer.is_some() {
            return Err(TrainingError::AlreadyAnswered);
        }
        if !answers_valid(&task.criteria, values) {
            return Err(TrainingError::InvalidAnswers);
        }
        if !reason_valid(reason) {
            return Err(TrainingError::EvaluationReasonRequired);
        }
        let answer = store::insert(
            &mut tx,
            scope,
            company,
            Table::Answer,
            json!({"review_id":review_id,"values":values,"reason":reason.trim()}),
        )
        .await?;
        tx.commit().await?;
        Ok(answer)
    }
    /// On-screen worklist, once per review; no mail or unattended delivery.
    pub async fn evaluation_reminders(
        &self,
        scope: &Scope,
        company: i64,
        today: Date,
    ) -> Result<EvaluationReminders> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company, EVALUATION_REMINDERS_MANAGE)
            .await?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.training.evaluation_reminders"),
            )
            .await?;
        writable(tx.sql(), scope, company).await?;
        self.authorize(tx.sql(), scope, company, EVALUATION_REMINDERS_MANAGE)
            .await?;
        let employees = self.evaluation_employees(scope, company).await?;
        let mut result = EvaluationReminders::default();
        for task in store::tasks(tx.sql(), scope, company, None, None, None)
            .await?
            .into_iter()
            .filter(|t| t.answer.is_none() && t.reminder.is_none())
        {
            let policy: EvaluationPolicy = store::fetch(
                tx.sql(),
                scope,
                company,
                Table::Policy,
                task.review.policy_id,
            )
            .await?;
            if today < shift_days(date(&task.review.due_on)?, -policy.reminder_days)? {
                continue;
            }
            let recipient = employees
                .get(&task.employee_id)
                .and_then(|e| match task.review.kind {
                    EvaluationKind::Evaluation => Some(task.employee_id),
                    EvaluationKind::Effectiveness => supervisor(e),
                })
                .filter(|id| employees.contains_key(id));
            if let Some(recipient) = recipient {
                store::insert::<Value>(&mut tx,scope,company,Table::Reminder,json!({"review_id":task.review.id,"recipient_employee_id":recipient,"available_on":today.to_string()})).await?;
                result.created += 1;
            } else {
                result.unknown += 1;
            }
        }
        if result.created > 0 {
            tx.commit().await?;
        }
        Ok(result)
    }
    /// Frozen snapshots only; scores are never recomputed by a read.
    pub async fn effectiveness_summary(
        &self,
        scope: &Scope,
        company: i64,
    ) -> Result<Vec<EffectivenessSummary>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company, EFFECTIVENESS_SUMMARY_VIEW)
            .await?;
        store::rows(
            &**client,
            scope,
            company,
            Table::Summary,
            "ORDER BY r.period_start DESC",
            &[],
        )
        .await
    }
    pub async fn effectiveness_period_start(
        &self,
        scope: &Scope,
        company: i64,
    ) -> Result<Option<String>> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company, EVALUATION_POLICY_MANAGE)
            .await?;
        let settings = self
            .evaluation_settings_on(&**client, scope, company)
            .await?;
        next_period(&**client, scope, company, report_months(&settings)?)
            .await
            .map(|d| d.map(|d| d.to_string()))
    }
    /// Freezes contiguous periods strictly after grace. Length changes start after the last snapshot.
    pub async fn freeze_effectiveness_summaries(
        &self,
        scope: &Scope,
        company: i64,
        today: Date,
    ) -> Result<FrozenEffectivenessSummaries> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        self.authorize(&**client, scope, company, EVALUATION_REMINDERS_MANAGE)
            .await?;
        let mut tx = self
            .mutations
            .begin(
                &mut client,
                scope.mutation_context("people.training.freeze_effectiveness_summaries"),
            )
            .await?;
        writable(tx.sql(), scope, company).await?;
        self.authorize(tx.sql(), scope, company, EVALUATION_REMINDERS_MANAGE)
            .await?;
        let settings = self
            .evaluation_settings_on(tx.sql(), scope, company)
            .await?;
        let months = report_months(&settings)?;
        let minimum = settings
            .minimum_cohort
            .filter(|m| (2..=1000).contains(m))
            .ok_or(TrainingError::EffectivenessReportNotConfigured)? as usize;
        let grace = settings
            .report_grace_days
            .filter(|d| (0..=3650).contains(d))
            .ok_or(TrainingError::EffectivenessReportNotConfigured)?;
        let employees = self.evaluation_employees(scope, company).await?;
        let mut first = next_period(tx.sql(), scope, company, months).await?;
        let tasks = store::tasks(
            tx.sql(),
            scope,
            company,
            Some(EvaluationKind::Effectiveness),
            None,
            None,
        )
        .await?;
        let mut frozen = 0;
        while let Some(start) = first {
            let end = shift_days(
                start
                    .checked_add(months.months())
                    .map_err(|_| TrainingError::InvalidEvaluationPeriod)?,
                -1,
            )?;
            if today <= shift_days(end, grace)? {
                break;
            }
            let cohort: Vec<_> = tasks
                .iter()
                .filter(|t| {
                    employees.contains_key(&t.employee_id)
                        && t.review.due_on >= start.to_string()
                        && t.review.due_on <= end.to_string()
                })
                .collect();
            let disclosed = subjects(&cohort) >= minimum;
            let groups = if disclosed {
                aggregate(&cohort, minimum)
            } else {
                vec![]
            };
            store::insert::<EffectivenessSummary>(&mut tx,scope,company,Table::Summary,json!({"period_start":start.to_string(),"period_end":end.to_string(),"minimum_cohort":minimum,"status":if disclosed {"current"} else {"suppressed"},"groups":{"items":groups}})).await?;
            frozen += 1;
            first = Some(shift_days(end, 1)?);
        }
        if frozen > 0 {
            tx.commit().await?;
        }
        Ok(FrozenEffectivenessSummaries { frozen })
    }
}
fn evaluation_employee_error(error: WorkforceError) -> TrainingError {
    match error {
        WorkforceError::NotLinked | WorkforceError::NotFound => {
            TrainingError::AttendanceEmployeeUnavailable
        }
        other => other.into(),
    }
}
fn supervisor(employee: &Employee) -> Option<i64> {
    employee
        .supervisor_reference
        .as_ref()?
        .stable_id
        .parse()
        .ok()
}
fn answers_valid(criteria: &[EvaluationCriterion], values: &Value) -> bool {
    values.as_object().is_some_and(|v| {
        v.len() == criteria.len()
            && criteria.iter().all(|c| {
                v.get(&c.code).is_some_and(|score| {
                    score.is_null()
                        || score.as_i64().is_some_and(|score| {
                            score >= i64::from(c.minimum) && score <= i64::from(c.maximum)
                        })
                })
            })
    })
}
fn report_months(settings: &EvaluationSettings) -> Result<i32> {
    settings
        .report_months
        .filter(|m| (1..=12).contains(m) && 12 % m == 0)
        .ok_or(TrainingError::EffectivenessReportNotConfigured)
}
async fn next_period(
    client: &impl GenericClient,
    scope: &Scope,
    company: i64,
    months: i32,
) -> Result<Option<Date>> {
    let row=client.query_one("SELECT (SELECT to_char(max(period_end),'YYYY-MM-DD') FROM people_training_effectiveness_summaries WHERE tenant_id=$1 AND company_id=$2) AS last, (SELECT to_char(min(due_on),'YYYY-MM-DD') FROM people_training_evaluation_reviews WHERE tenant_id=$1 AND company_id=$2 AND kind='effectiveness') AS first",&[&scope.tenant(),&company]).await.map_err(Error::from)?;
    if let Some(last) = row.get::<_, Option<String>>("last") {
        return Ok(Some(shift_days(date(&last)?, 1)?));
    }
    row.get::<_, Option<String>>("first")
        .map(|first| {
            let first = date(&first)?;
            Date::new(
                first.year(),
                ((i32::from(first.month()) - 1) / months * months + 1) as i8,
                1,
            )
            .map_err(|_| TrainingError::InvalidEvaluationPeriod)
        })
        .transpose()
}
fn subjects(rows: &[&EvaluationTask]) -> usize {
    rows.iter()
        .map(|t| t.employee_id)
        .collect::<BTreeSet<_>>()
        .len()
}
fn aggregate(rows: &[&EvaluationTask], minimum: usize) -> Vec<EffectivenessGroup> {
    let mut groups: BTreeMap<(i64, i32), Vec<&EvaluationTask>> = BTreeMap::new();
    for task in rows {
        groups
            .entry((task.review.policy_id, task.review.checkpoint_days))
            .or_default()
            .push(task);
    }
    groups
        .into_values()
        .map(|cohort| {
            let first = cohort[0];
            let disclosed = subjects(&cohort) >= minimum;
            let criteria = if disclosed {
                first
                    .criteria
                    .iter()
                    .map(|criterion| {
                        let known: Vec<_> = cohort
                            .iter()
                            .filter_map(|t| {
                                t.answer
                                    .as_ref()?
                                    .values
                                    .get(&criterion.code)?
                                    .as_i64()
                                    .map(|score| (t.employee_id, score))
                            })
                            .collect();
                        if known
                            .iter()
                            .map(|(id, _)| id)
                            .collect::<BTreeSet<_>>()
                            .len()
                            >= minimum
                        {
                            let mean =
                                (Decimal::from(known.iter().map(|(_, score)| score).sum::<i64>())
                                    / Decimal::from(known.len() as u64))
                                .round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
                            EffectivenessCriterion {
                                label: criterion.label.clone(),
                                status: "current".into(),
                                answered: Some(known.len()),
                                mean: Some(format!("{mean:.2}")),
                            }
                        } else {
                            EffectivenessCriterion {
                                label: criterion.label.clone(),
                                status: "suppressed".into(),
                                answered: None,
                                mean: None,
                            }
                        }
                    })
                    .collect()
            } else {
                vec![]
            };
            EffectivenessGroup {
                policy_version: first.policy_version,
                checkpoint_days: first.review.checkpoint_days,
                status: if disclosed { "current" } else { "suppressed" }.into(),
                criteria,
            }
        })
        .collect()
}
