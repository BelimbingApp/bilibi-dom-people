-- Bilimbi People.Progression.Migrations.CreateProgression, fresh-schema port.
CREATE TABLE people_progression_policy_versions (
    id bigserial PRIMARY KEY,
    tenant_id bigint NOT NULL,
    company_id bigint NOT NULL,
    code text NOT NULL,
    version integer NOT NULL,
    name text NOT NULL,
    effective_from date NOT NULL,
    rules jsonb NOT NULL,
    status text NOT NULL,
    actor_user_id bigint NOT NULL,
    published_by_user_id bigint,
    published_at timestamp(0) without time zone,
    inserted_at timestamp(0) without time zone NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    updated_at timestamp(0) without time zone NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    CONSTRAINT people_progression_policy_identity UNIQUE (company_id, code, version),
    CONSTRAINT people_progression_policy_content CHECK (
        version > 0 AND length(btrim(code)) > 0 AND length(btrim(name)) > 0
        AND jsonb_typeof(rules) = 'object'
    ),
    CONSTRAINT people_progression_policy_workflow CHECK (
        (status = 'draft' AND published_at IS NULL AND published_by_user_id IS NULL) OR
        (status = 'published' AND published_at IS NOT NULL AND published_by_user_id IS NOT NULL)
    )
);
CREATE INDEX people_progression_policy_scope ON people_progression_policy_versions (tenant_id, company_id);
CREATE FUNCTION people_progression_policy_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' OR OLD.status = 'published' OR
       (to_jsonb(NEW) - ARRAY['status','published_at','published_by_user_id','updated_at']) IS DISTINCT FROM
       (to_jsonb(OLD) - ARRAY['status','published_at','published_by_user_id','updated_at']) OR NEW.status <> 'published' THEN
        RAISE EXCEPTION 'Policy versions are immutable; publish a new version' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER people_progression_policy_immutable BEFORE UPDATE OR DELETE ON people_progression_policy_versions
    FOR EACH ROW EXECUTE FUNCTION people_progression_policy_guard();
