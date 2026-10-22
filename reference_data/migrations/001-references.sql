-- Reference data 001: a company's People references, aliases and calendar
-- exceptions, as Bilimbi's `20260930100101_create_people_reference_data` leaves
-- them (three tables, the same names, columns, unique indexes and the
-- restricting alias foreign key), less the Ecto timestamps' naive type: the
-- columns are UTC `timestamp(0)` as every Bilibi table's are.
--
-- Everything here is a company's own data. No kind, code, label or date is
-- seeded; the `employment_type` kind that Workforce reads is entered by the
-- company that wants it.

CREATE TABLE people_reference_data_entries (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  kind varchar(80) NOT NULL,
  code varchar(100) NOT NULL,
  label varchar(200) NOT NULL,
  active boolean NOT NULL DEFAULT true,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  CONSTRAINT people_reference_data_entries_company_fk
    FOREIGN KEY (tenant_id, company_id) REFERENCES companies (tenant_id, id),
  -- The key an alias names, so an alias cannot sit under another company's or
  -- another kind's entry.
  CONSTRAINT people_reference_data_entries_scope_unique UNIQUE (id, company_id, kind)
);

CREATE UNIQUE INDEX people_reference_data_entries_company_kind_code_unique
  ON people_reference_data_entries (company_id, kind, code);
CREATE INDEX people_reference_data_entries_tenant_id_company_id_index
  ON people_reference_data_entries (tenant_id, company_id);

CREATE TABLE people_reference_data_aliases (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  entry_id bigint NOT NULL,
  kind varchar(80) NOT NULL,
  label varchar(200) NOT NULL,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  CONSTRAINT people_reference_data_aliases_entry_id_fkey
    FOREIGN KEY (entry_id, company_id, kind)
    REFERENCES people_reference_data_entries (id, company_id, kind) ON DELETE RESTRICT,
  CONSTRAINT people_reference_data_aliases_company_fk
    FOREIGN KEY (tenant_id, company_id) REFERENCES companies (tenant_id, id)
);

-- An alias label is unique per kind within a company.
CREATE UNIQUE INDEX people_reference_data_aliases_company_kind_label_unique
  ON people_reference_data_aliases (company_id, kind, label);
CREATE INDEX people_reference_data_aliases_scope_entry_index
  ON people_reference_data_aliases (tenant_id, company_id, entry_id);

CREATE TABLE people_reference_data_calendar_overrides (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  on_date date NOT NULL,
  label varchar(200) NOT NULL,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  CONSTRAINT people_reference_data_calendar_overrides_company_fk
    FOREIGN KEY (tenant_id, company_id) REFERENCES companies (tenant_id, id)
);

CREATE UNIQUE INDEX people_reference_data_calendar_overrides_date_label_unique
  ON people_reference_data_calendar_overrides (company_id, on_date, label);
CREATE INDEX people_reference_data_calendar_overrides_scope_date_index
  ON people_reference_data_calendar_overrides (tenant_id, company_id, on_date);
