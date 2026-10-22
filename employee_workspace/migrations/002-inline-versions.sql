-- Bilimbi EmployeeWorkspace work facts, with Bilibi inline commit fences.
ALTER TABLE people_employee_workspace_profiles ADD COLUMN version bigint NOT NULL DEFAULT 1 CHECK (version > 0);
ALTER TABLE people_employee_accesses ADD COLUMN version bigint NOT NULL DEFAULT 1 CHECK (version > 0);
