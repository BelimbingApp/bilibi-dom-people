-- Bilimbi People Payroll.Migrations.CreateFoundation (fresh schema).
CREATE TABLE people_payroll_classifications (
    id bigserial PRIMARY KEY,
    tenant_id bigint NOT NULL,
    company_id bigint NOT NULL,
    created_by_actor_id bigint NOT NULL,
    code varchar(60) NOT NULL,
    name varchar(120) NOT NULL,
    effective_from date NOT NULL,
    effective_to date,
    inserted_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    updated_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    UNIQUE (tenant_id, company_id, id),
    CONSTRAINT people_payroll_classifications_dates CHECK (effective_to IS NULL OR effective_to >= effective_from)
);
CREATE INDEX ON people_payroll_classifications (tenant_id, company_id);
CREATE TABLE people_payroll_items (
    id bigserial PRIMARY KEY,
    tenant_id bigint NOT NULL,
    company_id bigint NOT NULL,
    created_by_actor_id bigint NOT NULL,
    code varchar(60) NOT NULL,
    name varchar(120) NOT NULL,
    classification_id bigint NOT NULL,
    currency varchar(3) NOT NULL,
    amount numeric(20,6) NOT NULL,
    effective_from date NOT NULL,
    effective_to date,
    inserted_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    updated_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    UNIQUE (tenant_id, company_id, id),
    FOREIGN KEY (tenant_id, company_id, classification_id) REFERENCES people_payroll_classifications (tenant_id, company_id, id) ON DELETE RESTRICT,
    CONSTRAINT people_payroll_items_dates CHECK (effective_to IS NULL OR effective_to >= effective_from),
    CONSTRAINT people_payroll_items_money CHECK (amount >= 0 AND currency ~ '^[A-Z]{3}$')
);
CREATE INDEX ON people_payroll_items (tenant_id, company_id);
CREATE TABLE people_payroll_pay_windows (
    id bigserial PRIMARY KEY,
    tenant_id bigint NOT NULL,
    company_id bigint NOT NULL,
    created_by_actor_id bigint NOT NULL,
    code varchar(60) NOT NULL,
    starts_on date NOT NULL,
    ends_on date NOT NULL,
    pay_on date NOT NULL,
    inserted_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    updated_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    UNIQUE (tenant_id, company_id, id),
    CONSTRAINT people_payroll_pay_windows_dates CHECK (ends_on >= starts_on AND pay_on >= ends_on)
);
CREATE INDEX ON people_payroll_pay_windows (tenant_id, company_id);
CREATE UNIQUE INDEX people_payroll_pay_windows_company_id_code_index ON people_payroll_pay_windows (company_id, code);
CREATE TABLE people_payroll_mappings (
    id bigserial PRIMARY KEY,
    tenant_id bigint NOT NULL,
    company_id bigint NOT NULL,
    created_by_actor_id bigint NOT NULL,
    source_kind varchar(20) NOT NULL,
    source_key varchar(100) NOT NULL,
    item_id bigint NOT NULL,
    effective_from date NOT NULL,
    effective_to date,
    inserted_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    updated_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    FOREIGN KEY (tenant_id, company_id, item_id) REFERENCES people_payroll_items (tenant_id, company_id, id) ON DELETE RESTRICT,
    CONSTRAINT people_payroll_mappings_dates CHECK (effective_to IS NULL OR effective_to >= effective_from),
    CONSTRAINT people_payroll_mappings_source CHECK (source_kind IN ('leave', 'claims'))
);
CREATE INDEX ON people_payroll_mappings (tenant_id, company_id);
CREATE TABLE people_payroll_setup_snapshots (
    id bigserial PRIMARY KEY,
    tenant_id bigint NOT NULL,
    company_id bigint NOT NULL,
    created_by_actor_id bigint NOT NULL,
    period_id bigint NOT NULL,
    country varchar(100) NOT NULL,
    currency varchar(3) NOT NULL,
    snapshot jsonb NOT NULL,
    locked_at timestamp,
    locked_by_actor_id bigint,
    inserted_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    updated_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    UNIQUE (tenant_id, company_id, id),
    FOREIGN KEY (tenant_id, company_id, period_id) REFERENCES people_payroll_pay_windows (tenant_id, company_id, id) ON DELETE RESTRICT,
    CONSTRAINT people_payroll_setup_snapshots_lock CHECK ((locked_at IS NULL) = (locked_by_actor_id IS NULL)),
    CONSTRAINT people_payroll_setup_snapshots_currency CHECK (currency ~ '^[A-Z]{3}$')
);
CREATE INDEX ON people_payroll_setup_snapshots (tenant_id, company_id);
CREATE UNIQUE INDEX people_payroll_setup_snapshots_period_id_currency_index ON people_payroll_setup_snapshots (period_id, currency);
CREATE FUNCTION people_payroll_immutable() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'payroll foundation records are immutable' USING ERRCODE = '23514';
END $$;
CREATE TRIGGER people_payroll_classifications_immutable BEFORE UPDATE OR DELETE ON people_payroll_classifications FOR EACH ROW EXECUTE FUNCTION people_payroll_immutable();
CREATE TRIGGER people_payroll_items_immutable BEFORE UPDATE OR DELETE ON people_payroll_items FOR EACH ROW EXECUTE FUNCTION people_payroll_immutable();
CREATE TRIGGER people_payroll_pay_windows_immutable BEFORE UPDATE OR DELETE ON people_payroll_pay_windows FOR EACH ROW EXECUTE FUNCTION people_payroll_immutable();
CREATE TRIGGER people_payroll_mappings_immutable BEFORE UPDATE OR DELETE ON people_payroll_mappings FOR EACH ROW EXECUTE FUNCTION people_payroll_immutable();
CREATE FUNCTION people_payroll_guard_run() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' OR OLD.locked_at IS NOT NULL THEN
        RAISE EXCEPTION 'payroll run cannot change' USING ERRCODE = '23514';
    END IF;
    IF (to_jsonb(NEW) - 'locked_at' - 'locked_by_actor_id' - 'updated_at') IS DISTINCT FROM
       (to_jsonb(OLD) - 'locked_at' - 'locked_by_actor_id' - 'updated_at') OR NEW.locked_at IS NULL THEN
        RAISE EXCEPTION 'only payroll run locking is permitted' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER people_payroll_setup_snapshots_guard BEFORE UPDATE OR DELETE ON people_payroll_setup_snapshots FOR EACH ROW EXECUTE FUNCTION people_payroll_guard_run();
