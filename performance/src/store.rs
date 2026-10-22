//! SQL is private. Only Table's fixed metadata becomes an identifier; values bind.
//! Every affected row is captured by MutationBoundary, including review pins.
use crate::{tables::Table, PerformanceError as E, Result, Write};
use bilibi_database::{Change, Error};
use bilibi_tenancy::Scope;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use tokio_postgres::{types::ToSql, GenericClient};

pub(crate) fn projection(table: Table) -> String {
    let mut projection = "to_jsonb(t) - ARRAY['tenant_id','company_id']".to_owned();
    // Explicit textual dates and UTC instants: no driver date feature or
    // dependence on PostgreSQL's DateStyle/TimeZone presentation settings.
    for date in table.dates() {
        projection.push_str(&format!(
            " || jsonb_build_object('{date}', to_char(t.{date}, 'YYYY-MM-DD'))"
        ));
    }
    for time in table.times() {
        projection.push_str(&format!(" || jsonb_build_object('{time}', to_char(t.{time}, 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'))"));
    }
    projection
}
fn decode<T: DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|_| E::Failed(Error::Internal))
}
pub(crate) async fn query<T: DeserializeOwned>(
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company: i64,
    table: Table,
    tail: &str,
    args: &[&(dyn ToSql + Sync)],
) -> Result<Vec<T>> {
    let tenant = scope.tenant();
    let mut parameters: Vec<&(dyn ToSql + Sync)> = vec![&tenant, &company];
    parameters.extend_from_slice(args);
    let sql = format!(
        "SELECT {} FROM {} t WHERE t.tenant_id = $1 AND t.company_id = $2 {tail}",
        projection(table),
        table.name()
    );
    client
        .query(&sql, &parameters)
        .await
        .map_err(Error::from)?
        .into_iter()
        .map(|r| decode(r.get(0)))
        .collect()
}
pub(crate) async fn get<T: DeserializeOwned>(
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company: i64,
    table: Table,
    id: i64,
) -> Result<T> {
    if id <= 0 {
        return Err(E::NotFound);
    }
    query(client, scope, company, table, "AND t.id = $3", &[&id])
        .await?
        .pop()
        .ok_or(E::NotFound)
}
pub(crate) async fn successor(w: &Write<'_>, table: Table, id: i64) -> Result<bool> {
    let sql = format!(
        "SELECT 1 FROM {} WHERE tenant_id = $1 AND company_id = $2 AND supersedes_id = $3",
        table.name()
    );
    Ok(w.tx
        .sql()
        .query_opt(&sql, &[&w.tenant, &w.company, &id])
        .await
        .map_err(Error::from)?
        .is_some())
}
pub(crate) async fn insert<T: DeserializeOwned>(
    w: &mut Write<'_>,
    table: Table,
    value: Value,
) -> Result<T> {
    let (tenant, company, actor) = (w.tenant, w.company, w.actor);
    let sql = format!("{} {}", table.insert_sql(), projection(table));
    let value =
        w.tx.mutate(move |db| {
            Box::pin(async move {
                let row = db
                    .query_one(&sql, &[&tenant, &company, &actor, &value])
                    .await?;
                let saved: Value = row.get(0);
                let id = saved["id"].as_i64().ok_or(Error::Internal)?;
                let change = Change {
                    kind: table.name().into(),
                    id,
                    old: json!({}),
                    new: saved.clone(),
                };
                Ok((saved, vec![change]))
            })
        })
        .await?;
    decode(value)
}
pub(crate) enum Transition {
    PublishDescription,
    ReviewTarget(String),
    PublishTarget,
    ReleaseReview,
}
pub(crate) async fn transition<T: DeserializeOwned + Serialize>(
    w: &mut Write<'_>,
    table: Table,
    id: i64,
    old: T,
    change: Transition,
) -> Result<T> {
    let (updates, note) = match change {
        Transition::PublishDescription | Transition::PublishTarget => ("status = 'published', published_at = (clock_timestamp() AT TIME ZONE 'UTC'), published_by_user_id = $4", None),
        Transition::ReviewTarget(note) => ("status = 'reviewed', review_note = $5, reviewed_by_user_id = $4", Some(note)),
        Transition::ReleaseReview => ("status = 'released', released_at = (clock_timestamp() AT TIME ZONE 'UTC'), released_by_user_id = $4", None),
    };
    let sql = format!("UPDATE {} t SET {updates}, updated_at = (clock_timestamp() AT TIME ZONE 'UTC') WHERE tenant_id = $1 AND company_id = $2 AND id = $3 RETURNING {}", table.name(), projection(table));
    let old = serde_json::to_value(old).map_err(|_| E::Failed(Error::Internal))?;
    let (tenant, company, actor) = (w.tenant, w.company, w.actor);
    let value =
        w.tx.mutate(move |db| {
            Box::pin(async move {
                let mut args: Vec<&(dyn ToSql + Sync)> = vec![&tenant, &company, &id, &actor];
                if let Some(note) = &note {
                    args.push(note);
                }
                let row = db.query_one(&sql, &args).await?;
                let new: Value = row.get(0);
                let change = Change {
                    kind: table.name().into(),
                    id,
                    old,
                    new: new.clone(),
                };
                Ok((new, vec![change]))
            })
        })
        .await?;
    decode(value)
}
