//! Private SQL for Bilimbi Training Evaluation; callers use the Training facade.
use crate::{governance_store::decode, *};
use bilibi_database::{Change, MutationTransaction};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tokio_postgres::types::ToSql;

#[derive(Clone, Copy)]
pub(super) enum Table {
    Policy,
    Review,
    Answer,
    Reminder,
    Summary,
}
impl Table {
    fn name(self) -> &'static str {
        match self {
            Self::Policy => "people_training_evaluation_policies",
            Self::Review => "people_training_evaluation_reviews",
            Self::Answer => "people_training_evaluation_answers",
            Self::Reminder => "people_training_review_reminders",
            Self::Summary => "people_training_effectiveness_summaries",
        }
    }
    fn projection(self) -> &'static str {
        match self {
            Self::Policy => "to_jsonb(r) || jsonb_build_object('effective_from',to_char(r.effective_from,'YYYY-MM-DD'),'effective_to',to_char(r.effective_to,'YYYY-MM-DD'))",
            Self::Review => "to_jsonb(r) || jsonb_build_object('due_on',to_char(r.due_on,'YYYY-MM-DD'))",
            Self::Reminder => "to_jsonb(r) || jsonb_build_object('available_on',to_char(r.available_on,'YYYY-MM-DD'))",
            Self::Summary => "to_jsonb(r) || jsonb_build_object('period_start',to_char(r.period_start,'YYYY-MM-DD'),'period_end',to_char(r.period_end,'YYYY-MM-DD'))",
            Self::Answer => "to_jsonb(r)",
        }
    }
    fn kind(self) -> &'static str {
        match self {
            Self::Policy => "people_training_evaluation_policy",
            Self::Review => "people_training_evaluation_review",
            Self::Answer => "people_training_evaluation_answer",
            Self::Reminder => "people_training_review_reminder",
            Self::Summary => "people_training_effectiveness_summary",
        }
    }
    fn columns(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Policy => &[
                ("version", "integer"),
                ("effective_from", "date"),
                ("effective_to", "date"),
                ("criteria", "jsonb"),
                ("effectiveness_criteria", "jsonb"),
                ("evaluation_days", "integer"),
                ("checkpoints", "jsonb"),
                ("reminder_days", "integer"),
                ("reason", "text"),
            ],
            Self::Review => &[
                ("policy_id", "bigint"),
                ("fact_id", "bigint"),
                ("kind", "text"),
                ("checkpoint_days", "integer"),
                ("due_on", "date"),
            ],
            Self::Answer => &[
                ("review_id", "bigint"),
                ("values", "jsonb"),
                ("reason", "text"),
            ],
            Self::Reminder => &[
                ("review_id", "bigint"),
                ("recipient_employee_id", "bigint"),
                ("available_on", "date"),
            ],
            Self::Summary => &[
                ("period_start", "date"),
                ("period_end", "date"),
                ("minimum_cohort", "integer"),
                ("status", "text"),
                ("groups", "jsonb"),
            ],
        }
    }
}
pub(super) async fn rows<T: DeserializeOwned>(
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
pub(super) async fn fetch<T: DeserializeOwned>(
    client: &impl GenericClient,
    scope: &Scope,
    company: i64,
    table: Table,
    id: i64,
) -> Result<T> {
    rows(client, scope, company, table, "AND r.id=$3", &[&id])
        .await?
        .pop()
        .ok_or(TrainingError::NotFound)
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
    let names = fields
        .iter()
        .map(|(n, _)| format!("\"{n}\""))
        .collect::<Vec<_>>()
        .join(",");
    let values = fields
        .iter()
        .map(|(n, t)| {
            if *t == "jsonb" {
                format!("$1::jsonb->'{n}'")
            } else {
                format!("($1::jsonb->>'{n}')::{t}")
            }
        })
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
                Ok((
                    value.clone(),
                    vec![Change {
                        kind: table.kind().into(),
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
/// All joins and latest-attendance checks carry both tenant and company axes.
/// A confirmed correction preserves the original obligation; a latest absent
/// revision hides it until attendance is confirmed again (Bilimbi parity).
pub(super) async fn tasks(
    client: &impl GenericClient,
    scope: &Scope,
    company: i64,
    kind: Option<EvaluationKind>,
    ids: Option<&[i64]>,
    review_id: Option<i64>,
) -> Result<Vec<EvaluationTask>> {
    let kind = kind.map(EvaluationKind::as_str);
    let ids = ids.map(<[i64]>::to_vec);
    client.query("SELECT to_jsonb(r) || jsonb_build_object('due_on',to_char(r.due_on,'YYYY-MM-DD'),'employee_id',f.employee_id,'session_id',f.session_id,'criteria',CASE WHEN r.kind='evaluation' THEN p.criteria->'items' ELSE p.effectiveness_criteria->'items' END,'policy_version',p.version,'answer',to_jsonb(a),'reminder',CASE WHEN m.id IS NULL THEN NULL ELSE jsonb_build_object('available_on',to_char(m.available_on,'YYYY-MM-DD')) END) AS value
        FROM people_training_evaluation_reviews r
        JOIN people_training_attendance_facts f ON f.id=r.fact_id AND f.tenant_id=r.tenant_id AND f.company_id=r.company_id
        JOIN people_training_evaluation_policies p ON p.id=r.policy_id AND p.tenant_id=r.tenant_id AND p.company_id=r.company_id
        LEFT JOIN people_training_evaluation_answers a ON a.review_id=r.id AND a.tenant_id=r.tenant_id AND a.company_id=r.company_id
        LEFT JOIN people_training_review_reminders m ON m.review_id=r.id AND m.tenant_id=r.tenant_id AND m.company_id=r.company_id
        WHERE r.tenant_id=$1 AND r.company_id=$2 AND ($3::text IS NULL OR r.kind=$3) AND ($4::bigint[] IS NULL OR f.employee_id=ANY($4)) AND ($5::bigint IS NULL OR r.id=$5)
        AND (SELECT latest.status FROM people_training_attendance_facts latest WHERE latest.tenant_id=r.tenant_id AND latest.company_id=r.company_id AND latest.session_id=f.session_id AND latest.employee_id=f.employee_id ORDER BY latest.revision DESC LIMIT 1)='confirmed'
        ORDER BY r.due_on,r.id",&[&scope.tenant(),&company,&kind,&ids,&review_id]).await.map_err(Error::from)?.into_iter().map(|r|decode(r.get("value"))).collect()
}
