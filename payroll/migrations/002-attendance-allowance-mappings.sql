-- Bilimbi People Payroll.Migrations.CreateAttendanceRulePayItems.
CREATE TABLE people_payroll_allowance_item_mappings (
    id bigserial PRIMARY KEY,
    tenant_id bigint NOT NULL,
    company_id bigint NOT NULL,
    created_by_actor_id bigint NOT NULL,
    attendance_rule_code varchar(40) NOT NULL,
    item_id bigint NOT NULL,
    effective_from date NOT NULL,
    effective_to date,
    inserted_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    updated_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    FOREIGN KEY (tenant_id, company_id, item_id) REFERENCES people_payroll_items (tenant_id, company_id, id) ON DELETE RESTRICT,
    CONSTRAINT people_payroll_allowance_item_mappings_dates CHECK (effective_to IS NULL OR effective_to >= effective_from)
);
CREATE INDEX people_payroll_allowance_item_mappings_company_index ON people_payroll_allowance_item_mappings (tenant_id, company_id);
CREATE TRIGGER people_payroll_allowance_item_mappings_immutable BEFORE UPDATE OR DELETE ON people_payroll_allowance_item_mappings FOR EACH ROW EXECUTE FUNCTION people_payroll_immutable();
