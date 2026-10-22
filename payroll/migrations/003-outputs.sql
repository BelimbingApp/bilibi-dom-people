-- Bilimbi People Payroll.Migrations.CreateOutputs; employment IDs belong to Workforce.
CREATE TABLE people_payroll_contributions (
    id bigserial PRIMARY KEY,
    tenant_id bigint NOT NULL,
    company_id bigint NOT NULL,
    created_by_actor_id bigint NOT NULL,
    run_id bigint NOT NULL,
    inserted_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    updated_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    FOREIGN KEY (tenant_id, company_id, run_id) REFERENCES people_payroll_setup_snapshots (tenant_id, company_id, id) ON DELETE RESTRICT,
    employee_id bigint NOT NULL,
    item_id bigint NOT NULL,
    source_key varchar(120) NOT NULL CHECK (length(btrim(source_key)) > 0),
    evidence varchar(500) NOT NULL CHECK (length(btrim(evidence)) > 0),
    on_date date NOT NULL,
    units numeric(20,6) NOT NULL CHECK (units > 0 AND units < 100000000000000),
    direction varchar(20) NOT NULL CHECK (direction IN ('earning','deduction','employer')),
    UNIQUE (tenant_id, company_id, run_id, id),
    UNIQUE (company_id, source_key),
    FOREIGN KEY (tenant_id, company_id, item_id) REFERENCES people_payroll_items (tenant_id, company_id, id) ON DELETE RESTRICT,
    FOREIGN KEY (employee_id, company_id) REFERENCES people_workforce_employments (id, company_id) ON DELETE RESTRICT
);
CREATE INDEX ON people_payroll_contributions (tenant_id, company_id, run_id);
CREATE TRIGGER people_payroll_contributions_immutable BEFORE UPDATE OR DELETE ON people_payroll_contributions FOR EACH ROW EXECUTE FUNCTION people_payroll_immutable();
CREATE TABLE people_payroll_calculations (
    id bigserial PRIMARY KEY,
    tenant_id bigint NOT NULL,
    company_id bigint NOT NULL,
    created_by_actor_id bigint NOT NULL,
    run_id bigint NOT NULL,
    inserted_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    updated_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    FOREIGN KEY (tenant_id, company_id, run_id) REFERENCES people_payroll_setup_snapshots (tenant_id, company_id, id) ON DELETE RESTRICT,
    snapshot jsonb NOT NULL,
    digest varchar(64) NOT NULL CHECK (digest ~ '^[0-9a-f]{64}$'),
    UNIQUE (run_id)
);
CREATE INDEX ON people_payroll_calculations (tenant_id, company_id, run_id);
CREATE TRIGGER people_payroll_calculations_immutable BEFORE UPDATE OR DELETE ON people_payroll_calculations FOR EACH ROW EXECUTE FUNCTION people_payroll_immutable();
CREATE TABLE people_payroll_decisions (
    id bigserial PRIMARY KEY,
    tenant_id bigint NOT NULL,
    company_id bigint NOT NULL,
    created_by_actor_id bigint NOT NULL,
    run_id bigint NOT NULL,
    inserted_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    updated_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    FOREIGN KEY (tenant_id, company_id, run_id) REFERENCES people_payroll_setup_snapshots (tenant_id, company_id, id) ON DELETE RESTRICT,
    outcome varchar(20) NOT NULL CHECK (outcome IN ('approved','rejected')),
    reason varchar(500) NOT NULL CHECK (length(btrim(reason)) > 0),
    UNIQUE (run_id)
);
CREATE INDEX ON people_payroll_decisions (tenant_id, company_id, run_id);
CREATE TRIGGER people_payroll_decisions_immutable BEFORE UPDATE OR DELETE ON people_payroll_decisions FOR EACH ROW EXECUTE FUNCTION people_payroll_immutable();
CREATE TABLE people_payroll_documents (
    id bigserial PRIMARY KEY,
    tenant_id bigint NOT NULL,
    company_id bigint NOT NULL,
    created_by_actor_id bigint NOT NULL,
    run_id bigint NOT NULL,
    inserted_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    updated_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    FOREIGN KEY (tenant_id, company_id, run_id) REFERENCES people_payroll_setup_snapshots (tenant_id, company_id, id) ON DELETE RESTRICT,
    employee_id bigint,
    artifact_id uuid NOT NULL UNIQUE,
    kind varchar(20) NOT NULL,
    CHECK ((kind = 'report' AND employee_id IS NULL) OR (kind = 'payslip' AND employee_id IS NOT NULL)),
    FOREIGN KEY (employee_id, company_id) REFERENCES people_workforce_employments (id, company_id) ON DELETE RESTRICT
);
CREATE INDEX ON people_payroll_documents (tenant_id, company_id, run_id);
CREATE TRIGGER people_payroll_documents_immutable BEFORE UPDATE OR DELETE ON people_payroll_documents FOR EACH ROW EXECUTE FUNCTION people_payroll_immutable();
CREATE TABLE people_payroll_calculation_entries (
    id bigserial PRIMARY KEY,
    tenant_id bigint NOT NULL,
    company_id bigint NOT NULL,
    created_by_actor_id bigint NOT NULL,
    run_id bigint NOT NULL,
    inserted_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    updated_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    FOREIGN KEY (tenant_id, company_id, run_id) REFERENCES people_payroll_setup_snapshots (tenant_id, company_id, id) ON DELETE RESTRICT,
    contribution_id bigint NOT NULL UNIQUE,
    employee_id bigint NOT NULL,
    direction varchar(20) NOT NULL CHECK (direction IN ('earning','deduction','employer')),
    amount numeric(40,12) NOT NULL CHECK (amount >= 0),
    FOREIGN KEY (tenant_id, company_id, run_id, contribution_id) REFERENCES people_payroll_contributions (tenant_id, company_id, run_id, id) ON DELETE RESTRICT,
    FOREIGN KEY (employee_id, company_id) REFERENCES people_workforce_employments (id, company_id) ON DELETE RESTRICT
);
CREATE INDEX ON people_payroll_calculation_entries (tenant_id, company_id, run_id);
CREATE TRIGGER people_payroll_calculation_entries_immutable BEFORE UPDATE OR DELETE ON people_payroll_calculation_entries FOR EACH ROW EXECUTE FUNCTION people_payroll_immutable();

-- The run lock serializes output stage transitions even for direct SQL callers.
CREATE FUNCTION people_payroll_output_stage() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE r people_payroll_setup_snapshots%ROWTYPE;
DECLARE c people_payroll_calculations%ROWTYPE;
BEGIN
    SELECT * INTO r FROM people_payroll_setup_snapshots
      WHERE tenant_id=NEW.tenant_id AND company_id=NEW.company_id AND id=NEW.run_id FOR UPDATE;
    IF r.id IS NULL THEN
        RAISE EXCEPTION 'payroll output scope mismatch' USING ERRCODE='23514';
    END IF;
    SELECT * INTO c FROM people_payroll_calculations
      WHERE tenant_id=NEW.tenant_id AND company_id=NEW.company_id AND run_id=NEW.run_id;
    IF TG_TABLE_NAME IN ('people_payroll_contributions','people_payroll_calculation_entries') AND c.id IS NOT NULL THEN
        RAISE EXCEPTION 'payroll calculation is frozen' USING ERRCODE='23514';
    END IF;
    IF TG_TABLE_NAME IN ('people_payroll_calculations','people_payroll_calculation_entries') AND r.locked_at IS NULL THEN
        RAISE EXCEPTION 'payroll setup must be locked' USING ERRCODE='23514';
    END IF;
    IF TG_TABLE_NAME='people_payroll_decisions' THEN
        IF c.id IS NULL OR NEW.created_by_actor_id IN (r.created_by_actor_id,r.locked_by_actor_id,c.created_by_actor_id) OR
          EXISTS (SELECT 1 FROM people_payroll_contributions WHERE tenant_id=NEW.tenant_id AND company_id=NEW.company_id AND run_id=NEW.run_id AND created_by_actor_id=NEW.created_by_actor_id) THEN
            RAISE EXCEPTION 'payroll independent decision required' USING ERRCODE='23514';
        END IF;
    END IF;
    IF TG_TABLE_NAME='people_payroll_documents' AND NOT EXISTS (
      SELECT 1 FROM people_payroll_decisions WHERE tenant_id=NEW.tenant_id AND company_id=NEW.company_id AND run_id=NEW.run_id AND outcome='approved') THEN
        RAISE EXCEPTION 'payroll approval required' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER people_payroll_contributions_stage BEFORE INSERT ON people_payroll_contributions FOR EACH ROW EXECUTE FUNCTION people_payroll_output_stage();
CREATE TRIGGER people_payroll_calculations_stage BEFORE INSERT ON people_payroll_calculations FOR EACH ROW EXECUTE FUNCTION people_payroll_output_stage();
CREATE TRIGGER people_payroll_decisions_stage BEFORE INSERT ON people_payroll_decisions FOR EACH ROW EXECUTE FUNCTION people_payroll_output_stage();
CREATE TRIGGER people_payroll_documents_stage BEFORE INSERT ON people_payroll_documents FOR EACH ROW EXECUTE FUNCTION people_payroll_output_stage();
CREATE TRIGGER people_payroll_calculation_entries_stage BEFORE INSERT ON people_payroll_calculation_entries FOR EACH ROW EXECUTE FUNCTION people_payroll_output_stage();
