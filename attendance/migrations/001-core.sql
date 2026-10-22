-- Attendance 001: the end state of Bilimbi's attendance core
-- (`20260930100301_create_people_attendance_core` plus the shift definitions,
-- clocking locations and clock-fact coordinates of
-- `20260930180101_create_people_attendance_rosters`). Rosters, adjustments
-- and allowance policies follow in their own files (port plan row PA2).
--
-- `employee_id` is the employment id (`people_workforce_employments.id`), the
-- one durable employee reference every People table keys on; it is kept as
-- a plain column, as Bilimbi keeps it, because attendance facts outlive an
-- employment's working status. Instants are UTC `timestamptz`, the
-- Platform's convention; `on_date` is the local date in the company's
-- attendance time zone at the time of the event.

CREATE TABLE people_attendance_daily_summaries (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  employee_id bigint NOT NULL,
  on_date date NOT NULL,
  status varchar(32) NOT NULL,
  first_in_at timestamptz(0),
  last_out_at timestamptz(0),
  worked_minutes integer NOT NULL DEFAULT 0,
  inserted_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT people_attendance_daily_summaries_company_employee_date_unique
    UNIQUE (company_id, employee_id, on_date),
  CONSTRAINT people_attendance_daily_summaries_status_check
    CHECK (status IN ('in_progress', 'exception_pending', 'ready_for_review')),
  CONSTRAINT people_attendance_daily_summaries_minutes_check
    CHECK (worked_minutes >= 0),
  CONSTRAINT people_attendance_daily_summaries_company_fk
    FOREIGN KEY (tenant_id, company_id) REFERENCES companies (tenant_id, id)
);

CREATE INDEX people_attendance_daily_summaries_scope_date_index
  ON people_attendance_daily_summaries (tenant_id, company_id, on_date);

CREATE TABLE people_attendance_shift_definitions (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  code varchar(40) NOT NULL,
  name varchar(120) NOT NULL,
  -- Minutes past local midnight in the company's attendance time zone; an
  -- end at or before the start crosses midnight.
  start_minute integer NOT NULL,
  end_minute integer NOT NULL,
  break_minutes integer NOT NULL DEFAULT 0,
  status varchar(16) NOT NULL,
  inserted_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT people_attendance_shift_definitions_company_code_unique UNIQUE (company_id, code),
  CONSTRAINT people_attendance_shift_definitions_status_check
    CHECK (status IN ('active', 'retired')),
  CONSTRAINT people_attendance_shift_definitions_span_check
    CHECK (start_minute BETWEEN 0 AND 1439 AND end_minute BETWEEN 0 AND 1439
      AND start_minute <> end_minute AND break_minutes >= 0),
  CONSTRAINT people_attendance_shift_definitions_company_fk
    FOREIGN KEY (tenant_id, company_id) REFERENCES companies (tenant_id, id)
);

CREATE INDEX people_attendance_shift_definitions_status_index
  ON people_attendance_shift_definitions (tenant_id, company_id, status);

CREATE TABLE people_attendance_clocking_locations (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  code varchar(40) NOT NULL,
  name varchar(120) NOT NULL,
  latitude numeric(9, 6) NOT NULL,
  longitude numeric(9, 6) NOT NULL,
  radius_meters integer NOT NULL,
  status varchar(16) NOT NULL,
  inserted_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT people_attendance_clocking_locations_company_code_unique UNIQUE (company_id, code),
  CONSTRAINT people_attendance_clocking_locations_point_check
    CHECK (latitude BETWEEN -90 AND 90 AND longitude BETWEEN -180 AND 180 AND radius_meters > 0),
  CONSTRAINT people_attendance_clocking_locations_status_check
    CHECK (status IN ('active', 'retired')),
  CONSTRAINT people_attendance_clocking_locations_company_fk
    FOREIGN KEY (tenant_id, company_id) REFERENCES companies (tenant_id, id)
);

CREATE INDEX people_attendance_clocking_locations_status_index
  ON people_attendance_clocking_locations (tenant_id, company_id, status);

CREATE TABLE people_attendance_clock_facts (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  employee_id bigint NOT NULL,
  day_id bigint NOT NULL REFERENCES people_attendance_daily_summaries (id) ON DELETE RESTRICT,
  event_key varchar(160) NOT NULL,
  event_type varchar(16) NOT NULL,
  source varchar(32) NOT NULL,
  occurred_at timestamptz(0) NOT NULL,
  timezone varchar(100) NOT NULL,
  actor_user_id bigint,
  latitude numeric(9, 6),
  longitude numeric(9, 6),
  clocking_location_id bigint REFERENCES people_attendance_clocking_locations (id) ON DELETE RESTRICT,
  inserted_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  -- One fact per company, source and key: a replay is answered, never doubled.
  CONSTRAINT people_attendance_events_source_key_unique UNIQUE (company_id, source, event_key),
  CONSTRAINT people_attendance_clock_facts_type_check
    CHECK (event_type IN ('in', 'out', 'break_in', 'break_out')),
  CONSTRAINT people_attendance_clock_facts_point_check
    CHECK ((latitude IS NULL) = (longitude IS NULL)),
  CONSTRAINT people_attendance_clock_facts_company_fk
    FOREIGN KEY (tenant_id, company_id) REFERENCES companies (tenant_id, id)
);

CREATE INDEX people_attendance_clock_facts_employee_time_index
  ON people_attendance_clock_facts (tenant_id, company_id, employee_id, occurred_at);
CREATE INDEX people_attendance_clock_facts_day_index
  ON people_attendance_clock_facts (day_id);
