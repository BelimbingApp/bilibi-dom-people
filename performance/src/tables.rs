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
Self::Description => "INSERT INTO people_performance_descriptions AS t (tenant_id, company_id, actor_user_id, code, version, position_id, position_version, effective_from, effective_to, purpose, responsibilities, duties, authority, qualifications, competency_links, status, published_at, published_by_user_id) VALUES ($1, $2, $3, $4->>'code', ($4->>'version')::text::integer, ($4->>'position_id')::text::bigint, ($4->>'position_version')::text::integer, ($4->>'effective_from')::text::date, ($4->>'effective_to')::text::date, $4->>'purpose', $4->>'responsibilities', $4->>'duties', $4->>'authority', $4->>'qualifications', $4->'competency_links', $4->>'status', (($4->>'published_at')::timestamptz AT TIME ZONE 'UTC'), ($4->>'published_by_user_id')::text::bigint) RETURNING ",
Self::Definition => "INSERT INTO people_performance_kpi_definitions AS t (tenant_id, company_id, actor_user_id, code, version, name, purpose, unit, measure, source_reference, calculation_version, direction, rubric, precision, interpretation) VALUES ($1, $2, $3, $4->>'code', ($4->>'version')::text::integer, $4->>'name', $4->>'purpose', $4->>'unit', $4->>'measure', $4->>'source_reference', $4->>'calculation_version', $4->>'direction', $4->>'rubric', ($4->>'precision')::text::integer, $4->>'interpretation') RETURNING ",
Self::Target => "INSERT INTO people_performance_kpi_targets AS t (tenant_id, company_id, actor_user_id, definition_id, definition_version, employee_id, target, period_start, period_end, effective_from, version, supersedes_id, change_reason, confidential, status, review_note, reviewed_by_user_id, published_by_user_id, published_at) VALUES ($1, $2, $3, ($4->>'definition_id')::text::bigint, ($4->>'definition_version')::text::integer, ($4->>'employee_id')::text::bigint, $4->>'target', ($4->>'period_start')::text::date, ($4->>'period_end')::text::date, ($4->>'effective_from')::text::date, ($4->>'version')::text::integer, ($4->>'supersedes_id')::text::bigint, $4->>'change_reason', ($4->>'confidential')::text::boolean, $4->>'status', $4->>'review_note', ($4->>'reviewed_by_user_id')::text::bigint, ($4->>'published_by_user_id')::text::bigint, (($4->>'published_at')::timestamptz AT TIME ZONE 'UTC')) RETURNING ",
Self::Observation => "INSERT INTO people_performance_evidence AS t (tenant_id, company_id, actor_user_id, employee_id, window_start, window_end, evidence, source_reference, source_version, supersedes_id, change_reason) VALUES ($1, $2, $3, ($4->>'employee_id')::text::bigint, ($4->>'window_start')::text::date, ($4->>'window_end')::text::date, $4->>'evidence', $4->>'source_reference', $4->>'source_version', ($4->>'supersedes_id')::text::bigint, $4->>'change_reason') RETURNING ",
Self::Review => "INSERT INTO people_performance_appraisals AS t (tenant_id, company_id, actor_user_id, employee_id, description_id, period_start, period_end, cutoff_at, outcome, rationale, version, supersedes_id, change_reason, status, released_at, released_by_user_id) VALUES ($1, $2, $3, ($4->>'employee_id')::text::bigint, ($4->>'description_id')::text::bigint, ($4->>'period_start')::text::date, ($4->>'period_end')::text::date, (($4->>'cutoff_at')::timestamptz AT TIME ZONE 'UTC'), $4->>'outcome', $4->>'rationale', ($4->>'version')::text::integer, ($4->>'supersedes_id')::text::bigint, $4->>'change_reason', $4->>'status', (($4->>'released_at')::timestamptz AT TIME ZONE 'UTC'), ($4->>'released_by_user_id')::text::bigint) RETURNING ",
Self::ReviewObservation => "INSERT INTO people_performance_review_evidence AS t (tenant_id, company_id, actor_user_id, review_id, observation_id) VALUES ($1, $2, $3, ($4->>'review_id')::text::bigint, ($4->>'observation_id')::text::bigint) RETURNING ",
Self::ReviewTarget => "INSERT INTO people_performance_review_targets AS t (tenant_id, company_id, actor_user_id, review_id, target_id) VALUES ($1, $2, $3, ($4->>'review_id')::text::bigint, ($4->>'target_id')::text::bigint) RETURNING ",
Self::Response => "INSERT INTO people_performance_responses AS t (tenant_id, company_id, actor_user_id, review_id, employee_id, response) VALUES ($1, $2, $3, ($4->>'review_id')::text::bigint, ($4->>'employee_id')::text::bigint, $4->>'response') RETURNING ",
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
