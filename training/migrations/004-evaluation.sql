-- Mirrors Bilimbi Training CreateEvaluation: permanent policies, obligations,
-- answers, reminder worklist and disclosure-controlled frozen summaries.

CREATE TABLE people_training_evaluation_policies (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  actor_user_id bigint NOT NULL,
  impersonator_id bigint,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  version integer NOT NULL CHECK (version > 0),
  effective_from date NOT NULL,
  effective_to date NOT NULL CHECK (effective_to >= effective_from),
  criteria jsonb NOT NULL,
  effectiveness_criteria jsonb NOT NULL,
  evaluation_days integer NOT NULL CHECK (evaluation_days BETWEEN 0 AND 3650),
  checkpoints jsonb NOT NULL,
  reminder_days integer NOT NULL CHECK (reminder_days BETWEEN 0 AND 3650),
  reason text NOT NULL CHECK (length(btrim(reason)) > 0 AND octet_length(reason) <= 4000),
  UNIQUE (company_id, version),
  UNIQUE (id, tenant_id, company_id),
  FOREIGN KEY (tenant_id, company_id) REFERENCES companies (tenant_id, id)
);
CREATE INDEX people_training_evaluation_policies_scope ON people_training_evaluation_policies (tenant_id, company_id);

CREATE TABLE people_training_evaluation_reviews (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  actor_user_id bigint NOT NULL,
  impersonator_id bigint,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  policy_id bigint NOT NULL,
  fact_id bigint NOT NULL,
  kind text NOT NULL,
  checkpoint_days integer NOT NULL,
  due_on date NOT NULL,
  CHECK ((kind = 'evaluation' AND checkpoint_days = 0) OR (kind = 'effectiveness' AND checkpoint_days > 0)),
  UNIQUE (fact_id, kind, checkpoint_days),
  UNIQUE (id, tenant_id, company_id),
  FOREIGN KEY (policy_id, tenant_id, company_id) REFERENCES people_training_evaluation_policies (id, tenant_id, company_id),
  FOREIGN KEY (fact_id, tenant_id, company_id) REFERENCES people_training_attendance_facts (id, tenant_id, company_id)
);
CREATE INDEX people_training_evaluation_reviews_scope ON people_training_evaluation_reviews (tenant_id, company_id);

CREATE TABLE people_training_evaluation_answers (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  actor_user_id bigint NOT NULL,
  impersonator_id bigint,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  review_id bigint NOT NULL UNIQUE,
  values jsonb NOT NULL,
  reason text NOT NULL CHECK (length(btrim(reason)) > 0 AND octet_length(reason) <= 4000),
  FOREIGN KEY (review_id, tenant_id, company_id) REFERENCES people_training_evaluation_reviews (id, tenant_id, company_id)
);
CREATE INDEX people_training_evaluation_answers_scope ON people_training_evaluation_answers (tenant_id, company_id);

CREATE TABLE people_training_review_reminders (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  actor_user_id bigint NOT NULL,
  impersonator_id bigint,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  review_id bigint NOT NULL UNIQUE,
  recipient_employee_id bigint NOT NULL,
  available_on date NOT NULL,
  FOREIGN KEY (review_id, tenant_id, company_id) REFERENCES people_training_evaluation_reviews (id, tenant_id, company_id)
);
CREATE INDEX people_training_review_reminders_scope ON people_training_review_reminders (tenant_id, company_id);

CREATE TABLE people_training_effectiveness_summaries (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  actor_user_id bigint NOT NULL,
  impersonator_id bigint,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  period_start date NOT NULL,
  period_end date NOT NULL CHECK (period_end >= period_start),
  minimum_cohort integer NOT NULL CHECK (minimum_cohort BETWEEN 2 AND 1000),
  status text NOT NULL CHECK (status IN ('current', 'suppressed')),
  groups jsonb NOT NULL CHECK (jsonb_typeof(groups->'items') = 'array'),
  UNIQUE (tenant_id, company_id, period_start),
  FOREIGN KEY (tenant_id, company_id) REFERENCES companies (tenant_id, id)
);
CREATE INDEX people_training_effectiveness_summaries_scope ON people_training_effectiveness_summaries (tenant_id, company_id);

CREATE FUNCTION people_training_criteria_valid(criteria jsonb) RETURNS boolean LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE item jsonb; codes text[] := ARRAY[]::text[];
BEGIN
  IF jsonb_typeof(criteria->'items') IS DISTINCT FROM 'array' THEN RETURN false; END IF;
  IF jsonb_array_length(criteria->'items') NOT BETWEEN 1 AND 50 THEN RETURN false; END IF;
  FOR item IN SELECT value FROM jsonb_array_elements(criteria->'items') LOOP
    IF jsonb_typeof(item) IS DISTINCT FROM 'object' THEN RETURN false; END IF;
    IF (SELECT count(*) FROM jsonb_object_keys(item)) <> 4 OR NOT item ?& ARRAY['code','label','minimum','maximum'] THEN RETURN false; END IF;
    IF jsonb_typeof(item->'code') IS DISTINCT FROM 'string' OR (item->>'code') !~ '^[a-z][a-z0-9_]{0,79}$'
      OR jsonb_typeof(item->'label') IS DISTINCT FROM 'string' OR length(btrim(item->>'label')) NOT BETWEEN 1 AND 4000
      OR jsonb_typeof(item->'minimum') IS DISTINCT FROM 'number' OR (item->>'minimum') !~ '^-?[0-9]{1,5}$'
      OR jsonb_typeof(item->'maximum') IS DISTINCT FROM 'number' OR (item->>'maximum') !~ '^-?[0-9]{1,5}$' THEN RETURN false; END IF;
    IF (item->>'minimum')::integer < -10000 OR (item->>'maximum')::integer > 10000
      OR (item->>'minimum')::integer > (item->>'maximum')::integer OR (item->>'code') = ANY(codes) THEN RETURN false; END IF;
    codes := array_append(codes, item->>'code');
  END LOOP;
  RETURN true;
END $$;

CREATE FUNCTION people_training_evaluation_review_guard() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE fact people_training_attendance_facts; policy people_training_evaluation_policies;
  session people_training_session_runs; completed date; offset_days integer;
BEGIN
  SELECT * INTO fact FROM people_training_attendance_facts WHERE id = NEW.fact_id AND tenant_id = NEW.tenant_id AND company_id = NEW.company_id;
  SELECT * INTO session FROM people_training_session_runs WHERE id = fact.session_id AND tenant_id = NEW.tenant_id AND company_id = NEW.company_id FOR UPDATE;
  SELECT * INTO policy FROM people_training_evaluation_policies WHERE id = NEW.policy_id AND tenant_id = NEW.tenant_id AND company_id = NEW.company_id;
  completed := (session.ends_at AT TIME ZONE 'UTC' AT TIME ZONE session.time_zone)::date;
  offset_days := CASE WHEN NEW.kind = 'evaluation' THEN policy.evaluation_days ELSE NEW.checkpoint_days END;
  IF fact.status IS DISTINCT FROM 'confirmed' OR completed NOT BETWEEN policy.effective_from AND policy.effective_to
    OR NEW.due_on <> completed + offset_days OR (NEW.kind = 'effectiveness' AND NOT (policy.checkpoints->'days') @> to_jsonb(ARRAY[NEW.checkpoint_days]))
    OR EXISTS (SELECT 1 FROM people_training_evaluation_reviews r JOIN people_training_attendance_facts f ON f.id = r.fact_id AND f.tenant_id = r.tenant_id AND f.company_id = r.company_id
      WHERE r.tenant_id = NEW.tenant_id AND r.company_id = NEW.company_id AND f.session_id = fact.session_id AND f.employee_id = fact.employee_id AND r.kind = NEW.kind AND r.checkpoint_days = NEW.checkpoint_days) THEN
    RAISE EXCEPTION 'Invalid or duplicate review obligation' USING ERRCODE = '23514';
  END IF;
  RETURN NEW;
END $$;

CREATE FUNCTION people_training_evaluation_answer_guard() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE criteria jsonb; criterion jsonb; score jsonb;
BEGIN
  SELECT CASE WHEN r.kind = 'evaluation' THEN p.criteria ELSE p.effectiveness_criteria END INTO criteria
    FROM people_training_evaluation_reviews r JOIN people_training_evaluation_policies p ON p.id = r.policy_id AND p.tenant_id = r.tenant_id AND p.company_id = r.company_id
    WHERE r.id = NEW.review_id AND r.tenant_id = NEW.tenant_id AND r.company_id = NEW.company_id;
  IF jsonb_typeof(NEW.values) IS DISTINCT FROM 'object' THEN
    RAISE EXCEPTION 'Answers must be criterion values' USING ERRCODE = '23514';
  END IF;
  IF (SELECT count(*) FROM jsonb_object_keys(NEW.values)) <> jsonb_array_length(criteria->'items') THEN
    RAISE EXCEPTION 'Answers must include exactly the criteria version' USING ERRCODE = '23514';
  END IF;
  FOR criterion IN SELECT value FROM jsonb_array_elements(criteria->'items') LOOP
    IF NOT NEW.values ? (criterion->>'code') THEN
      RAISE EXCEPTION 'Missing criterion' USING ERRCODE = '23514';
    END IF;
    score := NEW.values->(criterion->>'code');
    IF score <> 'null'::jsonb THEN
      IF jsonb_typeof(score) IS DISTINCT FROM 'number' OR score::text !~ '^-?[0-9]{1,5}$' THEN
        RAISE EXCEPTION 'Invalid criterion score' USING ERRCODE = '23514';
      END IF;
      IF score::text::integer NOT BETWEEN (criterion->>'minimum')::integer AND (criterion->>'maximum')::integer THEN
        RAISE EXCEPTION 'Score outside criterion bounds' USING ERRCODE = '23514';
      END IF;
    END IF;
  END LOOP;
  RETURN NEW;
END $$;

CREATE FUNCTION people_training_evaluation_policy_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF NOT people_training_criteria_valid(NEW.criteria) OR NOT people_training_criteria_valid(NEW.effectiveness_criteria)
    OR jsonb_typeof(NEW.checkpoints->'days') IS DISTINCT FROM 'array' THEN
    RAISE EXCEPTION 'Invalid criteria or checkpoints' USING ERRCODE = '23514';
  END IF;
  IF jsonb_array_length(NEW.checkpoints->'days') NOT BETWEEN 1 AND 24 OR
    EXISTS (SELECT 1 FROM jsonb_array_elements(NEW.checkpoints->'days') d WHERE jsonb_typeof(d) <> 'number' OR d::text !~ '^[0-9]{1,4}$') THEN
    RAISE EXCEPTION 'Invalid checkpoints' USING ERRCODE = '23514';
  END IF;
  IF EXISTS (SELECT 1 FROM jsonb_array_elements_text(NEW.checkpoints->'days') d WHERE d::integer NOT BETWEEN 1 AND 3650)
    OR (SELECT count(DISTINCT d) FROM jsonb_array_elements_text(NEW.checkpoints->'days') d) <> jsonb_array_length(NEW.checkpoints->'days') THEN
    RAISE EXCEPTION 'Checkpoints must be distinct positive day offsets' USING ERRCODE = '23514';
  END IF;
  PERFORM pg_advisory_xact_lock(hashtextextended('people.training.evaluation:' || NEW.tenant_id || ':' || NEW.company_id, 0));
  IF EXISTS (SELECT 1 FROM people_training_evaluation_policies p WHERE p.company_id = NEW.company_id
      AND p.tenant_id = NEW.tenant_id AND p.effective_from <= NEW.effective_to AND p.effective_to >= NEW.effective_from) THEN
    RAISE EXCEPTION 'Evaluation policy periods overlap' USING ERRCODE = '23514';
  END IF;
  IF NEW.version <> (SELECT COALESCE(max(version), 0) + 1 FROM people_training_evaluation_policies
      WHERE tenant_id = NEW.tenant_id AND company_id = NEW.company_id) THEN
    RAISE EXCEPTION 'Policy must append next version' USING ERRCODE = '23514';
  END IF;
  RETURN NEW;
END $$;

CREATE FUNCTION people_training_evaluation_immutable() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'Evaluation history is immutable' USING ERRCODE = '23514';
END $$;

CREATE TRIGGER people_training_evaluation_review_guard BEFORE INSERT ON people_training_evaluation_reviews FOR EACH ROW EXECUTE FUNCTION people_training_evaluation_review_guard();

CREATE TRIGGER people_training_evaluation_answer_guard BEFORE INSERT ON people_training_evaluation_answers FOR EACH ROW EXECUTE FUNCTION people_training_evaluation_answer_guard();

CREATE TRIGGER people_training_evaluation_policy_guard BEFORE INSERT ON people_training_evaluation_policies FOR EACH ROW EXECUTE FUNCTION people_training_evaluation_policy_guard();

CREATE TRIGGER people_training_evaluation_policies_immutable BEFORE UPDATE OR DELETE ON people_training_evaluation_policies FOR EACH ROW EXECUTE FUNCTION people_training_evaluation_immutable();
CREATE TRIGGER people_training_evaluation_policies_truncate_guard BEFORE TRUNCATE ON people_training_evaluation_policies FOR EACH STATEMENT EXECUTE FUNCTION people_training_evaluation_immutable();

CREATE TRIGGER people_training_evaluation_reviews_immutable BEFORE UPDATE OR DELETE ON people_training_evaluation_reviews FOR EACH ROW EXECUTE FUNCTION people_training_evaluation_immutable();
CREATE TRIGGER people_training_evaluation_reviews_truncate_guard BEFORE TRUNCATE ON people_training_evaluation_reviews FOR EACH STATEMENT EXECUTE FUNCTION people_training_evaluation_immutable();

CREATE TRIGGER people_training_evaluation_answers_immutable BEFORE UPDATE OR DELETE ON people_training_evaluation_answers FOR EACH ROW EXECUTE FUNCTION people_training_evaluation_immutable();
CREATE TRIGGER people_training_evaluation_answers_truncate_guard BEFORE TRUNCATE ON people_training_evaluation_answers FOR EACH STATEMENT EXECUTE FUNCTION people_training_evaluation_immutable();

CREATE TRIGGER people_training_review_reminders_immutable BEFORE UPDATE OR DELETE ON people_training_review_reminders FOR EACH ROW EXECUTE FUNCTION people_training_evaluation_immutable();
CREATE TRIGGER people_training_review_reminders_truncate_guard BEFORE TRUNCATE ON people_training_review_reminders FOR EACH STATEMENT EXECUTE FUNCTION people_training_evaluation_immutable();

CREATE TRIGGER people_training_effectiveness_summaries_immutable BEFORE UPDATE OR DELETE ON people_training_effectiveness_summaries FOR EACH ROW EXECUTE FUNCTION people_training_evaluation_immutable();
CREATE TRIGGER people_training_effectiveness_summaries_truncate_guard BEFORE TRUNCATE ON people_training_effectiveness_summaries FOR EACH STATEMENT EXECUTE FUNCTION people_training_evaluation_immutable();
