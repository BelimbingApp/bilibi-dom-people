-- Workforce 001: People's employment record, as Bilimbi's
-- `20261016090000_create_workforce_employments` leaves it, less the legacy
-- Core Employee mapping Bilibi has no rows to claim (`legacy_employee_id`,
-- `legacy_created_at`, `claimed_at`) and plus the three columns the port
-- plan adds (`supervisor_employment_id`, `department_id`, `version`).
--
-- One row is one employment. It references the Core Person (`person_id`) and
-- that person's affiliation with the employer (`affiliation_id`); the
-- employer company is the affiliation's company, which the scoped foreign key
-- into `person_affiliations` keeps true for good. Names, contacts and titles
-- stay in Core Person and Affiliation and are not copied here. The row's id
-- is the employee ID every other People table keys on.
--
-- The five statuses are the one fixed classification; employment types are a
-- company's reference data (the `employment_type` kind of `people/reference_data`).

CREATE TABLE people_workforce_employments (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  person_id bigint NOT NULL,
  affiliation_id bigint NOT NULL,
  employee_number varchar(255) NOT NULL,
  employment_type varchar(255) NOT NULL,
  status varchar(255) NOT NULL,
  employment_start date,
  employment_end date,
  supervisor_employment_id bigint,
  department_id bigint,
  metadata jsonb,
  version bigint NOT NULL DEFAULT 1,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  CONSTRAINT people_workforce_employments_id_company_unique UNIQUE (id, company_id),
  CONSTRAINT people_workforce_employments_affiliation_unique UNIQUE (affiliation_id),
  CONSTRAINT people_workforce_employments_company_number_unique UNIQUE (company_id, employee_number),
  CONSTRAINT people_workforce_employments_status
    CHECK (status IN ('pending', 'probation', 'active', 'inactive', 'terminated')),
  CONSTRAINT people_workforce_employments_period
    CHECK (employment_start IS NULL OR employment_end IS NULL OR employment_start <= employment_end),
  CONSTRAINT people_workforce_employments_not_own_supervisor
    CHECK (supervisor_employment_id IS NULL OR supervisor_employment_id <> id),
  CONSTRAINT people_workforce_employments_version CHECK (version >= 1),
  -- The affiliation belongs to this person and this company, in this tenant
  -- (Core Affiliation publishes this unique key for Employment references).
  CONSTRAINT people_workforce_employments_affiliation_fk
    FOREIGN KEY (tenant_id, affiliation_id, person_id, company_id)
    REFERENCES person_affiliations (tenant_id, id, person_id, organisation_id),
  CONSTRAINT people_workforce_employments_company_fk
    FOREIGN KEY (tenant_id, company_id) REFERENCES companies (tenant_id, id),
  -- A supervisor is an employment of the same company.
  CONSTRAINT people_workforce_employments_supervisor_fk
    FOREIGN KEY (supervisor_employment_id, company_id)
    REFERENCES people_workforce_employments (id, company_id),
  -- A department is one of Core Company's departments of this tenant.
  CONSTRAINT people_workforce_employments_department_fk
    FOREIGN KEY (tenant_id, department_id) REFERENCES company_departments (tenant_id, id)
);

CREATE INDEX people_workforce_employments_tenant_id_company_id_index
  ON people_workforce_employments (tenant_id, company_id);
CREATE INDEX people_workforce_employments_company_id_status_index
  ON people_workforce_employments (company_id, status);
CREATE INDEX people_workforce_employments_supervisor_index
  ON people_workforce_employments (supervisor_employment_id);
CREATE INDEX people_workforce_employments_person_index
  ON people_workforce_employments (tenant_id, person_id);
