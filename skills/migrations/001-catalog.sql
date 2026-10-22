-- Skills 001: the catalog, as Bilimbi's
-- `20260930181101_create_people_skills_catalog` leaves it: categories,
-- skills, proficiency scales with their levels, requirement profiles with
-- their items and selectors, and the guards.
--
-- Codes are lowercase identifiers chosen by the company, unique per company.
-- A skill's code is its stable identity. A scale or profile code has numbered
-- versions; at most one draft and one published version exist per company
-- and code. Items and selectors belong to one profile version. Every row
-- carries the tenant and the company, and nothing here is deleted but a
-- draft. `actor_user_id` records who published or retired a version.
--
-- Dates and timestamps are bound and read as text by the facade; `timestamp(0)`
-- holds UTC wall clocks as Bilimbi's `naive_datetime` did.

CREATE TABLE people_skill_categories (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  code varchar(80) NOT NULL,
  name varchar(160) NOT NULL,
  description varchar(2000),
  active boolean NOT NULL,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  CONSTRAINT people_skill_categories_company_code_unique UNIQUE (company_id, code)
);
CREATE INDEX people_skill_categories_tenant_id_company_id_index
  ON people_skill_categories (tenant_id, company_id);

CREATE TABLE people_skills (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  category_id bigint NOT NULL REFERENCES people_skill_categories (id) ON DELETE RESTRICT,
  code varchar(80) NOT NULL,
  name varchar(160) NOT NULL,
  definition varchar(2000) NOT NULL,
  evidence_guide varchar(2000),
  critical boolean NOT NULL,
  reassessment_months integer,
  active boolean NOT NULL,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  CONSTRAINT people_skills_company_code_unique UNIQUE (company_id, code),
  CONSTRAINT people_skills_reassessment_months_range
    CHECK (reassessment_months IS NULL OR reassessment_months BETWEEN 1 AND 120)
);
CREATE INDEX people_skills_tenant_id_company_id_category_id_index
  ON people_skills (tenant_id, company_id, category_id);

CREATE TABLE people_skill_scales (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  code varchar(80) NOT NULL,
  name varchar(160) NOT NULL,
  version integer NOT NULL,
  status varchar(16) NOT NULL,
  published_at timestamp(0),
  retired_at timestamp(0),
  actor_user_id bigint,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  CONSTRAINT people_skill_scales_code_version_unique UNIQUE (company_id, code, version),
  CONSTRAINT people_skill_scales_status CHECK (status IN ('draft', 'published', 'retired'))
);
-- At most one open draft and one published version per company and code.
CREATE UNIQUE INDEX people_skill_scales_one_draft
  ON people_skill_scales (company_id, code) WHERE status = 'draft';
CREATE UNIQUE INDEX people_skill_scales_one_published
  ON people_skill_scales (company_id, code) WHERE status = 'published';
CREATE INDEX people_skill_scales_tenant_id_company_id_index
  ON people_skill_scales (tenant_id, company_id);

CREATE TABLE people_skill_scale_levels (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  scale_id bigint NOT NULL REFERENCES people_skill_scales (id) ON DELETE RESTRICT,
  level integer NOT NULL,
  name varchar(100) NOT NULL,
  anchor varchar(2000) NOT NULL,
  authority varchar(2000) NOT NULL,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  CONSTRAINT people_skill_scale_levels_scale_level_unique UNIQUE (scale_id, level),
  CONSTRAINT people_skill_scale_levels_level_range CHECK (level BETWEEN 0 AND 20)
);

CREATE TABLE people_skill_profiles (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  code varchar(80) NOT NULL,
  name varchar(160) NOT NULL,
  version integer NOT NULL,
  status varchar(16) NOT NULL,
  scale_id bigint NOT NULL REFERENCES people_skill_scales (id) ON DELETE RESTRICT,
  effective_from date,
  effective_to date,
  published_at timestamp(0),
  retired_at timestamp(0),
  actor_user_id bigint,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  CONSTRAINT people_skill_profiles_code_version_unique UNIQUE (company_id, code, version),
  CONSTRAINT people_skill_profiles_status CHECK (status IN ('draft', 'published', 'retired')),
  CONSTRAINT people_skill_profiles_effective_range
    CHECK (effective_to IS NULL OR effective_to >= effective_from),
  CONSTRAINT people_skill_profiles_published_dated
    CHECK (status = 'draft' OR effective_from IS NOT NULL)
);
CREATE UNIQUE INDEX people_skill_profiles_one_draft
  ON people_skill_profiles (company_id, code) WHERE status = 'draft';
CREATE UNIQUE INDEX people_skill_profiles_one_published
  ON people_skill_profiles (company_id, code) WHERE status = 'published';
CREATE INDEX people_skill_profiles_tenant_id_company_id_index
  ON people_skill_profiles (tenant_id, company_id);

CREATE TABLE people_skill_profile_items (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  profile_id bigint NOT NULL REFERENCES people_skill_profiles (id) ON DELETE RESTRICT,
  skill_id bigint NOT NULL REFERENCES people_skills (id) ON DELETE RESTRICT,
  sequence integer NOT NULL,
  required_level integer NOT NULL,
  criticality varchar(16) NOT NULL,
  weight_percent numeric(5, 2) NOT NULL,
  mandatory boolean NOT NULL,
  evidence_standard varchar(2000),
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  CONSTRAINT people_skill_profile_items_profile_skill_unique UNIQUE (profile_id, skill_id),
  CONSTRAINT people_skill_profile_items_profile_sequence_unique UNIQUE (profile_id, sequence),
  CONSTRAINT people_skill_profile_items_criticality
    CHECK (criticality IN ('critical', 'essential', 'development')),
  CONSTRAINT people_skill_profile_items_weight_range
    CHECK (weight_percent >= 0 AND weight_percent <= 100)
);

CREATE TABLE people_skill_profile_selectors (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  profile_id bigint NOT NULL REFERENCES people_skill_profiles (id) ON DELETE RESTRICT,
  selector_type varchar(16) NOT NULL,
  position_id bigint,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  CONSTRAINT people_skill_profile_selectors_unique
    UNIQUE NULLS NOT DISTINCT (profile_id, selector_type, position_id),
  CONSTRAINT people_skill_profile_selectors_target
    CHECK ((selector_type = 'company' AND position_id IS NULL) OR
           (selector_type = 'position' AND position_id IS NOT NULL))
);

-- The database refuses what the facade never does: a changed skill code or
-- owner, and any edit to a published or retired scale or profile, or to its
-- levels, items or selectors. The only permitted change after draft is
-- published to retired.

CREATE FUNCTION people_skills_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.code <> OLD.code OR NEW.company_id <> OLD.company_id
     OR NEW.tenant_id <> OLD.tenant_id THEN
    RAISE EXCEPTION 'skill % code and company are stable', OLD.id;
  END IF;
  RETURN NEW;
END;
$$;
CREATE TRIGGER people_skills_guard BEFORE UPDATE ON people_skills
FOR EACH ROW EXECUTE FUNCTION people_skills_guard();

CREATE FUNCTION people_skill_scales_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF OLD.status = 'draft' THEN
    IF TG_OP = 'DELETE' THEN RETURN OLD; END IF;
    IF NEW.code <> OLD.code OR NEW.version <> OLD.version
       OR NEW.company_id <> OLD.company_id OR NEW.tenant_id <> OLD.tenant_id THEN
      RAISE EXCEPTION 'skill scale % identity is stable', OLD.id;
    END IF;
    RETURN NEW;
  END IF;
  IF TG_OP = 'UPDATE' AND OLD.status = 'published' AND NEW.status = 'retired'
     AND NEW.code = OLD.code AND NEW.name = OLD.name AND NEW.version = OLD.version
     AND NEW.company_id = OLD.company_id AND NEW.tenant_id = OLD.tenant_id
     AND NEW.published_at IS NOT DISTINCT FROM OLD.published_at THEN
    RETURN NEW;
  END IF;
  RAISE EXCEPTION 'skill scale % is % and immutable', OLD.id, OLD.status;
END;
$$;
CREATE TRIGGER people_skill_scales_guard BEFORE UPDATE OR DELETE ON people_skill_scales
FOR EACH ROW EXECUTE FUNCTION people_skill_scales_guard();

CREATE FUNCTION people_skill_scale_levels_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  parent_status text;
BEGIN
  IF TG_OP IN ('UPDATE', 'DELETE') THEN
    SELECT status INTO parent_status FROM people_skill_scales WHERE id = OLD.scale_id;
    IF parent_status IS DISTINCT FROM 'draft' THEN
      RAISE EXCEPTION 'skill scale % is not draft; its levels are immutable', OLD.scale_id;
    END IF;
  END IF;
  IF TG_OP IN ('INSERT', 'UPDATE') THEN
    SELECT status INTO parent_status FROM people_skill_scales WHERE id = NEW.scale_id;
    IF parent_status IS DISTINCT FROM 'draft' THEN
      RAISE EXCEPTION 'skill scale % is not draft; its levels are immutable', NEW.scale_id;
    END IF;
    RETURN NEW;
  END IF;
  RETURN OLD;
END;
$$;
CREATE TRIGGER people_skill_scale_levels_guard
BEFORE INSERT OR UPDATE OR DELETE ON people_skill_scale_levels
FOR EACH ROW EXECUTE FUNCTION people_skill_scale_levels_guard();

CREATE FUNCTION people_skill_profiles_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF OLD.status = 'draft' THEN
    IF TG_OP = 'DELETE' THEN RETURN OLD; END IF;
    IF NEW.code <> OLD.code OR NEW.version <> OLD.version
       OR NEW.company_id <> OLD.company_id OR NEW.tenant_id <> OLD.tenant_id THEN
      RAISE EXCEPTION 'skill profile % identity is stable', OLD.id;
    END IF;
    RETURN NEW;
  END IF;
  IF TG_OP = 'UPDATE' AND OLD.status = 'published' AND NEW.status = 'retired'
     AND NEW.code = OLD.code AND NEW.name = OLD.name AND NEW.version = OLD.version
     AND NEW.company_id = OLD.company_id AND NEW.tenant_id = OLD.tenant_id
     AND NEW.scale_id = OLD.scale_id AND NEW.effective_from = OLD.effective_from
     AND NEW.published_at IS NOT DISTINCT FROM OLD.published_at
     AND NEW.effective_to IS NOT NULL THEN
    RETURN NEW;
  END IF;
  RAISE EXCEPTION 'skill profile % is % and immutable', OLD.id, OLD.status;
END;
$$;
CREATE TRIGGER people_skill_profiles_guard BEFORE UPDATE OR DELETE ON people_skill_profiles
FOR EACH ROW EXECUTE FUNCTION people_skill_profiles_guard();

CREATE FUNCTION people_skill_profile_children_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  parent_status text;
BEGIN
  IF TG_OP IN ('UPDATE', 'DELETE') THEN
    SELECT status INTO parent_status FROM people_skill_profiles WHERE id = OLD.profile_id;
    IF parent_status IS DISTINCT FROM 'draft' THEN
      RAISE EXCEPTION 'skill profile % is not draft; its % are immutable',
        OLD.profile_id, TG_TABLE_NAME;
    END IF;
  END IF;
  IF TG_OP IN ('INSERT', 'UPDATE') THEN
    SELECT status INTO parent_status FROM people_skill_profiles WHERE id = NEW.profile_id;
    IF parent_status IS DISTINCT FROM 'draft' THEN
      RAISE EXCEPTION 'skill profile % is not draft; its % are immutable',
        NEW.profile_id, TG_TABLE_NAME;
    END IF;
    RETURN NEW;
  END IF;
  RETURN OLD;
END;
$$;
CREATE TRIGGER people_skill_profile_items_guard
BEFORE INSERT OR UPDATE OR DELETE ON people_skill_profile_items
FOR EACH ROW EXECUTE FUNCTION people_skill_profile_children_guard();
CREATE TRIGGER people_skill_profile_selectors_guard
BEFORE INSERT OR UPDATE OR DELETE ON people_skill_profile_selectors
FOR EACH ROW EXECUTE FUNCTION people_skill_profile_children_guard();
