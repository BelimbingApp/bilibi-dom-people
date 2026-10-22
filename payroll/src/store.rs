//! Private Payroll SQL. Identifiers come only from Kind; all input is bound.
use crate::{PayrollError, Result};
use bilibi_database::{Change, Error, MutationTransaction};
use bilibi_tenancy::Scope;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tokio_postgres::GenericClient;
#[derive(Clone, Copy)]
pub(crate) enum Kind {
    Classification,
    Item,
    Period,
    Mapping,
    AttendanceMapping,
    Run,
}
impl Kind {
    pub(crate) fn table(self) -> &'static str {
        match self {
            Self::Classification => "people_payroll_classifications",
            Self::Item => "people_payroll_items",
            Self::Period => "people_payroll_pay_windows",
            Self::Mapping => "people_payroll_mappings",
            Self::AttendanceMapping => "people_payroll_allowance_item_mappings",
            Self::Run => "people_payroll_setup_snapshots",
        }
    }
    fn fields(self) -> &'static [&'static str] {
        match self {
            Self::Classification => &["code", "name", "effective_from", "effective_to"],
            Self::Item => &[
                "code",
                "name",
                "classification_id",
                "currency",
                "amount",
                "effective_from",
                "effective_to",
            ],
            Self::Period => &["code", "starts_on", "ends_on", "pay_on"],
            Self::Mapping => &[
                "source_kind",
                "source_key",
                "item_id",
                "effective_from",
                "effective_to",
            ],
            Self::AttendanceMapping => &[
                "attendance_rule_code",
                "item_id",
                "effective_from",
                "effective_to",
            ],
            Self::Run => &["period_id", "country", "currency", "snapshot"],
        }
    }
    fn projection(self) -> String {
        let base = "to_jsonb(t) - 'tenant_id' - 'company_id'";
        match self {
            Self::Period => format!("{base} || jsonb_build_object('starts_on',to_char(t.starts_on,'YYYY-MM-DD'),'ends_on',to_char(t.ends_on,'YYYY-MM-DD'),'pay_on',to_char(t.pay_on,'YYYY-MM-DD'))"),
            Self::Run => format!("{base} || jsonb_build_object('locked_at',to_char(t.locked_at,'YYYY-MM-DD\"T\"HH24:MI:SS'))"),
            _ => format!("{base} || jsonb_build_object('effective_from',to_char(t.effective_from,'YYYY-MM-DD'),'effective_to',to_char(t.effective_to,'YYYY-MM-DD')){}", if matches!(self, Self::Item) { " || jsonb_build_object('amount',t.amount::text)" } else { "" }),
        }
    }
}
pub(crate) fn decode<T: DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|_| PayrollError::Failed(Error::Internal))
}
pub(crate) async fn list<T: DeserializeOwned>(
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company: i64,
    kind: Kind,
) -> Result<Vec<T>> {
    client
        .query(
            &format!(
                "SELECT {} FROM {} t WHERE tenant_id=$1 AND company_id=$2 ORDER BY id",
                kind.projection(),
                kind.table()
            ),
            &[&scope.tenant(), &company],
        )
        .await?
        .into_iter()
        .map(|r| decode(r.get(0)))
        .collect()
}
pub(crate) async fn get<T: DeserializeOwned>(
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company: i64,
    kind: Kind,
    id: i64,
) -> Result<Option<T>> {
    client
        .query_opt(
            &format!(
                "SELECT {} FROM {} t WHERE tenant_id=$1 AND company_id=$2 AND id=$3",
                kind.projection(),
                kind.table()
            ),
            &[&scope.tenant(), &company, &id],
        )
        .await?
        .map(|r| decode(r.get(0)))
        .transpose()
}
pub(crate) async fn insert<T: DeserializeOwned>(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company: i64,
    kind: Kind,
    data: Value,
) -> Result<T> {
    let fields = kind.fields();
    let expressions = fields
        .iter()
        .map(|field| {
            let cast = match *field {
                "effective_from" | "effective_to" | "starts_on" | "ends_on" | "pay_on" => {
                    "::text::date"
                }
                "classification_id" | "item_id" | "period_id" => "::bigint",
                "amount" => "::numeric",
                _ => "",
            };
            if *field == "snapshot" {
                "($4::jsonb->'snapshot')".to_owned()
            } else {
                format!("($4::jsonb->>'{field}'){cast}")
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    let statement=format!("INSERT INTO {} AS t (tenant_id,company_id,created_by_actor_id,{}) VALUES ($1,$2,$3,{expressions}) RETURNING {}",kind.table(),fields.join(","),kind.projection());
    let tenant = scope.tenant();
    let actor = scope.actor().user();
    let value = tx
        .mutate(move |sql| {
            Box::pin(async move {
                let value: Value = sql
                    .query_one(&statement, &[&tenant, &company, &actor, &data])
                    .await?
                    .get(0);
                let id = value["id"].as_i64().ok_or(Error::Internal)?;
                Ok((
                    value.clone(),
                    vec![Change {
                        kind: kind.table().into(),
                        id,
                        old: json!({}),
                        new: value,
                    }],
                ))
            })
        })
        .await?;
    decode(value)
}
pub(crate) async fn lock_run(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company: i64,
    id: i64,
) -> Result<crate::Run> {
    let before: Value=tx.sql().query_opt(&format!("SELECT {} FROM people_payroll_setup_snapshots t WHERE tenant_id=$1 AND company_id=$2 AND id=$3 FOR UPDATE",Kind::Run.projection()),&[&scope.tenant(),&company,&id]).await?.ok_or(PayrollError::NotFound)?.get(0);
    if !before["locked_at"].is_null() {
        return Err(PayrollError::Locked);
    }
    let tenant = scope.tenant();
    let actor = scope.actor().user();
    let after=tx.mutate(move |sql| Box::pin(async move {
        let after: Value=sql.query_one(&format!("UPDATE people_payroll_setup_snapshots t SET locked_at=now() AT TIME ZONE 'UTC',locked_by_actor_id=$4,updated_at=now() AT TIME ZONE 'UTC' WHERE tenant_id=$1 AND company_id=$2 AND id=$3 RETURNING {}",Kind::Run.projection()),&[&tenant,&company,&id,&actor]).await?.get(0);
        Ok((after.clone(),vec![Change{kind:Kind::Run.table().into(),id,old:before,new:after}]))
    })).await?;
    decode(after)
}
