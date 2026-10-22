-- Claims 002: assignments and hand-off batches, as Bilimbi's
-- `20260930170101_add_claim_approval` adds them (its alterations of the 001
-- tables are already in 001), and the hand-off batch reference on a request.
--
-- An assignment (Bilimbi `people_claim_employee_enrolments`) is a named,
-- effective-dated group: the claim types it opens and the employees it
-- covers. An `assigned_only` claim type is open to an employee only while
-- an assignment in effect covers both. A hand-off batch is the immutable
-- record of which approved claims of one currency were handed off, by whom
-- and when.

CREATE TABLE people_claim_employee_enrolments (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  code varchar(60) NOT NULL,
  name varchar(120) NOT NULL,
  effective_from date NOT NULL,
  effective_to date,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  CONSTRAINT people_claim_employee_enrolments_company_code_unique UNIQUE (company_id, code),
  CONSTRAINT people_claim_employee_enrolments_id_company_unique UNIQUE (id, company_id),
  CONSTRAINT people_claim_employee_enrolments_period_check
    CHECK (effective_to IS NULL OR effective_to >= effective_from),
  CONSTRAINT people_claim_employee_enrolments_company_fk
    FOREIGN KEY (tenant_id, company_id) REFERENCES companies (tenant_id, id)
);
CREATE INDEX people_claim_employee_enrolments_tenant_id_company_id_index
  ON people_claim_employee_enrolments (tenant_id, company_id);

-- The claim types an assignment opens.
CREATE TABLE people_claim_assignment_types (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  assignment_id bigint NOT NULL,
  claim_type_id bigint NOT NULL,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  CONSTRAINT people_claim_assignment_types_unique UNIQUE (assignment_id, claim_type_id),
  CONSTRAINT people_claim_assignment_types_assignment_id_fkey
    FOREIGN KEY (assignment_id, company_id)
    REFERENCES people_claim_employee_enrolments (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT people_claim_assignment_types_claim_type_id_fkey
    FOREIGN KEY (claim_type_id, company_id)
    REFERENCES people_claim_catalog_types (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT people_claim_assignment_types_company_fk
    FOREIGN KEY (tenant_id, company_id) REFERENCES companies (tenant_id, id)
);
CREATE INDEX people_claim_assignment_types_scope_idx
  ON people_claim_assignment_types (tenant_id, company_id, claim_type_id);

-- The employees an assignment covers: employments of the same company.
CREATE TABLE people_claim_assignment_employees (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  assignment_id bigint NOT NULL,
  employee_id bigint NOT NULL,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  CONSTRAINT people_claim_assignment_employees_unique UNIQUE (assignment_id, employee_id),
  CONSTRAINT people_claim_assignment_employees_assignment_id_fkey
    FOREIGN KEY (assignment_id, company_id)
    REFERENCES people_claim_employee_enrolments (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT people_claim_assignment_employees_employee_fk
    FOREIGN KEY (employee_id, company_id)
    REFERENCES people_workforce_employments (id, company_id),
  CONSTRAINT people_claim_assignment_employees_company_fk
    FOREIGN KEY (tenant_id, company_id) REFERENCES companies (tenant_id, id)
);
CREATE INDEX people_claim_assignment_employees_scope_idx
  ON people_claim_assignment_employees (tenant_id, company_id, employee_id);

CREATE TABLE people_claim_handoff_batches (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  currency varchar(3) NOT NULL,
  request_count integer NOT NULL,
  total_amount numeric(14, 2) NOT NULL,
  created_by_actor_id bigint NOT NULL,
  created_at timestamp(0) NOT NULL,
  CONSTRAINT people_claim_handoff_batches_id_company_unique UNIQUE (id, company_id),
  CONSTRAINT people_claim_handoff_batches_currency_check CHECK (currency ~ '^[A-Z]{3}$'),
  CONSTRAINT people_claim_handoff_batches_totals_check
    CHECK (request_count > 0 AND total_amount > 0),
  CONSTRAINT people_claim_handoff_batches_company_fk
    FOREIGN KEY (tenant_id, company_id) REFERENCES companies (tenant_id, id)
);
CREATE INDEX people_claim_handoff_batches_scope_idx
  ON people_claim_handoff_batches (tenant_id, company_id, created_at);

-- A request joins at most one batch, and only once approved.
ALTER TABLE people_claim_submissions
  ADD COLUMN handoff_batch_id bigint,
  ADD CONSTRAINT people_claim_submissions_handoff_batch_id_fkey
    FOREIGN KEY (handoff_batch_id, company_id)
    REFERENCES people_claim_handoff_batches (id, company_id) ON DELETE RESTRICT,
  ADD CONSTRAINT people_claim_submissions_handoff_check
    CHECK (handoff_batch_id IS NULL OR status IN ('approved', 'reimbursed'));
CREATE INDEX people_claim_submissions_handoff_batch_id_index
  ON people_claim_submissions (handoff_batch_id);
