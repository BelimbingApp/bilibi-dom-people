-- Bilimbi People EmployeeWorkspace.Migrations.CreateEmployeeWorkspace.
CREATE TABLE people_employee_workspace_profiles (
 id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
 employee_id bigint NOT NULL REFERENCES people_workforce_employments(id),
 work_location varchar(200), work_arrangement varchar(120), notes text,
 inserted_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
 updated_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
 UNIQUE(company_id, employee_id)
);
CREATE INDEX ON people_employee_workspace_profiles(tenant_id, company_id);
CREATE TABLE people_employee_accesses (
 id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
 employee_id bigint NOT NULL REFERENCES people_workforce_employments(id),
 portal_enabled boolean NOT NULL DEFAULT false, reason text,
 inserted_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
 updated_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
 UNIQUE(company_id, employee_id)
);
CREATE INDEX ON people_employee_accesses(tenant_id, company_id);
CREATE TABLE people_employee_change_requests (
 id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
 employee_id bigint NOT NULL REFERENCES people_workforce_employments(id),
 field varchar(80) NOT NULL, proposed_value varchar(500) NOT NULL, reason text,
 status varchar(20) NOT NULL DEFAULT 'pending', requested_by_actor_id bigint NOT NULL,
 reviewed_by_actor_id bigint, reviewed_at timestamp,
 inserted_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
 updated_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
 CONSTRAINT people_employee_change_requests_status_check CHECK (status IN ('pending','approved','rejected'))
);
CREATE INDEX people_employee_change_requests_scope_idx ON people_employee_change_requests(tenant_id, company_id, employee_id);
CREATE TABLE people_employee_saved_views (
 id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
 actor_id bigint NOT NULL, name varchar(120) NOT NULL, search varchar(200), status varchar(40),
 inserted_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
 updated_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
 UNIQUE(company_id, actor_id, name)
);
CREATE INDEX ON people_employee_saved_views(tenant_id, company_id, actor_id);
