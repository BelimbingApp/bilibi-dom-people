-- Claims 001: the claim catalog, effective-dated claim policies, claim
-- requests and their status history, as Bilimbi's
-- `20260930150101_create_people_claims` leaves them once
-- `20260930170101_add_claim_approval` has altered them (the claim type's
-- eligibility, the request's decision and reimbursement columns, the
-- event's reason, the five statuses and the final receipt index). The
-- hand-off batch reference on a request lands in 002 with the batch table.
--
-- Every row carries its tenant and company. An employee is named by the
-- employment id (`people_workforce_employments.id`); Bilibi has no Core
-- Employee. Money columns are numeric(14, 2): twelve integer digits and two
-- minor-unit digits, in the currency the row names. Nothing here assumes a
-- currency: the company's claim currencies are the setting
-- `people.claims.currencies`, and each policy and request stores its own.

-- Categories (Bilimbi `people_claim_catalog_groups`).
CREATE TABLE people_claim_catalog_groups (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  code varchar(60) NOT NULL,
  name varchar(120) NOT NULL,
  active boolean NOT NULL DEFAULT true,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  CONSTRAINT people_claim_catalog_groups_company_code_unique UNIQUE (company_id, code),
  -- Published for the claim type's scoped reference.
  CONSTRAINT people_claim_catalog_groups_id_company_unique UNIQUE (id, company_id),
  CONSTRAINT people_claim_catalog_groups_company_fk
    FOREIGN KEY (tenant_id, company_id) REFERENCES companies (tenant_id, id)
);
CREATE INDEX people_claim_catalog_groups_tenant_id_company_id_index
  ON people_claim_catalog_groups (tenant_id, company_id);

-- Claim types: one category, a receipt rule and who may claim.
CREATE TABLE people_claim_catalog_types (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  category_id bigint NOT NULL,
  code varchar(60) NOT NULL,
  name varchar(120) NOT NULL,
  receipt_requirement varchar(20) NOT NULL,
  eligibility varchar(20) NOT NULL DEFAULT 'all_employees',
  active boolean NOT NULL DEFAULT true,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  CONSTRAINT people_claim_catalog_types_company_code_unique UNIQUE (company_id, code),
  CONSTRAINT people_claim_catalog_types_id_company_unique UNIQUE (id, company_id),
  CONSTRAINT people_claim_catalog_types_receipt_requirement_check
    CHECK (receipt_requirement IN ('always', 'above_threshold', 'never')),
  CONSTRAINT people_claim_catalog_types_eligibility_check
    CHECK (eligibility IN ('all_employees', 'assigned_only')),
  -- The category is one of this company's.
  CONSTRAINT people_claim_catalog_types_category_id_fkey
    FOREIGN KEY (category_id, company_id)
    REFERENCES people_claim_catalog_groups (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT people_claim_catalog_types_company_fk
    FOREIGN KEY (tenant_id, company_id) REFERENCES companies (tenant_id, id)
);
CREATE INDEX people_claim_catalog_types_scope_idx
  ON people_claim_catalog_types (tenant_id, company_id, category_id);

-- Effective-dated policies: one claim type, a period, a currency from the
-- company's list and optional limits. Periods of one claim type never
-- overlap (the facade decides it under the claim type's row lock).
CREATE TABLE people_claim_policy_versions (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  claim_type_id bigint NOT NULL,
  effective_from date NOT NULL,
  effective_to date,
  currency varchar(3) NOT NULL,
  per_claim_limit numeric(14, 2),
  monthly_limit numeric(14, 2),
  yearly_limit numeric(14, 2),
  receipt_threshold numeric(14, 2),
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  CONSTRAINT people_claim_policy_versions_id_company_unique UNIQUE (id, company_id),
  CONSTRAINT people_claim_policy_versions_period_check
    CHECK (effective_to IS NULL OR effective_to >= effective_from),
  CONSTRAINT people_claim_policy_versions_currency_check CHECK (currency ~ '^[A-Z]{3}$'),
  CONSTRAINT people_claim_policy_versions_amounts_check
    CHECK ((per_claim_limit IS NULL OR per_claim_limit > 0) AND
           (monthly_limit IS NULL OR monthly_limit > 0) AND
           (yearly_limit IS NULL OR yearly_limit > 0) AND
           (receipt_threshold IS NULL OR receipt_threshold >= 0)),
  CONSTRAINT people_claim_policy_versions_claim_type_id_fkey
    FOREIGN KEY (claim_type_id, company_id)
    REFERENCES people_claim_catalog_types (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT people_claim_policy_versions_company_fk
    FOREIGN KEY (tenant_id, company_id) REFERENCES companies (tenant_id, id)
);
CREATE INDEX people_claim_policy_versions_scope_idx
  ON people_claim_policy_versions (tenant_id, company_id, claim_type_id, effective_from);

-- Claim requests (Bilimbi `people_claim_submissions`): one employee's claim
-- under the policy in effect on the expense date, its status, and the
-- decision and reimbursement facts the status requires.
CREATE TABLE people_claim_submissions (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  employee_id bigint NOT NULL,
  claim_type_id bigint NOT NULL,
  claim_policy_id bigint NOT NULL,
  incurred_on date NOT NULL,
  amount numeric(14, 2) NOT NULL,
  currency varchar(3) NOT NULL,
  description varchar(500),
  receipt_number varchar(100),
  status varchar(20) NOT NULL,
  duplicate_confirmed boolean NOT NULL DEFAULT false,
  submitted_by_actor_id bigint NOT NULL,
  submitted_at timestamp(0) NOT NULL,
  withdrawn_by_actor_id bigint,
  withdrawn_at timestamp(0),
  approved_amount numeric(14, 2),
  decided_by_actor_id bigint,
  decided_at timestamp(0),
  decision_reason varchar(500),
  reimbursed_by_actor_id bigint,
  reimbursed_at timestamp(0),
  payment_reference varchar(100),
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  CONSTRAINT people_claim_submissions_id_company_unique UNIQUE (id, company_id),
  CONSTRAINT people_claim_submissions_status_check
    CHECK (status IN ('submitted', 'approved', 'rejected', 'reimbursed', 'withdrawn')),
  CONSTRAINT people_claim_submissions_amount_check CHECK (amount > 0),
  CONSTRAINT people_claim_submissions_currency_check CHECK (currency ~ '^[A-Z]{3}$'),
  CONSTRAINT people_claim_submissions_decision_check
    CHECK ((status IN ('submitted', 'withdrawn') AND approved_amount IS NULL
            AND decided_by_actor_id IS NULL AND decided_at IS NULL) OR
           (status IN ('approved', 'reimbursed') AND approved_amount > 0
            AND approved_amount <= amount AND decided_by_actor_id IS NOT NULL
            AND decided_at IS NOT NULL) OR
           (status = 'rejected' AND approved_amount IS NULL
            AND decided_by_actor_id IS NOT NULL AND decided_at IS NOT NULL
            AND decision_reason IS NOT NULL)),
  CONSTRAINT people_claim_submissions_reimbursement_check
    CHECK ((status = 'reimbursed' AND reimbursed_by_actor_id IS NOT NULL
            AND reimbursed_at IS NOT NULL) OR
           (status <> 'reimbursed' AND reimbursed_by_actor_id IS NULL
            AND reimbursed_at IS NULL AND payment_reference IS NULL)),
  -- The employee is an employment of this company (Workforce publishes the key).
  CONSTRAINT people_claim_submissions_employee_fk
    FOREIGN KEY (employee_id, company_id)
    REFERENCES people_workforce_employments (id, company_id),
  CONSTRAINT people_claim_submissions_claim_type_id_fkey
    FOREIGN KEY (claim_type_id, company_id)
    REFERENCES people_claim_catalog_types (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT people_claim_submissions_claim_policy_id_fkey
    FOREIGN KEY (claim_policy_id, company_id)
    REFERENCES people_claim_policy_versions (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT people_claim_submissions_company_fk
    FOREIGN KEY (tenant_id, company_id) REFERENCES companies (tenant_id, id)
);
CREATE INDEX people_claim_submissions_scope_idx
  ON people_claim_submissions (tenant_id, company_id, employee_id, status);
CREATE INDEX people_claim_submissions_usage_idx
  ON people_claim_submissions (company_id, employee_id, claim_type_id, incurred_on);
CREATE INDEX people_claim_submissions_queue_idx
  ON people_claim_submissions (tenant_id, company_id, status, currency);
-- A receipt reference is on at most one live claim of an employee in the
-- company; a withdrawn or rejected claim releases it.
CREATE UNIQUE INDEX people_claim_submissions_receipt_unique
  ON people_claim_submissions (company_id, employee_id, receipt_number)
  WHERE receipt_number IS NOT NULL AND status NOT IN ('withdrawn', 'rejected');

-- Status history: one row per transition, with the decision reason when
-- there is one.
CREATE TABLE people_claim_request_events (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  request_id bigint NOT NULL,
  from_status varchar(20),
  to_status varchar(20) NOT NULL,
  actor_id bigint NOT NULL,
  reason varchar(500),
  occurred_at timestamp(0) NOT NULL,
  CONSTRAINT people_claim_request_events_request_id_fkey
    FOREIGN KEY (request_id, company_id)
    REFERENCES people_claim_submissions (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT people_claim_request_events_company_fk
    FOREIGN KEY (tenant_id, company_id) REFERENCES companies (tenant_id, id)
);
CREATE INDEX people_claim_request_events_scope_idx
  ON people_claim_request_events (tenant_id, company_id, request_id);
