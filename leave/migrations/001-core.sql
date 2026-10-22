-- Leave 001: types, entitlement policy versions and the append-only ledger,
-- as Bilimbi's `20260930160101_create_people_leave_core` leaves them with the
-- two columns `20260930190101_add_people_leave_applications` added to these
-- tables (`balance_required`, `carry_forward_cap`). Every employee column
-- keys on the employment id (`people_workforce_employments.id`), the one
-- durable employee reference in People; Bilimbi keyed on Core Employee.

-- A company's leave types: a unique lowercase code, a name, a unit (`day` or
-- `hour`), a paid flag, whether requests need an available balance, and
-- `active` or `archived`. Archived types take no new policy versions, grants,
-- requests or entries, but keep their history and balances.
CREATE TABLE people_leave_catalog_types (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  code varchar(40) NOT NULL,
  name varchar(120) NOT NULL,
  unit varchar(8) NOT NULL,
  paid boolean NOT NULL,
  balance_required boolean NOT NULL DEFAULT true,
  status varchar(16) NOT NULL,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  CONSTRAINT people_leave_catalog_types_company_code_unique UNIQUE (company_id, code),
  CONSTRAINT people_leave_catalog_types_unit CHECK (unit IN ('day', 'hour')),
  CONSTRAINT people_leave_catalog_types_status CHECK (status IN ('active', 'archived')),
  CONSTRAINT people_leave_catalog_types_company_fk
    FOREIGN KEY (tenant_id, company_id) REFERENCES companies (tenant_id, id)
);

CREATE INDEX people_leave_catalog_types_tenant_id_company_id_index
  ON people_leave_catalog_types (tenant_id, company_id);

-- Effective-dated entitlement versions of a type. A new version starts after
-- the latest one, which it closes on the previous day; versions are never
-- edited. `carry_forward_cap` is the most of a closing balance carried into
-- the next leave year; NULL means the type does not carry forward.
CREATE TABLE people_leave_policies (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  leave_type_id bigint NOT NULL REFERENCES people_leave_catalog_types (id) ON DELETE RESTRICT,
  version integer NOT NULL,
  effective_from date NOT NULL,
  effective_to date,
  entitlement numeric(8, 2) NOT NULL,
  carry_forward_cap numeric(8, 2),
  actor_user_id bigint,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  CONSTRAINT people_leave_policies_type_version_unique UNIQUE (leave_type_id, version),
  CONSTRAINT people_leave_policies_type_from_unique UNIQUE (leave_type_id, effective_from),
  CONSTRAINT people_leave_policies_effective_range
    CHECK (effective_to IS NULL OR effective_to >= effective_from),
  CONSTRAINT people_leave_policies_entitlement_non_negative CHECK (entitlement >= 0),
  CONSTRAINT people_leave_policies_carry_forward_cap_non_negative
    CHECK (carry_forward_cap IS NULL OR carry_forward_cap >= 0),
  CONSTRAINT people_leave_policies_company_fk
    FOREIGN KEY (tenant_id, company_id) REFERENCES companies (tenant_id, id)
);

CREATE INDEX people_leave_policies_tenant_id_company_id_index
  ON people_leave_policies (tenant_id, company_id);

-- The balance ledger. One row is one movement in the type's unit for one
-- employee and leave year: `entitlement` (a grant), `opening`, `adjustment`,
-- `taken`, `cancelled`, `carried_forward` and `expired`. Idempotent by
-- company, source and key.
CREATE TABLE people_leave_ledger_entries (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  employee_id bigint NOT NULL,
  leave_type_id bigint NOT NULL REFERENCES people_leave_catalog_types (id) ON DELETE RESTRICT,
  leave_year integer NOT NULL,
  entry_type varchar(24) NOT NULL,
  quantity numeric(10, 2) NOT NULL,
  unit varchar(8) NOT NULL,
  policy_id bigint REFERENCES people_leave_policies (id) ON DELETE RESTRICT,
  policy_version integer,
  occurred_on date NOT NULL,
  source varchar(32) NOT NULL,
  entry_key varchar(160) NOT NULL,
  actor_user_id bigint,
  note varchar(500),
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  CONSTRAINT people_leave_ledger_entries_source_key_unique UNIQUE (company_id, source, entry_key),
  CONSTRAINT people_leave_ledger_entries_entry_type CHECK (entry_type IN
    ('entitlement', 'opening', 'adjustment', 'taken', 'cancelled', 'carried_forward', 'expired')),
  CONSTRAINT people_leave_ledger_entries_company_fk
    FOREIGN KEY (tenant_id, company_id) REFERENCES companies (tenant_id, id),
  -- The employee is an employment of this company.
  CONSTRAINT people_leave_ledger_entries_employee_fk
    FOREIGN KEY (employee_id, company_id) REFERENCES people_workforce_employments (id, company_id)
);

CREATE INDEX people_leave_ledger_entries_employee_year_index
  ON people_leave_ledger_entries (tenant_id, company_id, employee_id, leave_year);

-- The ledger is append-only: a correction is a new entry, never an edit.
CREATE FUNCTION people_leave_ledger_entries_append_only() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'people_leave_ledger_entries is append-only';
END;
$$;

CREATE TRIGGER people_leave_ledger_entries_append_only
BEFORE UPDATE OR DELETE ON people_leave_ledger_entries
FOR EACH ROW EXECUTE FUNCTION people_leave_ledger_entries_append_only();
