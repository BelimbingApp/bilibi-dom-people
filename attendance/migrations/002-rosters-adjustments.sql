-- Bilimbi Attendance CreateRosters: definitions and locations are already in 001.
CREATE TABLE people_attendance_roster_entries (
 id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
 employee_id bigint NOT NULL, on_date date NOT NULL, kind varchar(16) NOT NULL,
 shift_template_id bigint REFERENCES people_attendance_shift_definitions(id) ON DELETE RESTRICT,
 published_kind varchar(16),
 published_shift_template_id bigint REFERENCES people_attendance_shift_definitions(id) ON DELETE RESTRICT,
 published_at timestamptz(0), published_by_user_id bigint,
 revision integer NOT NULL DEFAULT 1, updated_by_user_id bigint,
 inserted_at timestamptz NOT NULL DEFAULT now(), updated_at timestamptz NOT NULL DEFAULT now(),
 CONSTRAINT people_attendance_roster_entries_company_employee_date_unique UNIQUE(company_id,employee_id,on_date),
 CONSTRAINT people_attendance_roster_entries_kind_check CHECK(kind IN ('shift','rest','none') AND (kind='shift')=(shift_template_id IS NOT NULL)),
 CONSTRAINT people_attendance_roster_entries_published_check CHECK(
 (published_kind IS NULL AND published_shift_template_id IS NULL AND published_at IS NULL) OR
 (published_kind IS NOT NULL AND published_kind IN ('shift','rest') AND published_at IS NOT NULL AND (published_kind='shift')=(published_shift_template_id IS NOT NULL))),
 FOREIGN KEY(tenant_id,company_id) REFERENCES companies(tenant_id,id)
);
CREATE INDEX people_attendance_roster_entries_date_index ON people_attendance_roster_entries(tenant_id,company_id,on_date);
CREATE TABLE people_attendance_corrections (
 id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
 employee_id bigint NOT NULL, request_key varchar(160) NOT NULL, event_type varchar(16) NOT NULL,
 proposed_at timestamptz(0) NOT NULL, on_date date NOT NULL, timezone varchar(100) NOT NULL,
 reason varchar(500) NOT NULL, status varchar(16) NOT NULL DEFAULT 'pending', requested_by_user_id bigint NOT NULL,
 decided_by_user_id bigint, decided_at timestamptz(0), decision_note varchar(500),
 applied_clock_event_id bigint REFERENCES people_attendance_clock_facts(id) ON DELETE RESTRICT,
 inserted_at timestamptz NOT NULL DEFAULT now(), updated_at timestamptz NOT NULL DEFAULT now(),
 CONSTRAINT people_attendance_corrections_company_key_unique UNIQUE(company_id,request_key),
 CONSTRAINT people_attendance_corrections_state_check CHECK(event_type IN ('in','out') AND status IN ('pending','approved','rejected','cancelled')
 AND (status='pending')=(decided_at IS NULL) AND (status='pending')=(decided_by_user_id IS NULL)
 AND (status='approved')=(applied_clock_event_id IS NOT NULL)),
 FOREIGN KEY(tenant_id,company_id) REFERENCES companies(tenant_id,id)
);
CREATE INDEX people_attendance_corrections_status_index ON people_attendance_corrections(tenant_id,company_id,status);
CREATE INDEX people_attendance_corrections_employee_date_index ON people_attendance_corrections(company_id,employee_id,on_date);
