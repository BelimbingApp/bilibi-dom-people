-- Leave 002: requests, their counted dates and their history, as Bilimbi's
-- `20260930190101_add_people_leave_applications` leaves them (the two
-- catalog columns it added are in 001). Requests land in slice PL2; this
-- slice creates the tables so `balances` can report the quantity pending
-- requests reserve, and so the slot indexes are in place before any request.

CREATE TABLE people_leave_applications (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  employee_id bigint NOT NULL,
  leave_type_id bigint NOT NULL REFERENCES people_leave_catalog_types (id) ON DELETE RESTRICT,
  leave_year integer NOT NULL,
  starts_on date NOT NULL,
  ends_on date NOT NULL,
  day_part varchar(8) NOT NULL,
  quantity numeric(10, 2) NOT NULL,
  unit varchar(8) NOT NULL,
  status varchar(16) NOT NULL,
  reason varchar(500),
  request_key varchar(160) NOT NULL,
  requested_by_user_id bigint NOT NULL,
  decided_by_user_id bigint,
  decided_at timestamptz,
  decision_note varchar(500),
  cancelled_by_user_id bigint,
  cancelled_at timestamptz,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  CONSTRAINT people_leave_applications_employee_key_unique UNIQUE (company_id, employee_id, request_key),
  CONSTRAINT people_leave_applications_date_range CHECK (ends_on >= starts_on),
  CONSTRAINT people_leave_applications_quantity_positive CHECK (quantity > 0),
  CONSTRAINT people_leave_applications_status
    CHECK (status IN ('pending', 'approved', 'rejected', 'cancelled')),
  CONSTRAINT people_leave_applications_day_part CHECK (day_part IN ('full', 'am', 'pm', 'hours')),
  CONSTRAINT people_leave_applications_company_fk
    FOREIGN KEY (tenant_id, company_id) REFERENCES companies (tenant_id, id),
  CONSTRAINT people_leave_applications_employee_fk
    FOREIGN KEY (employee_id, company_id) REFERENCES people_workforce_employments (id, company_id)
);

CREATE INDEX people_leave_applications_tenant_id_company_id_status_index
  ON people_leave_applications (tenant_id, company_id, status);
CREATE INDEX people_leave_applications_employee_year_index
  ON people_leave_applications (tenant_id, company_id, employee_id, leave_year);

-- One row per counted date. A live request holds its half-day slots, so two
-- live requests can never cover the same half of one employee's date.
CREATE TABLE people_leave_application_dates (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  employee_id bigint NOT NULL,
  request_id bigint NOT NULL REFERENCES people_leave_applications (id) ON DELETE RESTRICT,
  on_date date NOT NULL,
  am boolean NOT NULL,
  pm boolean NOT NULL,
  quantity numeric(10, 2) NOT NULL,
  active boolean NOT NULL,
  CONSTRAINT people_leave_application_dates_request_date_unique UNIQUE (request_id, on_date),
  CONSTRAINT people_leave_application_dates_slot CHECK (am OR pm)
);

CREATE UNIQUE INDEX people_leave_application_dates_am_unique
  ON people_leave_application_dates (company_id, employee_id, on_date) WHERE active AND am;
CREATE UNIQUE INDEX people_leave_application_dates_pm_unique
  ON people_leave_application_dates (company_id, employee_id, on_date) WHERE active AND pm;

CREATE TABLE people_leave_request_events (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  request_id bigint NOT NULL REFERENCES people_leave_applications (id) ON DELETE RESTRICT,
  from_status varchar(16),
  to_status varchar(16) NOT NULL,
  actor_user_id bigint NOT NULL,
  note varchar(500),
  occurred_at timestamptz NOT NULL
);

CREATE INDEX people_leave_request_events_request_id_index
  ON people_leave_request_events (request_id);

-- Request history is append-only, like the ledger.
CREATE FUNCTION people_leave_request_events_append_only() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'people_leave_request_events is append-only';
END;
$$;

CREATE TRIGGER people_leave_request_events_append_only
BEFORE UPDATE OR DELETE ON people_leave_request_events
FOR EACH ROW EXECUTE FUNCTION people_leave_request_events_append_only();
