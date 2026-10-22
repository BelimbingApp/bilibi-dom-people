//! Private scoped policy SQL; mutation snapshots use the same row projection.
use crate::{Policy, ProgressionError as E, Result, Write};
use bilibi_database::{Change, Error};
use bilibi_tenancy::Scope;
use serde_json::{json, Value};
use tokio_postgres::{types::ToSql, GenericClient};

const TABLE: &str = "people_progression_policy_versions";
const VIEW: &str = "jsonb_build_object('id',t.id,'code',t.code,'version',t.version,'name',t.name,'effective_from',to_char(t.effective_from,'YYYY-MM-DD'),'rules',t.rules,'status',t.status,'actor_user_id',t.actor_user_id,'published_by_user_id',t.published_by_user_id,'published_at',to_char(t.published_at,'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'))";
fn decode(value: Value) -> Result<Policy> {
    serde_json::from_value(value).map_err(|_| E::Failed(Error::Internal))
}
pub(crate) async fn rows(
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company: i64,
    tail: &str,
    args: &[&(dyn ToSql + Sync)],
) -> Result<Vec<Policy>> {
    let tenant = scope.tenant();
    let mut params: Vec<&(dyn ToSql + Sync)> = vec![&tenant, &company];
    params.extend_from_slice(args);
    client
        .query(
            &format!(
                "SELECT {VIEW} FROM {TABLE} t WHERE t.tenant_id=$1 AND t.company_id=$2 {tail}"
            ),
            &params,
        )
        .await
        .map_err(Error::from)?
        .into_iter()
        .map(|r| decode(r.get(0)))
        .collect()
}
pub(crate) async fn insert(
    w: &mut Write<'_>,
    input: &crate::DraftInput,
    rules: crate::Rules,
) -> Result<Policy> {
    let (tenant, company, actor) = (w.tenant, w.company, w.actor);
    let code = input.code.trim().to_owned();
    let name = input.name.trim().to_owned();
    let version = input.version;
    let effective = input.effective_from.clone();
    let rules = serde_json::to_value(rules).map_err(|_| E::InvalidRules)?;
    let value = w.tx.mutate(move |db| Box::pin(async move {
        let row = db.query_one(&format!("INSERT INTO {TABLE} AS t (tenant_id,company_id,actor_user_id,code,version,name,effective_from,rules,status) VALUES ($1,$2,$3,$4,$5,$6,$7::text::date,$8,'draft') RETURNING {VIEW}"), &[&tenant,&company,&actor,&code,&version,&name,&effective,&rules]).await?;
        let value: Value = row.get(0);
        let id = value["id"].as_i64().ok_or(Error::Internal)?;
        Ok((value.clone(),vec![Change {kind: TABLE.into(),id,old:json!({}),new:value}]))
    })).await?;
    decode(value)
}
pub(crate) async fn publish(w: &mut Write<'_>, old: Policy) -> Result<Policy> {
    let (tenant, company, actor, id) = (w.tenant, w.company, w.actor, old.id);
    let before = serde_json::to_value(old).map_err(|_| E::Failed(Error::Internal))?;
    let value = w.tx.mutate(move |db| Box::pin(async move {
        let row = db.query_one(&format!("UPDATE {TABLE} AS t SET status='published',published_by_user_id=$4,published_at=date_trunc('second',now() AT TIME ZONE 'UTC'),updated_at=date_trunc('second',now() AT TIME ZONE 'UTC') WHERE tenant_id=$1 AND company_id=$2 AND id=$3 AND status='draft' RETURNING {VIEW}"), &[&tenant,&company,&id,&actor]).await?;
        let value: Value = row.get(0);
        Ok((value.clone(),vec![Change {kind: TABLE.into(),id,old:before,new:value}]))
    })).await?;
    decode(value)
}

pub(crate) async fn governing(
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company: i64,
    today: &str,
) -> Result<Vec<Policy>> {
    client.query(&format!("SELECT DISTINCT ON (t.code) {VIEW} FROM {TABLE} t WHERE t.tenant_id=$1 AND t.company_id=$2 AND t.status='published' AND t.effective_from <= $3::text::date ORDER BY t.code,t.effective_from DESC,t.version DESC"), &[&scope.tenant(),&company,&today]).await.map_err(Error::from)?.into_iter().map(|r|decode(r.get(0))).collect()
}
