//! Private SQL for Bilimbi Training Governance. Table and column identifiers
//! come only from this module's closed enum; all caller data is bound.
use crate::{Result, TrainingError};
use bilibi_database::{Change, Error, MutationTransaction};
use bilibi_tenancy::Scope;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tokio_postgres::{types::ToSql, GenericClient};

#[derive(Clone, Copy)]
pub(super) enum Table {
    Budget,
    Request,
    Plan,
    Item,
    RequestDecision,
    PlanDecision,
}
impl Table {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Budget => "people_training_budget_policies",
            Self::Request => "people_training_needs",
            Self::Plan => "people_training_team_plans",
            Self::Item => "people_training_team_plan_items",
            Self::RequestDecision => "people_training_need_decisions",
            Self::PlanDecision => "people_training_plan_decisions",
        }
    }
    fn kind(self) -> &'static str {
        match self {
            Self::Budget => "people_training_budget_policy",
            Self::Request => "people_training_need",
            Self::Plan => "people_training_team_plan",
            Self::Item => "people_training_team_plan_item",
            Self::RequestDecision => "people_training_need_decision",
            Self::PlanDecision => "people_training_plan_decision",
        }
    }
    pub(super) fn projection(self) -> &'static str {
        match self {
            Self::Budget => "to_jsonb(r) || jsonb_build_object('amount',r.amount::text)",
            Self::Request => "to_jsonb(r) || jsonb_build_object('estimated_cost',r.estimated_cost::text,'approved_cost',r.approved_cost::text)",
            _ => "to_jsonb(r)",
        }
    }
    fn columns(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Budget => &[
                ("currency", "text"),
                ("effective_from", "date"),
                ("effective_to", "date"),
                ("amount", "numeric"),
                ("reason", "text"),
                ("supersedes_id", "bigint"),
            ],
            Self::Request => &[
                ("employee_id", "bigint"),
                ("course_id", "bigint"),
                ("need", "text"),
                ("objective", "text"),
                ("expected_result", "text"),
                ("proposed_on", "date"),
                ("estimated_cost", "numeric"),
                ("currency", "text"),
                ("status", "text"),
            ],
            Self::Plan => &[
                ("plan_key", "text"),
                ("version", "integer"),
                ("manager_employee_id", "bigint"),
                ("period_start", "date"),
                ("period_end", "date"),
                ("objectives", "text"),
                ("status", "text"),
                ("prior_plan_id", "bigint"),
                ("reason", "text"),
            ],
            Self::Item => &[
                ("plan_id", "bigint"),
                ("request_id", "bigint"),
                ("need", "text"),
                ("expected_result", "text"),
                ("target_cohort", "text"),
                ("responsible_owner", "text"),
                ("intended_timing", "text"),
                ("evaluation_approach", "text"),
            ],
            Self::RequestDecision => &[
                ("request_id", "bigint"),
                ("action", "text"),
                ("from_status", "text"),
                ("to_status", "text"),
                ("reason", "text"),
            ],
            Self::PlanDecision => &[
                ("plan_id", "bigint"),
                ("action", "text"),
                ("from_status", "text"),
                ("to_status", "text"),
                ("reason", "text"),
            ],
        }
    }
}
pub(super) fn decode<T: DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|_| TrainingError::Failed(Error::Internal))
}
pub(super) async fn fetch<T: DeserializeOwned>(
    client: &impl GenericClient,
    scope: &Scope,
    company: i64,
    table: Table,
    id: i64,
) -> Result<T> {
    let value: Value = client
        .query_opt(
            &format!(
                "SELECT {} AS value FROM {} r WHERE r.tenant_id=$1 AND r.company_id=$2 AND r.id=$3",
                table.projection(),
                table.name()
            ),
            &[&scope.tenant(), &company, &id],
        )
        .await
        .map_err(Error::from)?
        .ok_or(TrainingError::NotFound)?
        .get("value");
    decode(value)
}
pub(super) async fn list<T: DeserializeOwned>(
    client: &impl GenericClient,
    scope: &Scope,
    company: i64,
    table: Table,
    suffix: &str,
    extra: &[&(dyn ToSql + Sync)],
) -> Result<Vec<T>> {
    let tenant = scope.tenant();
    let mut args: Vec<&(dyn ToSql + Sync)> = vec![&tenant, &company];
    args.extend_from_slice(extra);
    client
        .query(
            &format!(
                "SELECT {} AS value FROM {} r WHERE r.tenant_id=$1 AND r.company_id=$2 {suffix}",
                table.projection(),
                table.name()
            ),
            &args,
        )
        .await
        .map_err(Error::from)?
        .into_iter()
        .map(|r| decode(r.get("value")))
        .collect()
}
pub(super) async fn insert<T: DeserializeOwned>(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company: i64,
    table: Table,
    mut data: Value,
) -> Result<T> {
    data["tenant_id"] = json!(scope.tenant());
    data["company_id"] = json!(company);
    data["actor_user_id"] = json!(scope.user());
    data["impersonator_id"] = json!(scope.impersonator().map(|s| s.user()));
    let common = &[
        ("tenant_id", "bigint"),
        ("company_id", "bigint"),
        ("actor_user_id", "bigint"),
        ("impersonator_id", "bigint"),
    ];
    let fields: Vec<_> = common.iter().chain(table.columns()).collect();
    let names = fields.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(",");
    let values = fields
        .iter()
        .map(|(n, t)| format!("($1::jsonb->>'{n}')::{t}"))
        .collect::<Vec<_>>()
        .join(",");
    let statement = format!(
        "INSERT INTO {} AS r ({names}) VALUES ({values}) RETURNING {} AS value",
        table.name(),
        table.projection()
    );
    let value = tx
        .mutate(move |sql| {
            Box::pin(async move {
                let value: Value = sql.query_one(&statement, &[&data]).await?.get("value");
                let id = value["id"].as_i64().ok_or(Error::Internal)?;
                let change = Change {
                    kind: table.kind().into(),
                    id,
                    old: json!({}),
                    new: value.clone(),
                };
                Ok((value, vec![change]))
            })
        })
        .await?;
    decode(value)
}
pub(super) async fn transition<T: DeserializeOwned>(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company: i64,
    table: Table,
    id: i64,
    status: &str,
    budget: Option<(i64, String)>,
) -> Result<T> {
    let old: Value = fetch(tx.sql(), scope, company, table, id).await?;
    let status = status.to_owned();
    let tenant = scope.tenant();
    let value=tx.mutate(move |sql| Box::pin(async move {
        let value:Value=if matches!(table,Table::Request) {
            let (policy,amount)=budget.map(|(p,a)|(Some(p),Some(a))).unwrap_or((None,None));
            sql.query_one(&format!("UPDATE {} AS r SET status=$4,budget_policy_id=$5,approved_cost=$6::text::numeric,updated_at=now() AT TIME ZONE 'UTC' WHERE tenant_id=$1 AND company_id=$2 AND id=$3 RETURNING {} AS value",table.name(),table.projection()),&[&tenant,&company,&id,&status,&policy,&amount]).await?.get("value")
        } else {
            sql.query_one(&format!("UPDATE {} AS r SET status=$4,updated_at=now() AT TIME ZONE 'UTC' WHERE tenant_id=$1 AND company_id=$2 AND id=$3 RETURNING {} AS value",table.name(),table.projection()),&[&tenant,&company,&id,&status]).await?.get("value")
        };
        let change=Change{kind:table.kind().into(),id,old,new:value.clone()};
        Ok((value,vec![change]))
    })).await?;
    decode(value)
}
pub(super) async fn decision(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company: i64,
    table: Table,
    data: Value,
) -> Result<()> {
    insert::<Value>(tx, scope, company, table, data).await?;
    Ok(())
}
/// The same advisory lock the database triggers take, after the company lock.
pub(super) async fn lock(client: &impl GenericClient, scope: &Scope, company: i64) -> Result<()> {
    let key = format!("people.training:{}:{company}", scope.tenant());
    client
        .query_one(
            "SELECT pg_advisory_xact_lock(hashtextextended($1,0))",
            &[&key],
        )
        .await
        .map_err(Error::from)?;
    Ok(())
}
