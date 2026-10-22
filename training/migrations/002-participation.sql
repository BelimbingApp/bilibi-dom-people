-- Bilimbi Training 20261001060201: permanent attendance revisions and
-- evidence provenance. Base Artifacts alone owns bytes and their lifecycle.
CREATE UNIQUE INDEX people_training_session_runs_id_tenant_id_company_id_index
  ON people_training_session_runs (id, tenant_id, company_id);

CREATE TABLE people_training_attendance_facts (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  session_id bigint NOT NULL,
  employee_id bigint NOT NULL,
  revision integer NOT NULL,
  status varchar(20) NOT NULL,
  reason text NOT NULL,
  import_key varchar(160) NOT NULL,
  actor_user_id bigint NOT NULL,
  impersonator_id bigint,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  CONSTRAINT people_training_attendance_facts_values CHECK (
    revision > 0 AND employee_id > 0 AND status IN ('confirmed', 'absent')
    AND length(btrim(reason)) BETWEEN 1 AND 2000
    AND length(btrim(import_key)) BETWEEN 1 AND 160),
  CONSTRAINT people_training_attendance_facts_session_scope
    FOREIGN KEY (session_id, tenant_id, company_id)
    REFERENCES people_training_session_runs (id, tenant_id, company_id)
);
CREATE UNIQUE INDEX people_training_attendance_facts_import
  ON people_training_attendance_facts (tenant_id, company_id, import_key);
CREATE UNIQUE INDEX people_training_attendance_facts_revision
  ON people_training_attendance_facts (session_id, employee_id, revision);
CREATE UNIQUE INDEX people_training_facts_scope
  ON people_training_attendance_facts (id, tenant_id, company_id);

CREATE TABLE people_training_evidence (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  fact_id bigint NOT NULL,
  artifact_id uuid NOT NULL UNIQUE,
  actor_user_id bigint NOT NULL,
  impersonator_id bigint,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  CONSTRAINT people_training_evidence_fact_scope
    FOREIGN KEY (fact_id, tenant_id, company_id)
    REFERENCES people_training_attendance_facts (id, tenant_id, company_id)
);
CREATE INDEX people_training_evidence_scope
  ON people_training_evidence (tenant_id, company_id, fact_id);

CREATE FUNCTION people_training_fact_guard() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE maximum integer; previous_revision integer; occupied integer; prior_status text;
BEGIN
  IF TG_OP <> 'INSERT' THEN
    RAISE EXCEPTION 'Training facts are append only' USING ERRCODE = '23514';
  END IF;
  SELECT capacity INTO maximum FROM people_training_session_runs
    WHERE id = NEW.session_id AND tenant_id = NEW.tenant_id AND company_id = NEW.company_id FOR UPDATE;
  SELECT revision, status INTO previous_revision, prior_status FROM people_training_attendance_facts
    WHERE tenant_id = NEW.tenant_id AND company_id = NEW.company_id
      AND session_id = NEW.session_id AND employee_id = NEW.employee_id ORDER BY revision DESC LIMIT 1;
  IF NEW.revision <> COALESCE(previous_revision, 0) + 1 THEN
    RAISE EXCEPTION 'Training correction must append next revision' USING ERRCODE = '23514';
  END IF;
  IF NEW.status = 'confirmed' AND prior_status IS DISTINCT FROM 'confirmed' THEN
    SELECT count(*) INTO occupied FROM (
      SELECT DISTINCT ON (employee_id) status FROM people_training_attendance_facts
      WHERE tenant_id = NEW.tenant_id AND company_id = NEW.company_id
        AND session_id = NEW.session_id ORDER BY employee_id, revision DESC
    ) latest WHERE status = 'confirmed';
    IF occupied >= maximum THEN
      RAISE EXCEPTION 'Training session capacity exceeded' USING ERRCODE = '23514';
    END IF;
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER people_training_fact_guard
  BEFORE INSERT OR UPDATE OR DELETE ON people_training_attendance_facts
  FOR EACH ROW EXECUTE FUNCTION people_training_fact_guard();

CREATE FUNCTION people_training_evidence_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'Training evidence provenance is append only' USING ERRCODE = '23514';
END $$;
CREATE TRIGGER people_training_evidence_guard
  BEFORE UPDATE OR DELETE ON people_training_evidence
  FOR EACH ROW EXECUTE FUNCTION people_training_evidence_guard();
-- TRUNCATE is not a row operation; protect permanent history there too.
CREATE TRIGGER people_training_facts_truncate_guard
  BEFORE TRUNCATE ON people_training_attendance_facts
  FOR EACH STATEMENT EXECUTE FUNCTION people_training_evidence_guard();
CREATE TRIGGER people_training_evidence_truncate_guard
  BEFORE TRUNCATE ON people_training_evidence
  FOR EACH STATEMENT EXECUTE FUNCTION people_training_evidence_guard();

CREATE FUNCTION people_training_participation_capacity_guard() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE occupied integer;
BEGIN
  SELECT count(*) INTO occupied FROM (
    SELECT DISTINCT ON (employee_id) status FROM people_training_attendance_facts
    WHERE tenant_id = NEW.tenant_id AND company_id = NEW.company_id
      AND session_id = NEW.id ORDER BY employee_id, revision DESC
  ) latest WHERE status = 'confirmed';
  IF NEW.capacity < occupied THEN
    RAISE EXCEPTION 'Session capacity is below confirmed attendance' USING ERRCODE = '23514';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER people_training_participation_capacity_guard
  BEFORE UPDATE ON people_training_session_runs
  FOR EACH ROW EXECUTE FUNCTION people_training_participation_capacity_guard();
