//! Private, fixed table metadata from Bilimbi Performance.
#[derive(Clone, Copy)]
pub(crate) enum Table {
    Description,
    Definition,
    Target,
    Observation,
    Review,
    ReviewObservation,
    ReviewTarget,
    Response,
}
impl Table {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Description => "people_performance_descriptions",
            Self::Definition => "people_performance_kpi_definitions",
            Self::Target => "people_performance_kpi_targets",
            Self::Observation => "people_performance_evidence",
            Self::Review => "people_performance_appraisals",
            Self::ReviewObservation => "people_performance_review_evidence",
            Self::ReviewTarget => "people_performance_review_targets",
            Self::Response => "people_performance_responses",
        }
    }
    pub(crate) fn insert_sql(self) -> &'static str {
        match self {
Self::Description => "INSERT INTO people_performance_descriptions AS t (tenant_id, company_id, actor_user_id, code, version, position_id, position_version, effective_from, effective_to, purpose, responsibilities, duties, authority, qualifications, competency_links, status, published_at, published_by_user_id) VALUES ($1, $2, $3, $4::jsonb->>'code', ($4::jsonb->>'version')::text::integer, ($4::jsonb->>'position_id')::text::bigint, ($4::jsonb->>'position_version')::text::integer, ($4::jsonb->>'effective_from')::text::date, ($4::jsonb->>'effective_to')::text::date, $4::jsonb->>'purpose', $4::jsonb->>'responsibilities', $4::jsonb->>'duties', $4::jsonb->>'authority', $4::jsonb->>'qualifications', $4::jsonb->'competency_links', $4::jsonb->>'status', (($4::jsonb->>'published_at')::timestamptz AT TIME ZONE 'UTC'), ($4::jsonb->>'published_by_user_id')::text::bigint) RETURNING ",
Self::Definition => "INSERT INTO people_performance_kpi_definitions AS t (tenant_id, company_id, actor_user_id, code, version, name, purpose, unit, measure, source_reference, calculation_version, direction, rubric, precision, interpretation) VALUES ($1, $2, $3, $4::jsonb->>'code', ($4::jsonb->>'version')::text::integer, $4::jsonb->>'name', $4::jsonb->>'purpose', $4::jsonb->>'unit', $4::jsonb->>'measure', $4::jsonb->>'source_reference', $4::jsonb->>'calculation_version', $4::jsonb->>'direction', $4::jsonb->>'rubric', ($4::jsonb->>'precision')::text::integer, $4::jsonb->>'interpretation') RETURNING ",
Self::Target => "INSERT INTO people_performance_kpi_targets AS t (tenant_id, company_id, actor_user_id, definition_id, definition_version, employee_id, target, period_start, period_end, effective_from, version, supersedes_id, change_reason, confidential, status, review_note, reviewed_by_user_id, published_by_user_id, published_at) VALUES ($1, $2, $3, ($4::jsonb->>'definition_id')::text::bigint, ($4::jsonb->>'definition_version')::text::integer, ($4::jsonb->>'employee_id')::text::bigint, $4::jsonb->>'target', ($4::jsonb->>'period_start')::text::date, ($4::jsonb->>'period_end')::text::date, ($4::jsonb->>'effective_from')::text::date, ($4::jsonb->>'version')::text::integer, ($4::jsonb->>'supersedes_id')::text::bigint, $4::jsonb->>'change_reason', ($4::jsonb->>'confidential')::text::boolean, $4::jsonb->>'status', $4::jsonb->>'review_note', ($4::jsonb->>'reviewed_by_user_id')::text::bigint, ($4::jsonb->>'published_by_user_id')::text::bigint, (($4::jsonb->>'published_at')::timestamptz AT TIME ZONE 'UTC')) RETURNING ",
Self::Observation => "INSERT INTO people_performance_evidence AS t (tenant_id, company_id, actor_user_id, employee_id, window_start, window_end, evidence, source_reference, source_version, supersedes_id, change_reason) VALUES ($1, $2, $3, ($4::jsonb->>'employee_id')::text::bigint, ($4::jsonb->>'window_start')::text::date, ($4::jsonb->>'window_end')::text::date, $4::jsonb->>'evidence', $4::jsonb->>'source_reference', $4::jsonb->>'source_version', ($4::jsonb->>'supersedes_id')::text::bigint, $4::jsonb->>'change_reason') RETURNING ",
Self::Review => "INSERT INTO people_performance_appraisals AS t (tenant_id, company_id, actor_user_id, employee_id, description_id, period_start, period_end, cutoff_at, outcome, rationale, version, supersedes_id, change_reason, status, released_at, released_by_user_id) VALUES ($1, $2, $3, ($4::jsonb->>'employee_id')::text::bigint, ($4::jsonb->>'description_id')::text::bigint, ($4::jsonb->>'period_start')::text::date, ($4::jsonb->>'period_end')::text::date, (($4::jsonb->>'cutoff_at')::timestamptz AT TIME ZONE 'UTC'), $4::jsonb->>'outcome', $4::jsonb->>'rationale', ($4::jsonb->>'version')::text::integer, ($4::jsonb->>'supersedes_id')::text::bigint, $4::jsonb->>'change_reason', $4::jsonb->>'status', (($4::jsonb->>'released_at')::timestamptz AT TIME ZONE 'UTC'), ($4::jsonb->>'released_by_user_id')::text::bigint) RETURNING ",
Self::ReviewObservation => "INSERT INTO people_performance_review_evidence AS t (tenant_id, company_id, actor_user_id, review_id, observation_id) VALUES ($1, $2, $3, ($4::jsonb->>'review_id')::text::bigint, ($4::jsonb->>'observation_id')::text::bigint) RETURNING ",
Self::ReviewTarget => "INSERT INTO people_performance_review_targets AS t (tenant_id, company_id, actor_user_id, review_id, target_id) VALUES ($1, $2, $3, ($4::jsonb->>'review_id')::text::bigint, ($4::jsonb->>'target_id')::text::bigint) RETURNING ",
Self::Response => "INSERT INTO people_performance_responses AS t (tenant_id, company_id, actor_user_id, review_id, employee_id, response) VALUES ($1, $2, $3, ($4::jsonb->>'review_id')::text::bigint, ($4::jsonb->>'employee_id')::text::bigint, $4::jsonb->>'response') RETURNING ",
}
    }
    pub(crate) fn times(self) -> &'static [&'static str] {
        match self {
            Self::Description => &["inserted_at", "updated_at", "published_at"],
            Self::Definition => &["inserted_at", "updated_at"],
            Self::Target => &["inserted_at", "updated_at", "published_at"],
            Self::Observation => &["inserted_at", "updated_at"],
            Self::Review => &["inserted_at", "updated_at", "cutoff_at", "released_at"],
            Self::ReviewObservation => &["inserted_at", "updated_at"],
            Self::ReviewTarget => &["inserted_at", "updated_at"],
            Self::Response => &["inserted_at", "updated_at"],
        }
    }
    pub(crate) fn dates(self) -> &'static [&'static str] {
        match self {
            Self::Description => &["effective_from", "effective_to"],
            Self::Definition => &[],
            Self::Target => &["period_start", "period_end", "effective_from"],
            Self::Observation => &["window_start", "window_end"],
            Self::Review => &["period_start", "period_end"],
            Self::ReviewObservation => &[],
            Self::ReviewTarget => &[],
            Self::Response => &[],
        }
    }
}
