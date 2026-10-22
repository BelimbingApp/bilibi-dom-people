-- Bilimbi Attendance CreateAllowanceRules; codes, units and currencies are company data.
CREATE TABLE people_attendance_allowance_policies (
 id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
 code varchar(40) NOT NULL, name varchar(120) NOT NULL, unit varchar(32) NOT NULL,
 value numeric(14,4) NOT NULL, currency varchar(3) NOT NULL,
 effective_from date NOT NULL, effective_until date, status varchar(16) NOT NULL DEFAULT 'active',
 inserted_at timestamptz NOT NULL DEFAULT now(), updated_at timestamptz NOT NULL DEFAULT now(),
 CONSTRAINT people_attendance_allowance_policies_company_code_from_unique UNIQUE(company_id,code,effective_from),
 CONSTRAINT people_attendance_allowance_policies_status_check CHECK(status IN ('active','retired')),
 CONSTRAINT people_attendance_allowance_policies_value_check CHECK(value>0),
 CONSTRAINT people_attendance_allowance_policies_period_check CHECK(effective_until IS NULL OR effective_until>=effective_from),
 FOREIGN KEY(tenant_id,company_id) REFERENCES companies(tenant_id,id)
);
CREATE INDEX people_attendance_allowance_policies_company_effective_index ON people_attendance_allowance_policies(tenant_id,company_id,status,effective_from);
