-- Mirrors Bilimbi Training CreateLearningGovernance: immutable facts, decisions,
-- state transitions and effective budget commitments. Employee IDs are People employments.

CREATE TABLE people_training_budget_policies (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  actor_user_id bigint NOT NULL,
  impersonator_id bigint,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  currency text NOT NULL,
  effective_from date NOT NULL,
  effective_to date NOT NULL,
  amount numeric(18,4) NOT NULL CHECK (amount >= 0),
  reason text NOT NULL,
  supersedes_id bigint UNIQUE,
  CHECK (effective_to >= effective_from),
  UNIQUE (id,tenant_id,company_id),
  FOREIGN KEY (tenant_id,company_id) REFERENCES companies(tenant_id,id)
);

CREATE TABLE people_training_needs (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  actor_user_id bigint NOT NULL,
  impersonator_id bigint,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  employee_id bigint NOT NULL,
  course_id bigint,
  need text NOT NULL,
  objective text NOT NULL,
  expected_result text NOT NULL,
  proposed_on date NOT NULL,
  estimated_cost numeric(18,4) NOT NULL,
  currency text NOT NULL,
  status text NOT NULL CHECK (status IN ('draft','pending_hod','pending_hr','pending_approval','approved','rejected','cancelled')),
  budget_policy_id bigint,
  approved_cost numeric(18,4),
  CHECK (estimated_cost >= 0 AND (approved_cost IS NULL OR approved_cost >= 0)),
  CHECK ((status = 'approved' AND budget_policy_id IS NOT NULL AND approved_cost IS NOT NULL) OR (status <> 'approved' AND budget_policy_id IS NULL AND approved_cost IS NULL)),
  FOREIGN KEY (employee_id,company_id) REFERENCES people_workforce_employments(id,company_id),
  UNIQUE (id,tenant_id,company_id),
  FOREIGN KEY (tenant_id,company_id) REFERENCES companies(tenant_id,id)
);

CREATE TABLE people_training_team_plans (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  actor_user_id bigint NOT NULL,
  impersonator_id bigint,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  plan_key text NOT NULL,
  version integer NOT NULL CHECK (version > 0),
  manager_employee_id bigint NOT NULL,
  period_start date NOT NULL,
  period_end date NOT NULL,
  objectives text NOT NULL,
  status text NOT NULL CHECK (status IN ('draft','submitted','approved','rejected','cancelled','superseded')),
  prior_plan_id bigint,
  reason text NOT NULL,
  UNIQUE (company_id,plan_key,version),
  CHECK (period_end >= period_start),
  FOREIGN KEY (manager_employee_id,company_id) REFERENCES people_workforce_employments(id,company_id),
  UNIQUE (id,tenant_id,company_id),
  FOREIGN KEY (tenant_id,company_id) REFERENCES companies(tenant_id,id)
);

CREATE TABLE people_training_team_plan_items (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  actor_user_id bigint NOT NULL,
  impersonator_id bigint,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  plan_id bigint NOT NULL,
  request_id bigint,
  need text NOT NULL,
  expected_result text NOT NULL,
  target_cohort text NOT NULL,
  responsible_owner text NOT NULL,
  intended_timing text NOT NULL,
  evaluation_approach text NOT NULL,
  UNIQUE (id,tenant_id,company_id),
  FOREIGN KEY (tenant_id,company_id) REFERENCES companies(tenant_id,id)
);

CREATE TABLE people_training_need_decisions (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  actor_user_id bigint NOT NULL,
  impersonator_id bigint,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  request_id bigint NOT NULL,
  action text NOT NULL,
  from_status text NOT NULL,
  to_status text NOT NULL,
  reason text NOT NULL,
  UNIQUE (id,tenant_id,company_id),
  FOREIGN KEY (tenant_id,company_id) REFERENCES companies(tenant_id,id)
);

CREATE TABLE people_training_plan_decisions (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  actor_user_id bigint NOT NULL,
  impersonator_id bigint,
  inserted_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  updated_at timestamp(0) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
  plan_id bigint NOT NULL,
  action text NOT NULL,
  from_status text NOT NULL,
  to_status text NOT NULL,
  reason text NOT NULL,
  UNIQUE (id,tenant_id,company_id),
  FOREIGN KEY (tenant_id,company_id) REFERENCES companies(tenant_id,id)
);

ALTER TABLE people_training_budget_policies ADD FOREIGN KEY (supersedes_id,tenant_id,company_id) REFERENCES people_training_budget_policies(id,tenant_id,company_id);
ALTER TABLE people_training_needs ADD FOREIGN KEY (course_id,tenant_id,company_id) REFERENCES people_training_courses(id,tenant_id,company_id);
ALTER TABLE people_training_needs ADD FOREIGN KEY (budget_policy_id,tenant_id,company_id) REFERENCES people_training_budget_policies(id,tenant_id,company_id);
ALTER TABLE people_training_team_plans ADD FOREIGN KEY (prior_plan_id,tenant_id,company_id) REFERENCES people_training_team_plans(id,tenant_id,company_id);
ALTER TABLE people_training_team_plan_items ADD FOREIGN KEY (plan_id,tenant_id,company_id) REFERENCES people_training_team_plans(id,tenant_id,company_id);
ALTER TABLE people_training_team_plan_items ADD FOREIGN KEY (request_id,tenant_id,company_id) REFERENCES people_training_needs(id,tenant_id,company_id);
ALTER TABLE people_training_need_decisions ADD FOREIGN KEY (request_id,tenant_id,company_id) REFERENCES people_training_needs(id,tenant_id,company_id);
ALTER TABLE people_training_plan_decisions ADD FOREIGN KEY (plan_id,tenant_id,company_id) REFERENCES people_training_team_plans(id,tenant_id,company_id);

CREATE INDEX people_training_needs_employee_index ON people_training_needs(tenant_id,company_id,employee_id);
CREATE INDEX people_training_need_decisions_request_index ON people_training_need_decisions(tenant_id,company_id,request_id,id);
CREATE INDEX people_training_plan_decisions_plan_index ON people_training_plan_decisions(tenant_id,company_id,plan_id,id);
CREATE INDEX people_training_team_plan_items_plan_index ON people_training_team_plan_items(tenant_id,company_id,plan_id,id);

CREATE FUNCTION people_training_budget_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  PERFORM pg_advisory_xact_lock(hashtextextended('people.training:' || NEW.tenant_id || ':' || NEW.company_id, 0));
  IF EXISTS (SELECT 1 FROM people_training_budget_policies p WHERE p.tenant_id = NEW.tenant_id AND p.company_id = NEW.company_id AND p.currency = NEW.currency AND p.effective_from <= NEW.effective_to AND p.effective_to >= NEW.effective_from AND p.id IS DISTINCT FROM NEW.supersedes_id AND NOT EXISTS (SELECT 1 FROM people_training_budget_policies s WHERE s.supersedes_id = p.id)) THEN
    RAISE EXCEPTION 'Budget periods overlap' USING ERRCODE = '23514';
  END IF;
  IF NEW.supersedes_id IS NOT NULL AND (
    NOT EXISTS (SELECT 1 FROM people_training_budget_policies p WHERE p.id = NEW.supersedes_id AND p.tenant_id = NEW.tenant_id AND p.company_id = NEW.company_id AND p.currency = NEW.currency) OR
    EXISTS (SELECT 1 FROM people_training_needs r JOIN people_training_budget_policies p ON p.id = NEW.supersedes_id WHERE r.tenant_id = NEW.tenant_id AND r.company_id = NEW.company_id AND r.currency = NEW.currency AND r.status = 'approved' AND r.proposed_on BETWEEN p.effective_from AND p.effective_to AND r.proposed_on NOT BETWEEN NEW.effective_from AND NEW.effective_to)
  ) THEN
    RAISE EXCEPTION 'Invalid budget correction' USING ERRCODE = '23514';
  END IF;
  IF (SELECT COALESCE(sum(approved_cost), 0) FROM people_training_needs r WHERE r.tenant_id = NEW.tenant_id AND r.company_id = NEW.company_id AND r.currency = NEW.currency AND r.status = 'approved' AND r.proposed_on BETWEEN NEW.effective_from AND NEW.effective_to) > NEW.amount THEN
    RAISE EXCEPTION 'Budget below commitments' USING ERRCODE = '23514';
  END IF;
  RETURN NEW;
END $$;

CREATE FUNCTION people_training_commitment_guard() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE p people_training_budget_policies%ROWTYPE; spent numeric;
BEGIN
  PERFORM pg_advisory_xact_lock(hashtextextended('people.training:' || NEW.tenant_id || ':' || NEW.company_id, 0));
  IF NEW.status = 'approved' THEN
    SELECT * INTO p FROM people_training_budget_policies WHERE id = NEW.budget_policy_id AND tenant_id = NEW.tenant_id AND company_id = NEW.company_id AND NOT EXISTS (SELECT 1 FROM people_training_budget_policies s WHERE s.supersedes_id = NEW.budget_policy_id);
    SELECT COALESCE(sum(approved_cost), 0) INTO spent FROM people_training_needs WHERE id <> NEW.id AND tenant_id = NEW.tenant_id AND company_id = NEW.company_id AND currency = NEW.currency AND status = 'approved' AND proposed_on BETWEEN p.effective_from AND p.effective_to;
    IF p.id IS NULL OR p.currency <> NEW.currency OR NEW.proposed_on NOT BETWEEN p.effective_from AND p.effective_to OR NEW.approved_cost IS DISTINCT FROM NEW.estimated_cost OR spent + NEW.approved_cost > p.amount THEN
      RAISE EXCEPTION 'Approval violates effective budget' USING ERRCODE = '23514';
    END IF;
  END IF;
  RETURN NEW;
END $$;

CREATE FUNCTION people_training_governance_immutable() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'Learning governance facts are immutable' USING ERRCODE = '23514';
END $$;

CREATE FUNCTION people_training_governance_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF TG_OP = 'DELETE' OR (to_jsonb(NEW) - ARRAY['status','updated_at','budget_policy_id','approved_cost']) IS DISTINCT FROM (to_jsonb(OLD) - ARRAY['status','updated_at','budget_policy_id','approved_cost']) THEN
    RAISE EXCEPTION 'Learning content cannot be rewritten' USING ERRCODE = '23514';
  END IF;
  IF OLD.status IN ('approved','rejected','cancelled','superseded') AND NOT (TG_TABLE_NAME = 'people_training_team_plans' AND OLD.status = 'approved' AND NEW.status = 'superseded') THEN
    RAISE EXCEPTION 'Learning decision is terminal' USING ERRCODE = '23514';
  END IF;
  IF TG_TABLE_NAME = 'people_training_needs' AND NOT (
    (OLD.status = 'draft' AND NEW.status IN ('pending_hod', 'cancelled')) OR
    (OLD.status = 'pending_hod' AND NEW.status IN ('pending_hr', 'rejected', 'cancelled')) OR
    (OLD.status = 'pending_hr' AND NEW.status IN ('pending_approval', 'rejected', 'cancelled')) OR
    (OLD.status = 'pending_approval' AND NEW.status IN ('approved', 'rejected', 'cancelled'))
  ) THEN
    RAISE EXCEPTION 'Invalid request transition' USING ERRCODE = '23514';
  END IF;
  IF TG_TABLE_NAME = 'people_training_team_plans' AND NOT (
    (OLD.status = 'draft' AND NEW.status IN ('submitted', 'cancelled')) OR
    (OLD.status = 'submitted' AND NEW.status IN ('approved', 'rejected')) OR
    (OLD.status = 'approved' AND NEW.status = 'superseded')
  ) THEN
    RAISE EXCEPTION 'Invalid plan transition' USING ERRCODE = '23514';
  END IF;
  RETURN NEW;
END $$;

CREATE TRIGGER people_training_budget_guard BEFORE INSERT ON people_training_budget_policies FOR EACH ROW EXECUTE FUNCTION people_training_budget_guard();
CREATE TRIGGER people_training_commitment_guard BEFORE INSERT OR UPDATE ON people_training_needs FOR EACH ROW EXECUTE FUNCTION people_training_commitment_guard();
CREATE TRIGGER people_training_budget_policies_immutable BEFORE UPDATE OR DELETE ON people_training_budget_policies FOR EACH ROW EXECUTE FUNCTION people_training_governance_immutable();
CREATE TRIGGER people_training_team_plan_items_immutable BEFORE UPDATE OR DELETE ON people_training_team_plan_items FOR EACH ROW EXECUTE FUNCTION people_training_governance_immutable();
CREATE TRIGGER people_training_need_decisions_immutable BEFORE UPDATE OR DELETE ON people_training_need_decisions FOR EACH ROW EXECUTE FUNCTION people_training_governance_immutable();
CREATE TRIGGER people_training_plan_decisions_immutable BEFORE UPDATE OR DELETE ON people_training_plan_decisions FOR EACH ROW EXECUTE FUNCTION people_training_governance_immutable();
CREATE TRIGGER people_training_needs_guard BEFORE UPDATE OR DELETE ON people_training_needs FOR EACH ROW EXECUTE FUNCTION people_training_governance_guard();
CREATE TRIGGER people_training_team_plans_guard BEFORE UPDATE OR DELETE ON people_training_team_plans FOR EACH ROW EXECUTE FUNCTION people_training_governance_guard();
