-- Training 001: the course catalog, delivery events and session runs, as
-- Bilimbi's `20261001060101_create_people_training_catalog` leaves them.
--
-- A course is a company's own: its code is unique in the company and stable;
-- operators supply every name and description, nothing is seeded. An event
-- delivers one active course to a cohort of at most `capacity` learners; a
-- session is one run of that event, entered as local wall clocks in an
-- explicit IANA zone and stored as UTC instants (`timestamp(0)`, UTC, bound
-- and read as text) beside the delivery zone. Composite foreign keys keep an
-- event in its course's tenant and company and a session in its event's.
-- Authorship (`actor_user_id`, `impersonator_id`) comes from the sealed scope.
--
-- Two triggers guard what a check cannot: a session's capacity never exceeds
-- its event's (the event row is locked while a session is written), an event
-- is never shrunk below a session it already has, and a stored zone is one
-- PostgreSQL knows.

CREATE TABLE people_training_courses (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  code varchar(80) NOT NULL,
  name varchar(160) NOT NULL,
  description text,
  active boolean NOT NULL,
  actor_user_id bigint NOT NULL,
  impersonator_id bigint,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  CONSTRAINT people_training_courses_company_fk
    FOREIGN KEY (tenant_id, company_id) REFERENCES companies (tenant_id, id)
);

CREATE UNIQUE INDEX people_training_courses_company_id_code_index
  ON people_training_courses (company_id, code);
CREATE UNIQUE INDEX people_training_courses_id_tenant_id_company_id_index
  ON people_training_courses (id, tenant_id, company_id);

CREATE TABLE people_training_events (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  course_id bigint NOT NULL,
  name varchar(160) NOT NULL,
  capacity integer NOT NULL,
  actor_user_id bigint NOT NULL,
  impersonator_id bigint,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  CONSTRAINT people_training_events_capacity CHECK (capacity > 0),
  -- The course is one of this tenant and company.
  CONSTRAINT people_training_events_course_scope
    FOREIGN KEY (course_id, tenant_id, company_id)
    REFERENCES people_training_courses (id, tenant_id, company_id)
);

CREATE UNIQUE INDEX people_training_events_id_tenant_id_company_id_index
  ON people_training_events (id, tenant_id, company_id);
CREATE INDEX people_training_events_tenant_id_company_id_index
  ON people_training_events (tenant_id, company_id);

CREATE TABLE people_training_session_runs (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  event_id bigint NOT NULL,
  name varchar(160) NOT NULL,
  capacity integer NOT NULL,
  time_zone varchar(100) NOT NULL,
  starts_at timestamp(0) NOT NULL,
  ends_at timestamp(0) NOT NULL,
  actor_user_id bigint NOT NULL,
  impersonator_id bigint,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  CONSTRAINT people_training_session_runs_capacity CHECK (capacity > 0),
  CONSTRAINT people_training_session_runs_times CHECK (ends_at > starts_at),
  -- The event is one of this tenant and company.
  CONSTRAINT people_training_session_runs_event_scope
    FOREIGN KEY (event_id, tenant_id, company_id)
    REFERENCES people_training_events (id, tenant_id, company_id)
);

CREATE INDEX people_training_session_runs_scope_starts_index
  ON people_training_session_runs (tenant_id, company_id, starts_at);

-- A session never exceeds its event's capacity (the event row is locked for
-- the rest of the writing transaction) and names a zone PostgreSQL knows.
CREATE FUNCTION people_training_session_guard() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE event_capacity integer;
BEGIN
  SELECT capacity INTO event_capacity FROM people_training_events
    WHERE id = NEW.event_id AND tenant_id = NEW.tenant_id AND company_id = NEW.company_id
    FOR UPDATE;
  IF event_capacity IS NULL OR NEW.capacity > event_capacity THEN
    RAISE EXCEPTION 'Session capacity exceeds its scoped event' USING ERRCODE = '23514';
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_timezone_names WHERE name = NEW.time_zone) THEN
    RAISE EXCEPTION 'Unknown session time zone' USING ERRCODE = '23514';
  END IF;
  RETURN NEW;
END $$;

CREATE TRIGGER people_training_session_guard
  BEFORE INSERT OR UPDATE ON people_training_session_runs
  FOR EACH ROW EXECUTE FUNCTION people_training_session_guard();

-- An event is never shrunk below a session it already has.
CREATE FUNCTION people_training_event_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF EXISTS (SELECT 1 FROM people_training_session_runs WHERE event_id = NEW.id AND capacity > NEW.capacity) THEN
    RAISE EXCEPTION 'Event capacity is below a session capacity' USING ERRCODE = '23514';
  END IF;
  RETURN NEW;
END $$;

CREATE TRIGGER people_training_event_guard
  BEFORE UPDATE ON people_training_events
  FOR EACH ROW EXECUTE FUNCTION people_training_event_guard();
