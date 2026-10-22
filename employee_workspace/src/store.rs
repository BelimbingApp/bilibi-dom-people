//! Private SQL and validation, mirroring Bilimbi WorkProfile, Access,
//! ChangeRequest and SavedView changesets. Employee identity stays in Workforce.
use crate::*;
use bilibi_database::{Change, MutationTransaction};
use bilibi_people_workforce::FieldError;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tokio_postgres::{types::ToSql, GenericClient};

fn invalid(field: &'static str, message: impl Into<String>) -> WorkspaceError {
    WorkforceError::Invalid(vec![FieldError::new(field, message)]).into()
}
fn text(field: &'static str, value: &str, max: usize, required: bool) -> Result<String> {
    let value = value.trim();
    if required && value.is_empty() {
        return Err(invalid(field, "can't be blank"));
    }
    if value.chars().count() > max {
        return Err(invalid(
            field,
            format!("should be at most {max} character(s)"),
        ));
    }
    if value.contains('\0') {
        return Err(invalid(field, "is invalid"));
    }
    Ok(value.to_owned())
}
fn optional(field: &'static str, value: &Option<String>, max: usize) -> Result<Option<String>> {
    value
        .as_deref()
        .map(|s| text(field, s, max, false))
        .transpose()
        .map(|s| s.filter(|s| !s.is_empty()))
}
pub(crate) fn validate_profile(value: &WorkProfile) -> Result<WorkProfile> {
    Ok(WorkProfile {
        work_location: optional("work_location", &value.work_location, 200)?,
        work_arrangement: optional("work_arrangement", &value.work_arrangement, 120)?,
        notes: optional("notes", &value.notes, 2000)?,
    })
}
pub(crate) fn validate_access(value: &Access) -> Result<Access> {
    Ok(Access {
        portal_enabled: value.portal_enabled,
        reason: optional("reason", &value.reason, 2000)?,
    })
}
pub(crate) fn validate_request(value: &NewChangeRequest) -> Result<NewChangeRequest> {
    if !CHANGE_FIELDS.contains(&value.field.as_str()) {
        return Err(invalid("field", "is invalid"));
    }
    Ok(NewChangeRequest {
        field: value.field.clone(),
        proposed_value: text("proposed_value", &value.proposed_value, 500, true)?,
        reason: optional("reason", &value.reason, 2000)?,
    })
}
pub(crate) fn validate_view(value: &NewSavedView) -> Result<NewSavedView> {
    Ok(NewSavedView {
        name: text("name", &value.name, 120, true)?,
        search: optional("search", &value.search, 200)?,
        status: optional("status", &value.status, 40)?,
    })
}
fn decode<T: DeserializeOwned>(value: Value) -> std::result::Result<T, Error> {
    serde_json::from_value(value).map_err(|_| Error::Internal)
}
async fn one<T: DeserializeOwned>(
    client: &impl GenericClient,
    scope: &Scope,
    company: i64,
    employee: i64,
    table: &'static str,
) -> Result<Option<T>> {
    let sql = format!(
        "SELECT to_jsonb(t) FROM {table} t WHERE tenant_id=$1 AND company_id=$2 AND employee_id=$3"
    );
    client
        .query_opt(&sql, &[&scope.tenant(), &company, &employee])
        .await
        .map_err(Error::from)?
        .map(|row| decode(row.get(0)))
        .transpose()
        .map_err(Into::into)
}
pub(crate) async fn profile(
    client: &impl GenericClient,
    scope: &Scope,
    company: i64,
    employee: i64,
) -> Result<Option<WorkProfile>> {
    one(
        client,
        scope,
        company,
        employee,
        "people_employee_workspace_profiles",
    )
    .await
}
pub(crate) async fn access(
    client: &impl GenericClient,
    scope: &Scope,
    company: i64,
    employee: i64,
) -> Result<Option<Access>> {
    one(client, scope, company, employee, "people_employee_accesses").await
}
async fn record<T: DeserializeOwned + Default>(
    client: &impl GenericClient,
    scope: &Scope,
    company: i64,
    employee: i64,
    table: &'static str,
) -> Result<FactRecord<T>> {
    let raw = one::<Value>(client, scope, company, employee, table).await?;
    match raw {
        Some(raw) => Ok(FactRecord {
            id: raw["id"].as_i64(),
            version: raw["version"].as_i64().ok_or(Error::Internal)?,
            value: decode(raw)?,
        }),
        None => Ok(FactRecord {
            id: None,
            version: 0,
            value: T::default(),
        }),
    }
}
pub(crate) async fn profile_record(
    client: &impl GenericClient,
    scope: &Scope,
    company: i64,
    employee: i64,
) -> Result<FactRecord<WorkProfile>> {
    record(
        client,
        scope,
        company,
        employee,
        "people_employee_workspace_profiles",
    )
    .await
}
pub(crate) async fn access_record(
    client: &impl GenericClient,
    scope: &Scope,
    company: i64,
    employee: i64,
) -> Result<FactRecord<Access>> {
    record(client, scope, company, employee, "people_employee_accesses").await
}
pub(crate) async fn requests(
    client: &impl GenericClient,
    scope: &Scope,
    company: i64,
    employee: i64,
) -> Result<Vec<ChangeRequest>> {
    client.query("SELECT to_jsonb(t) FROM people_employee_change_requests t WHERE tenant_id=$1 AND company_id=$2 AND employee_id=$3 ORDER BY id DESC", &[&scope.tenant(), &company, &employee]).await.map_err(Error::from)?.into_iter().map(|r| decode(r.get(0)).map_err(Into::into)).collect()
}
pub(crate) async fn views(
    client: &impl GenericClient,
    scope: &Scope,
    company: i64,
) -> Result<Vec<SavedView>> {
    client.query("SELECT to_jsonb(t) FROM people_employee_saved_views t WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 ORDER BY name,id", &[&scope.tenant(), &company, &scope.actor().user()]).await.map_err(Error::from)?.into_iter().map(|r| decode(r.get(0)).map_err(Into::into)).collect()
}
// Only module-owned fixed statements call this helper. Owned parameters let the
// mutation future retain every value until both SQL and audit capture complete.
type Param = Box<dyn ToSql + Sync + Send>;
async fn write(
    tx: &mut MutationTransaction<'_>,
    sql: &'static str,
    args: Vec<Param>,
    kind: &'static str,
    old: Value,
    deleting: bool,
) -> Result<Value> {
    Ok(tx
        .mutate(move |client| {
            Box::pin(async move {
                let params = args
                    .iter()
                    .map(|p| p.as_ref() as &(dyn ToSql + Sync))
                    .collect::<Vec<_>>();
                let row = client.query_one(sql, &params).await?;
                let value: Value = row.get(0);
                let id = value["id"].as_i64().ok_or(Error::Internal)?;
                let change = Change {
                    kind: kind.into(),
                    id,
                    old: if deleting { value.clone() } else { old },
                    new: if deleting { json!({}) } else { value.clone() },
                };
                Ok((value, vec![change]))
            })
        })
        .await?)
}
async fn previous(
    tx: &MutationTransaction<'_>,
    scope: &Scope,
    company: i64,
    employee: i64,
    table: &'static str,
) -> Result<Value> {
    Ok(one::<Value>(tx.sql(), scope, company, employee, table)
        .await?
        .unwrap_or(json!({})))
}
pub(crate) async fn put_profile(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company: i64,
    employee: i64,
    value: WorkProfile,
) -> Result<WorkProfile> {
    let old = previous(
        tx,
        scope,
        company,
        employee,
        "people_employee_workspace_profiles",
    )
    .await?;
    let value = write(tx, "INSERT INTO people_employee_workspace_profiles AS t (tenant_id,company_id,employee_id,work_location,work_arrangement,notes) VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT (company_id,employee_id) DO UPDATE SET work_location=EXCLUDED.work_location,work_arrangement=EXCLUDED.work_arrangement,notes=EXCLUDED.notes,version=t.version+1,updated_at=now() AT TIME ZONE 'UTC' WHERE t.tenant_id=EXCLUDED.tenant_id RETURNING to_jsonb(t)", vec![Box::new(scope.tenant()),Box::new(company),Box::new(employee),Box::new(value.work_location),Box::new(value.work_arrangement),Box::new(value.notes)], "people_employee_workspace_profile", old, false).await?;
    Ok(decode(value)?)
}
pub(crate) async fn put_access(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company: i64,
    employee: i64,
    value: Access,
) -> Result<Access> {
    let old = previous(tx, scope, company, employee, "people_employee_accesses").await?;
    let value = write(tx, "INSERT INTO people_employee_accesses AS t (tenant_id,company_id,employee_id,portal_enabled,reason) VALUES ($1,$2,$3,$4,$5) ON CONFLICT (company_id,employee_id) DO UPDATE SET portal_enabled=EXCLUDED.portal_enabled,reason=EXCLUDED.reason,version=t.version+1,updated_at=now() AT TIME ZONE 'UTC' WHERE t.tenant_id=EXCLUDED.tenant_id RETURNING to_jsonb(t)", vec![Box::new(scope.tenant()),Box::new(company),Box::new(employee),Box::new(value.portal_enabled),Box::new(value.reason)], "people_employee_access", old, false).await?;
    Ok(decode(value)?)
}
pub(crate) async fn insert_request(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company: i64,
    employee: i64,
    value: NewChangeRequest,
) -> Result<ChangeRequest> {
    let value = write(tx, "INSERT INTO people_employee_change_requests AS t (tenant_id,company_id,employee_id,field,proposed_value,reason,requested_by_actor_id) VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING to_jsonb(t)", vec![Box::new(scope.tenant()),Box::new(company),Box::new(employee),Box::new(value.field),Box::new(value.proposed_value),Box::new(value.reason),Box::new(scope.actor().user())], "people_employee_change_request", json!({}), false).await?;
    Ok(decode(value)?)
}
pub(crate) async fn review_request(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company: i64,
    employee: i64,
    request_id: i64,
    status: &str,
) -> Result<ChangeRequest> {
    let row = tx.sql().query_opt("SELECT to_jsonb(t) FROM people_employee_change_requests t WHERE tenant_id=$1 AND company_id=$2 AND employee_id=$3 AND id=$4 FOR UPDATE", &[&scope.tenant(),&company,&employee,&request_id]).await.map_err(Error::from)?.ok_or(WorkforceError::NotFound)?;
    let old: Value = row.get(0);
    if old["status"] != "pending" {
        return Err(WorkspaceError::AlreadyReviewed);
    }
    let value = write(tx, "UPDATE people_employee_change_requests AS t SET status=$5,reviewed_by_actor_id=$6,reviewed_at=now() AT TIME ZONE 'UTC',updated_at=now() AT TIME ZONE 'UTC' WHERE tenant_id=$1 AND company_id=$2 AND employee_id=$3 AND id=$4 RETURNING to_jsonb(t)", vec![Box::new(scope.tenant()),Box::new(company),Box::new(employee),Box::new(request_id),Box::new(status.to_owned()),Box::new(scope.actor().user())], "people_employee_change_request", old, false).await?;
    Ok(decode(value)?)
}
pub(crate) async fn insert_view(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company: i64,
    value: NewSavedView,
) -> Result<SavedView> {
    if tx.sql().query_opt("SELECT id FROM people_employee_saved_views WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND name=$4", &[&scope.tenant(),&company,&scope.actor().user(),&value.name]).await.map_err(Error::from)?.is_some() { return Err(invalid("name", "has already been taken")); }
    let value = write(tx, "INSERT INTO people_employee_saved_views AS t (tenant_id,company_id,actor_id,name,search,status) VALUES ($1,$2,$3,$4,$5,$6) RETURNING to_jsonb(t)", vec![Box::new(scope.tenant()),Box::new(company),Box::new(scope.actor().user()),Box::new(value.name),Box::new(value.search),Box::new(value.status)], "people_employee_saved_view", json!({}), false).await?;
    Ok(decode(value)?)
}
pub(crate) async fn delete_view(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company: i64,
    view_id: i64,
) -> Result<()> {
    if tx.sql().query_opt("SELECT id FROM people_employee_saved_views WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND id=$4 FOR UPDATE", &[&scope.tenant(),&company,&scope.actor().user(),&view_id]).await.map_err(Error::from)?.is_none() { return Err(WorkforceError::NotFound.into()); }
    write(tx, "DELETE FROM people_employee_saved_views AS t WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND id=$4 RETURNING to_jsonb(t)", vec![Box::new(scope.tenant()),Box::new(company),Box::new(scope.actor().user()),Box::new(view_id)], "people_employee_saved_view", json!({}), true).await?;
    Ok(())
}
