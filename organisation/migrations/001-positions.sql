-- Organisation 001: positions, their immutable effective-dated title versions
-- and the effective-dated assignments of employees to them, as Bilimbi's
-- `20260930060000_create_people_positions` leaves them, with one difference:
-- an assignment's employee is People's employment record
-- (`people_workforce_employments.id`, the employee id every People table keys
-- on), not Core Employee, which Bilibi does not have. The table names are
-- Bilimbi's (`people_positions`, `people_position_revisions`,
-- `people_position_placements`): persisted names, kept.
--
-- A position is a stable identity of one company; a vacancy leaves it and its
-- title in place. A parent is a position of the same company and never the
-- row itself (the facade also refuses a cycle, under the company lock). Both
-- interval ends are inclusive. The two exclusion constraints are what refuse
-- an overlapping title version and an overlapping substantive assignment even
-- for concurrent writers; acting and concurrent cover may overlap. The
-- `btree_gist` extension is what lets an exclusion constraint combine the
-- equality on `position_id` with the range overlap.

CREATE EXTENSION IF NOT EXISTS btree_gist;

CREATE TABLE people_positions (
  id bigserial PRIMARY KEY,
  company_id bigint NOT NULL REFERENCES companies (id) ON DELETE RESTRICT,
  code varchar(255) NOT NULL,
  parent_id bigint REFERENCES people_positions (id) ON DELETE RESTRICT,
  inserted_at timestamp(6) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(6) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  CONSTRAINT people_positions_company_id_code_index UNIQUE (company_id, code),
  CONSTRAINT people_positions_id_company_id_index UNIQUE (id, company_id),
  CONSTRAINT people_positions_not_own_parent CHECK (parent_id IS NULL OR parent_id <> id),
  -- A parent is a position of the same company.
  CONSTRAINT people_positions_parent_company_fk
    FOREIGN KEY (parent_id, company_id) REFERENCES people_positions (id, company_id) ON DELETE RESTRICT
);

CREATE INDEX people_positions_company_id_parent_id_index
  ON people_positions (company_id, parent_id);
CREATE INDEX people_positions_company_id_id_index
  ON people_positions (company_id, id);

CREATE TABLE people_position_revisions (
  id bigserial PRIMARY KEY,
  position_id bigint NOT NULL REFERENCES people_positions (id) ON DELETE RESTRICT,
  version integer NOT NULL,
  title varchar(255) NOT NULL,
  effective_from date NOT NULL,
  effective_to date,
  inserted_at timestamp(6) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  CONSTRAINT people_position_revisions_position_id_version_index UNIQUE (position_id, version),
  CONSTRAINT people_position_revisions_positive_version CHECK (version > 0),
  CONSTRAINT people_position_revisions_valid_interval
    CHECK (effective_to IS NULL OR effective_to >= effective_from),
  -- One title per position per day; an open end is unbounded.
  CONSTRAINT people_position_revisions_no_overlap
    EXCLUDE USING gist (position_id WITH =, daterange(effective_from, effective_to, '[]') WITH &&)
);

CREATE INDEX people_position_revisions_position_id_effective_from_index
  ON people_position_revisions (position_id, effective_from);

CREATE TABLE people_position_placements (
  id bigserial PRIMARY KEY,
  position_id bigint NOT NULL REFERENCES people_positions (id) ON DELETE RESTRICT,
  employee_id bigint NOT NULL REFERENCES people_workforce_employments (id) ON DELETE RESTRICT,
  kind varchar(255) NOT NULL,
  effective_from date NOT NULL,
  effective_to date,
  inserted_at timestamp(6) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  CONSTRAINT people_position_placements_kind
    CHECK (kind IN ('substantive', 'acting', 'concurrent')),
  CONSTRAINT people_position_placements_valid_interval
    CHECK (effective_to IS NULL OR effective_to >= effective_from),
  -- One substantive holder per position per day; acting and concurrent
  -- cover may overlap it and each other.
  CONSTRAINT people_position_placements_one_substantive
    EXCLUDE USING gist (position_id WITH =, daterange(effective_from, effective_to, '[]') WITH &&)
    WHERE (kind = 'substantive')
);

CREATE INDEX people_position_placements_position_id_effective_from_index
  ON people_position_placements (position_id, effective_from);
CREATE INDEX people_position_placements_employee_id_effective_from_index
  ON people_position_placements (employee_id, effective_from);
