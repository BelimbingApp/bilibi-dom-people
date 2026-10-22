-- Leave 003: the employees and types the latest carry-forward run of a leave
-- year left open, with the reason, as Bilimbi's
-- `20260930200101_add_people_leave_carry_forward_skips`. Each run replaces
-- its year's rows (slice PL2 writes them; this slice creates the table).

CREATE TABLE people_leave_carry_forward_skips (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  from_year integer NOT NULL,
  employee_id bigint NOT NULL,
  employee_label varchar(300) NOT NULL,
  leave_type_id bigint NOT NULL REFERENCES people_leave_catalog_types (id) ON DELETE RESTRICT,
  reason varchar(24) NOT NULL,
  blocking_year integer NOT NULL,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  CONSTRAINT people_leave_carry_forward_skips_unique
    UNIQUE (company_id, from_year, employee_id, leave_type_id),
  CONSTRAINT people_leave_carry_forward_skips_reason
    CHECK (reason IN ('pending', 'previous_year_open')),
  CONSTRAINT people_leave_carry_forward_skips_company_fk
    FOREIGN KEY (tenant_id, company_id) REFERENCES companies (tenant_id, id),
  CONSTRAINT people_leave_carry_forward_skips_employee_fk
    FOREIGN KEY (employee_id, company_id) REFERENCES people_workforce_employments (id, company_id)
);
