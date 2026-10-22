-- Skills 002: Bilimbi AddAssessments (20260930220137), eight workflow tables.
-- Table definitions come from the pinned reference schema; guard functions
-- mirror its migration. Employee IDs name People employments.
-- The later reassessment, action and reminder slices use these same tables.

CREATE TABLE people_skill_action_events (
    id bigserial PRIMARY KEY,
    tenant_id bigint NOT NULL,
    company_id bigint NOT NULL,
    action_id bigint NOT NULL,
    event_type character varying(32) NOT NULL,
    from_status character varying(24),
    to_status character varying(24),
    comment character varying(2000),
    evidence character varying(2000),
    actor_user_id bigint NOT NULL,
    inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC')
);

CREATE TABLE people_skill_action_types (
    id bigserial PRIMARY KEY,
    tenant_id bigint NOT NULL,
    company_id bigint NOT NULL,
    code character varying(80) NOT NULL,
    name character varying(160) NOT NULL,
    requires_provider boolean NOT NULL,
    active boolean NOT NULL,
    inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC')
);

CREATE TABLE people_skill_actions (
    id bigserial PRIMARY KEY,
    tenant_id bigint NOT NULL,
    company_id bigint NOT NULL,
    employee_id bigint NOT NULL,
    employee_name character varying(200) NOT NULL,
    skill_id bigint NOT NULL,
    source_assessment_id bigint,
    action_type_id bigint NOT NULL,
    starting_level integer NOT NULL,
    target_level integer NOT NULL,
    gap_at_start integer NOT NULL,
    criticality character varying(16) NOT NULL,
    mandatory boolean NOT NULL,
    priority_multiplier integer NOT NULL,
    priority_score integer NOT NULL,
    priority_explanation character varying(300) NOT NULL,
    manual_reason character varying(1000),
    objective character varying(2000) NOT NULL,
    intervention character varying(2000) NOT NULL,
    expected_evidence character varying(2000) NOT NULL,
    owner_employee_id bigint NOT NULL,
    coordinator_employee_id bigint NOT NULL,
    provider_employee_id bigint,
    provider_name character varying(160),
    start_on date NOT NULL,
    due_on date NOT NULL,
    status character varying(24) NOT NULL,
    closure character varying(24) NOT NULL,
    approved_by_user_id bigint,
    approved_at timestamp(0) without time zone,
    completed_at timestamp(0) without time zone,
    completion_evidence character varying(2000),
    reassessment_due_on date,
    post_assessment_id bigint,
    post_level integer,
    improvement integer,
    next_steps character varying(2000),
    created_by_user_id bigint NOT NULL,
    request_key character varying(80) NOT NULL,
    inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    CONSTRAINT people_skill_actions_basis CHECK (((source_assessment_id IS NOT NULL) OR (manual_reason IS NOT NULL))),
    CONSTRAINT people_skill_actions_closure CHECK (((closure)::text = ANY ((ARRAY['open'::character varying, 'pending_reassessment'::character varying, 'closed_competent'::character varying, 'further_action_required'::character varying, 'cancelled'::character varying])::text[]))),
    CONSTRAINT people_skill_actions_criticality CHECK (((criticality)::text = ANY ((ARRAY['critical'::character varying, 'essential'::character varying, 'development'::character varying])::text[]))),
    CONSTRAINT people_skill_actions_dates CHECK ((due_on >= start_on)),
    CONSTRAINT people_skill_actions_levels CHECK ((((starting_level >= 0) AND (starting_level <= 20)) AND ((target_level >= 0) AND (target_level <= 20)) AND (gap_at_start = GREATEST((target_level - starting_level), 0)) AND (priority_multiplier >= 0) AND (priority_score = (gap_at_start * priority_multiplier)))),
    CONSTRAINT people_skill_actions_status CHECK (((status)::text = ANY ((ARRAY['proposed'::character varying, 'scheduled'::character varying, 'not_started'::character varying, 'in_progress'::character varying, 'on_hold'::character varying, 'pending_reassessment'::character varying, 'completed'::character varying, 'cancelled'::character varying])::text[]))),
    CONSTRAINT people_skill_actions_terminal CHECK (((((status)::text = 'completed'::text) AND ((closure)::text = ANY ((ARRAY['closed_competent'::character varying, 'further_action_required'::character varying])::text[])) AND (post_assessment_id IS NOT NULL)) OR (((status)::text = 'cancelled'::text) AND ((closure)::text = 'cancelled'::text)) OR (((status)::text = 'pending_reassessment'::text) AND ((closure)::text = 'pending_reassessment'::text) AND (completed_at IS NOT NULL)) OR (((status)::text <> ALL ((ARRAY['completed'::character varying, 'cancelled'::character varying, 'pending_reassessment'::character varying])::text[])) AND ((closure)::text = 'open'::text))))
);

CREATE TABLE people_skill_assessment_decisions (
    id bigserial PRIMARY KEY,
    tenant_id bigint NOT NULL,
    company_id bigint NOT NULL,
    assessment_id bigint NOT NULL,
    decision character varying(16) NOT NULL,
    actor_user_id bigint NOT NULL,
    note character varying(2000),
    inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    CONSTRAINT people_skill_assessment_decisions_decision CHECK (((decision)::text = ANY ((ARRAY['submitted'::character varying, 'verified'::character varying, 'returned'::character varying, 'finalized'::character varying])::text[])))
);

CREATE TABLE people_skill_assessments (
    id bigserial PRIMARY KEY,
    tenant_id bigint NOT NULL,
    company_id bigint NOT NULL,
    employee_id bigint NOT NULL,
    skill_id bigint NOT NULL,
    profile_id bigint NOT NULL,
    scale_id bigint NOT NULL,
    required_level integer NOT NULL,
    criticality character varying(16) NOT NULL,
    weight_percent numeric(5,2) NOT NULL,
    mandatory boolean NOT NULL,
    assessed_level integer NOT NULL,
    gap integer NOT NULL,
    priority_multiplier integer NOT NULL,
    priority_score integer NOT NULL,
    result_band character varying(16) NOT NULL,
    method character varying(80),
    evidence character varying(4000) NOT NULL,
    notes character varying(2000),
    assessed_on date NOT NULL,
    valid_until date,
    next_due_on date NOT NULL,
    status character varying(16) NOT NULL,
    assessor_user_id bigint NOT NULL,
    reviewed_by_user_id bigint,
    reviewed_at timestamp(0) without time zone,
    review_note character varying(2000),
    finalized_by_user_id bigint,
    finalized_at timestamp(0) without time zone,
    supersedes_assessment_id bigint,
    request_key character varying(80) NOT NULL,
    inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    CONSTRAINT people_skill_assessments_band CHECK (((result_band)::text = ANY ((ARRAY['exceeds'::character varying, 'meets'::character varying, 'minor_gap'::character varying, 'major_gap'::character varying, 'critical_gap'::character varying])::text[]))),
    CONSTRAINT people_skill_assessments_criticality CHECK (((criticality)::text = ANY ((ARRAY['critical'::character varying, 'essential'::character varying, 'development'::character varying])::text[]))),
    CONSTRAINT people_skill_assessments_dates CHECK (((valid_until IS NULL) OR (valid_until >= assessed_on))),
    CONSTRAINT people_skill_assessments_evidence CHECK ((length(btrim((evidence)::text)) > 0)),
    CONSTRAINT people_skill_assessments_independent CHECK ((((reviewed_by_user_id IS NULL) OR (reviewed_by_user_id <> assessor_user_id)) AND ((finalized_by_user_id IS NULL) OR (finalized_by_user_id <> assessor_user_id)))),
    CONSTRAINT people_skill_assessments_levels CHECK ((((required_level >= 0) AND (required_level <= 20)) AND ((assessed_level >= 0) AND (assessed_level <= 20)) AND (gap = GREATEST((required_level - assessed_level), 0)) AND (priority_multiplier >= 0) AND (priority_score = (gap * priority_multiplier)))),
    CONSTRAINT people_skill_assessments_status CHECK (((status)::text = ANY ((ARRAY['pending_review'::character varying, 'verified'::character varying, 'returned'::character varying, 'finalized'::character varying])::text[]))),
    CONSTRAINT people_skill_assessments_workflow CHECK (((((status)::text = 'pending_review'::text) AND (reviewed_at IS NULL) AND (finalized_at IS NULL)) OR (((status)::text = 'verified'::text) AND (reviewed_at IS NOT NULL) AND (reviewed_by_user_id IS NOT NULL) AND (finalized_at IS NULL)) OR (((status)::text = 'returned'::text) AND (reviewed_at IS NOT NULL) AND (reviewed_by_user_id IS NOT NULL) AND (review_note IS NOT NULL) AND (finalized_at IS NULL)) OR (((status)::text = 'finalized'::text) AND (reviewed_at IS NOT NULL) AND (reviewed_by_user_id IS NOT NULL) AND (finalized_at IS NOT NULL) AND (finalized_by_user_id IS NOT NULL))))
);

CREATE TABLE people_skill_reassessment_requests (
    id bigserial PRIMARY KEY,
    tenant_id bigint NOT NULL,
    company_id bigint NOT NULL,
    employee_id bigint NOT NULL,
    skill_id bigint NOT NULL,
    reason character varying(1000) NOT NULL,
    requested_by_user_id bigint CONSTRAINT people_skill_reassessment_request_requested_by_user_id_not_null NOT NULL,
    due_on date NOT NULL,
    status character varying(12) NOT NULL,
    performed_by_user_id bigint,
    performed_at timestamp(0) without time zone,
    assessment_id bigint,
    cancelled_by_user_id bigint,
    cancelled_at timestamp(0) without time zone,
    inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    CONSTRAINT people_skill_reassessment_requests_state CHECK (((((status)::text = 'pending'::text) AND (performed_at IS NULL) AND (cancelled_at IS NULL)) OR (((status)::text = 'performed'::text) AND (performed_at IS NOT NULL) AND (performed_by_user_id IS NOT NULL) AND (assessment_id IS NOT NULL) AND (cancelled_at IS NULL)) OR (((status)::text = 'cancelled'::text) AND (cancelled_at IS NOT NULL) AND (cancelled_by_user_id IS NOT NULL) AND (performed_at IS NULL))))
);

CREATE TABLE people_skill_reminders (
    id bigserial PRIMARY KEY,
    tenant_id bigint NOT NULL,
    company_id bigint NOT NULL,
    rule character varying(24) NOT NULL,
    employee_id bigint,
    skill_id bigint NOT NULL,
    action_id bigint,
    period_key character varying(12) NOT NULL,
    recipient_user_id bigint NOT NULL,
    due_on date NOT NULL,
    state character varying(12) NOT NULL,
    failure character varying(300),
    sent_at timestamp(0) without time zone,
    inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    CONSTRAINT people_skill_reminders_rule CHECK (((rule)::text = ANY ((ARRAY['overdue_reassessment'::character varying, 'expiring_certificate'::character varying, 'overdue_action'::character varying, 'coverage_gap'::character varying])::text[]))),
    CONSTRAINT people_skill_reminders_sent CHECK ((((state)::text <> 'sent'::text) OR (sent_at IS NOT NULL))),
    CONSTRAINT people_skill_reminders_state CHECK (((state)::text = ANY ((ARRAY['pending'::character varying, 'sent'::character varying, 'failed'::character varying])::text[])))
);

CREATE TABLE people_skill_scores (
    id bigserial PRIMARY KEY,
    tenant_id bigint NOT NULL,
    company_id bigint NOT NULL,
    employee_id bigint NOT NULL,
    skill_id bigint NOT NULL,
    assessment_id bigint NOT NULL,
    profile_id bigint NOT NULL,
    required_level integer NOT NULL,
    current_level integer NOT NULL,
    gap integer NOT NULL,
    criticality character varying(16) NOT NULL,
    mandatory boolean NOT NULL,
    priority_score integer NOT NULL,
    assessed_on date NOT NULL,
    valid_until date,
    next_due_on date NOT NULL,
    inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    CONSTRAINT people_skill_scores_gap CHECK ((gap = GREATEST((required_level - current_level), 0)))
);

ALTER TABLE people_skill_action_events
    ADD CONSTRAINT people_skill_action_events_action_id_fkey FOREIGN KEY (action_id) REFERENCES people_skill_actions(id) ON DELETE RESTRICT;

ALTER TABLE people_skill_actions
    ADD CONSTRAINT people_skill_actions_action_type_id_fkey FOREIGN KEY (action_type_id) REFERENCES people_skill_action_types(id) ON DELETE RESTRICT;

ALTER TABLE people_skill_actions
    ADD CONSTRAINT people_skill_actions_post_assessment_id_fkey FOREIGN KEY (post_assessment_id) REFERENCES people_skill_assessments(id) ON DELETE RESTRICT;

ALTER TABLE people_skill_actions
    ADD CONSTRAINT people_skill_actions_skill_id_fkey FOREIGN KEY (skill_id) REFERENCES people_skills(id) ON DELETE RESTRICT;

ALTER TABLE people_skill_actions
    ADD CONSTRAINT people_skill_actions_source_assessment_id_fkey FOREIGN KEY (source_assessment_id) REFERENCES people_skill_assessments(id) ON DELETE RESTRICT;

ALTER TABLE people_skill_assessment_decisions
    ADD CONSTRAINT people_skill_assessment_decisions_assessment_id_fkey FOREIGN KEY (assessment_id) REFERENCES people_skill_assessments(id) ON DELETE RESTRICT;

ALTER TABLE people_skill_assessments
    ADD CONSTRAINT people_skill_assessments_profile_id_fkey FOREIGN KEY (profile_id) REFERENCES people_skill_profiles(id) ON DELETE RESTRICT;

ALTER TABLE people_skill_assessments
    ADD CONSTRAINT people_skill_assessments_scale_id_fkey FOREIGN KEY (scale_id) REFERENCES people_skill_scales(id) ON DELETE RESTRICT;

ALTER TABLE people_skill_assessments
    ADD CONSTRAINT people_skill_assessments_skill_id_fkey FOREIGN KEY (skill_id) REFERENCES people_skills(id) ON DELETE RESTRICT;

ALTER TABLE people_skill_assessments
    ADD CONSTRAINT people_skill_assessments_supersedes_assessment_id_fkey FOREIGN KEY (supersedes_assessment_id) REFERENCES people_skill_assessments(id) ON DELETE RESTRICT;

ALTER TABLE people_skill_reassessment_requests
    ADD CONSTRAINT people_skill_reassessment_requests_assessment_id_fkey FOREIGN KEY (assessment_id) REFERENCES people_skill_assessments(id) ON DELETE RESTRICT;

ALTER TABLE people_skill_reassessment_requests
    ADD CONSTRAINT people_skill_reassessment_requests_skill_id_fkey FOREIGN KEY (skill_id) REFERENCES people_skills(id) ON DELETE RESTRICT;

ALTER TABLE people_skill_reminders
    ADD CONSTRAINT people_skill_reminders_action_id_fkey FOREIGN KEY (action_id) REFERENCES people_skill_actions(id) ON DELETE RESTRICT;

ALTER TABLE people_skill_reminders
    ADD CONSTRAINT people_skill_reminders_skill_id_fkey FOREIGN KEY (skill_id) REFERENCES people_skills(id) ON DELETE RESTRICT;

ALTER TABLE people_skill_scores
    ADD CONSTRAINT people_skill_scores_assessment_fkey FOREIGN KEY (assessment_id, company_id, employee_id, skill_id) REFERENCES people_skill_assessments(id, company_id, employee_id, skill_id) ON DELETE RESTRICT;

CREATE INDEX people_skill_action_events_action_id_inserted_at_index ON people_skill_action_events USING btree (action_id, inserted_at);

CREATE INDEX people_skill_action_events_tenant_id_company_id_index ON people_skill_action_events USING btree (tenant_id, company_id);

CREATE UNIQUE INDEX people_skill_action_types_company_code_unique ON people_skill_action_types USING btree (company_id, code);

CREATE INDEX people_skill_action_types_tenant_id_company_id_index ON people_skill_action_types USING btree (tenant_id, company_id);

CREATE INDEX people_skill_actions_company_id_employee_id_skill_id_index ON people_skill_actions USING btree (company_id, employee_id, skill_id);

CREATE UNIQUE INDEX people_skill_actions_one_per_assessment ON people_skill_actions USING btree (source_assessment_id) WHERE (source_assessment_id IS NOT NULL);

CREATE UNIQUE INDEX people_skill_actions_request_key_unique ON people_skill_actions USING btree (company_id, request_key);

CREATE INDEX people_skill_actions_tenant_id_company_id_status_due_on_index ON people_skill_actions USING btree (tenant_id, company_id, status, due_on);

CREATE UNIQUE INDEX people_skill_assessment_decisions_once ON people_skill_assessment_decisions USING btree (assessment_id, decision);

CREATE INDEX people_skill_assessment_decisions_tenant_id_company_id_index ON people_skill_assessment_decisions USING btree (tenant_id, company_id);

CREATE UNIQUE INDEX people_skill_assessments_one_successor ON people_skill_assessments USING btree (supersedes_assessment_id) WHERE (supersedes_assessment_id IS NOT NULL);

CREATE UNIQUE INDEX people_skill_assessments_request_key_unique ON people_skill_assessments USING btree (company_id, assessor_user_id, request_key);

CREATE INDEX people_skill_assessments_subject_status_index ON people_skill_assessments USING btree (company_id, employee_id, skill_id, status);

CREATE UNIQUE INDEX people_skill_assessments_subject_unique ON people_skill_assessments USING btree (id, company_id, employee_id, skill_id);

CREATE INDEX people_skill_assessments_tenant_id_company_id_status_index ON people_skill_assessments USING btree (tenant_id, company_id, status);

CREATE UNIQUE INDEX people_skill_reassessment_requests_one_open ON people_skill_reassessment_requests USING btree (company_id, employee_id, skill_id) WHERE ((status)::text = 'pending'::text);

CREATE INDEX people_skill_reassessment_requests_status_index ON people_skill_reassessment_requests USING btree (tenant_id, company_id, status);

CREATE UNIQUE INDEX people_skill_reminders_once ON people_skill_reminders USING btree (company_id, rule, employee_id, skill_id, action_id, period_key, recipient_user_id) NULLS NOT DISTINCT;

CREATE INDEX people_skill_reminders_recipient_index ON people_skill_reminders USING btree (tenant_id, company_id, recipient_user_id);

CREATE INDEX people_skill_scores_company_id_next_due_on_index ON people_skill_scores USING btree (company_id, next_due_on);

CREATE UNIQUE INDEX people_skill_scores_subject_unique ON people_skill_scores USING btree (company_id, employee_id, skill_id);

CREATE INDEX people_skill_scores_tenant_id_company_id_index ON people_skill_scores USING btree (tenant_id, company_id);

CREATE FUNCTION people_skill_assessments_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  prior record;
BEGIN
  IF TG_OP = 'DELETE' THEN
    RAISE EXCEPTION 'skill assessment % is a fact and is never deleted', OLD.id;
  END IF;
  IF TG_OP = 'INSERT' THEN
    IF NEW.status <> 'pending_review' THEN
      RAISE EXCEPTION 'a skill assessment enters as pending_review';
    END IF;
    IF NEW.supersedes_assessment_id IS NOT NULL THEN
      SELECT * INTO prior FROM people_skill_assessments
      WHERE id = NEW.supersedes_assessment_id;
      IF NOT FOUND OR prior.tenant_id <> NEW.tenant_id OR prior.company_id <> NEW.company_id
         OR prior.employee_id <> NEW.employee_id OR prior.skill_id <> NEW.skill_id
         OR prior.status NOT IN ('returned', 'finalized') THEN
        RAISE EXCEPTION 'an assessment supersedes a returned or finalized assessment of the same employee and skill';
      END IF;
    END IF;
    RETURN NEW;
  END IF;
  IF NOT ((OLD.status = 'pending_review' AND NEW.status IN ('verified', 'returned'))
          OR (OLD.status = 'verified' AND NEW.status = 'finalized')) THEN
    RAISE EXCEPTION 'skill assessment % is % and cannot become %',
      OLD.id, OLD.status, NEW.status;
  END IF;
  IF NOT ((to_jsonb(NEW) - ARRAY['status', 'reviewed_by_user_id', 'reviewed_at',
                'review_note', 'finalized_by_user_id', 'finalized_at', 'updated_at'])
             = (to_jsonb(OLD) - ARRAY['status', 'reviewed_by_user_id', 'reviewed_at',
                'review_note', 'finalized_by_user_id', 'finalized_at', 'updated_at'])) THEN
    RAISE EXCEPTION 'skill assessment % facts are immutable', OLD.id;
  END IF;
  IF OLD.status = 'verified' AND (
       NEW.reviewed_by_user_id IS DISTINCT FROM OLD.reviewed_by_user_id
       OR NEW.reviewed_at IS DISTINCT FROM OLD.reviewed_at
       OR NEW.review_note IS DISTINCT FROM OLD.review_note) THEN
    RAISE EXCEPTION 'skill assessment % review is immutable', OLD.id;
  END IF;
  RETURN NEW;
END;
$$;

CREATE TRIGGER people_skill_assessments_guard
BEFORE INSERT OR UPDATE OR DELETE ON people_skill_assessments
FOR EACH ROW EXECUTE FUNCTION people_skill_assessments_guard();

CREATE FUNCTION people_skill_append_only() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION '% rows are append-only', TG_TABLE_NAME;
END;
$$;

  CREATE TRIGGER people_skill_assessment_decisions_guard BEFORE UPDATE OR DELETE ON people_skill_assessment_decisions
  FOR EACH ROW EXECUTE FUNCTION people_skill_append_only()
  """)
end

execute("""
CREATE FUNCTION people_skill_scores_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  source record;
BEGIN
  IF TG_OP = 'DELETE' THEN
    RAISE EXCEPTION 'skill score % is never deleted', OLD.id;
  END IF;
  SELECT * INTO source FROM people_skill_assessments WHERE id = NEW.assessment_id;
  IF NOT FOUND OR source.status <> 'finalized'
     OR source.tenant_id <> NEW.tenant_id OR source.company_id <> NEW.company_id
     OR source.employee_id <> NEW.employee_id OR source.skill_id <> NEW.skill_id THEN
    RAISE EXCEPTION 'a skill score mirrors a finalized assessment';
  END IF;
  IF NEW.profile_id <> source.profile_id OR NEW.required_level <> source.required_level
     OR NEW.current_level <> source.assessed_level OR NEW.gap <> source.gap
     OR NEW.criticality <> source.criticality OR NEW.mandatory <> source.mandatory
     OR NEW.priority_score <> source.priority_score
     OR NEW.assessed_on <> source.assessed_on
     OR NEW.valid_until IS DISTINCT FROM source.valid_until
     OR NEW.next_due_on <> source.next_due_on THEN
    RAISE EXCEPTION 'a skill score copies its assessment';
  END IF;
  RETURN NEW;
END;
$$;

  CREATE TRIGGER people_skill_action_events_guard BEFORE UPDATE OR DELETE ON people_skill_action_events
  FOR EACH ROW EXECUTE FUNCTION people_skill_append_only()
  """)
end

execute("""
CREATE FUNCTION people_skill_scores_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  source record;
BEGIN
  IF TG_OP = 'DELETE' THEN
    RAISE EXCEPTION 'skill score % is never deleted', OLD.id;
  END IF;
  SELECT * INTO source FROM people_skill_assessments WHERE id = NEW.assessment_id;
  IF NOT FOUND OR source.status <> 'finalized'
     OR source.tenant_id <> NEW.tenant_id OR source.company_id <> NEW.company_id
     OR source.employee_id <> NEW.employee_id OR source.skill_id <> NEW.skill_id THEN
    RAISE EXCEPTION 'a skill score mirrors a finalized assessment';
  END IF;
  IF NEW.profile_id <> source.profile_id OR NEW.required_level <> source.required_level
     OR NEW.current_level <> source.assessed_level OR NEW.gap <> source.gap
     OR NEW.criticality <> source.criticality OR NEW.mandatory <> source.mandatory
     OR NEW.priority_score <> source.priority_score
     OR NEW.assessed_on <> source.assessed_on
     OR NEW.valid_until IS DISTINCT FROM source.valid_until
     OR NEW.next_due_on <> source.next_due_on THEN
    RAISE EXCEPTION 'a skill score copies its assessment';
  END IF;
  RETURN NEW;
END;
$$;

CREATE TRIGGER people_skill_scores_guard
BEFORE INSERT OR UPDATE OR DELETE ON people_skill_scores
FOR EACH ROW EXECUTE FUNCTION people_skill_scores_guard();

CREATE FUNCTION people_skill_reassessment_requests_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF TG_OP = 'DELETE' THEN
    RAISE EXCEPTION 'reassessment request % is never deleted', OLD.id;
  END IF;
  IF OLD.status <> 'pending' THEN
    RAISE EXCEPTION 'reassessment request % is % and immutable', OLD.id, OLD.status;
  END IF;
  IF NEW.employee_id <> OLD.employee_id OR NEW.skill_id <> OLD.skill_id
     OR NEW.company_id <> OLD.company_id OR NEW.reason <> OLD.reason
     OR NEW.requested_by_user_id <> OLD.requested_by_user_id OR NEW.due_on <> OLD.due_on THEN
    RAISE EXCEPTION 'reassessment request % facts are immutable', OLD.id;
  END IF;
  RETURN NEW;
END;
$$;

CREATE TRIGGER people_skill_reassessment_requests_guard
BEFORE UPDATE OR DELETE ON people_skill_reassessment_requests
FOR EACH ROW EXECUTE FUNCTION people_skill_reassessment_requests_guard();

CREATE FUNCTION people_skill_actions_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF TG_OP = 'DELETE' THEN
    RAISE EXCEPTION 'skill action % is never deleted', OLD.id;
  END IF;
  IF OLD.status IN ('completed', 'cancelled') THEN
    RAISE EXCEPTION 'skill action % is % and immutable', OLD.id, OLD.status;
  END IF;
  IF NEW.company_id <> OLD.company_id OR NEW.tenant_id <> OLD.tenant_id
     OR NEW.employee_id <> OLD.employee_id OR NEW.skill_id <> OLD.skill_id
     OR NEW.source_assessment_id IS DISTINCT FROM OLD.source_assessment_id
     OR NEW.starting_level <> OLD.starting_level OR NEW.target_level <> OLD.target_level
     OR NEW.gap_at_start <> OLD.gap_at_start OR NEW.criticality <> OLD.criticality
     OR NEW.mandatory <> OLD.mandatory OR NEW.priority_score <> OLD.priority_score
     OR NEW.priority_explanation <> OLD.priority_explanation
     OR NEW.created_by_user_id <> OLD.created_by_user_id
     OR NEW.request_key <> OLD.request_key OR NEW.employee_name <> OLD.employee_name THEN
    RAISE EXCEPTION 'skill action % source snapshot is immutable', OLD.id;
  END IF;
  RETURN NEW;
END;
$$;

CREATE TRIGGER people_skill_actions_guard
BEFORE UPDATE OR DELETE ON people_skill_actions
FOR EACH ROW EXECUTE FUNCTION people_skill_actions_guard();

CREATE FUNCTION people_skill_reminders_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF TG_OP = 'DELETE' THEN
    RAISE EXCEPTION 'skill reminder % is never deleted', OLD.id;
  END IF;
  IF OLD.state = 'sent' THEN
    RAISE EXCEPTION 'skill reminder % was sent and is immutable', OLD.id;
  END IF;
  IF NEW.rule <> OLD.rule OR NEW.recipient_user_id <> OLD.recipient_user_id
     OR NEW.period_key <> OLD.period_key OR NEW.company_id <> OLD.company_id
     OR NEW.skill_id <> OLD.skill_id OR NEW.employee_id IS DISTINCT FROM OLD.employee_id
     OR NEW.action_id IS DISTINCT FROM OLD.action_id OR NEW.due_on <> OLD.due_on THEN
    RAISE EXCEPTION 'skill reminder % identity is immutable', OLD.id;
  END IF;
  RETURN NEW;
END;
$$;

CREATE TRIGGER people_skill_reminders_guard
BEFORE UPDATE OR DELETE ON people_skill_reminders
FOR EACH ROW EXECUTE FUNCTION people_skill_reminders_guard();
