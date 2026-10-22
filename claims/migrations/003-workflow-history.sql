-- Workflow history and hand-off snapshots are permanent records. Mirrors
-- Bilimbi Claims' append-only use of RequestEvent and HandoffBatch.
CREATE FUNCTION people_claim_history_immutable() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'claim history and hand-off batches are immutable';
END
$$;
CREATE TRIGGER people_claim_request_events_immutable
  BEFORE UPDATE OR DELETE ON people_claim_request_events
  FOR EACH ROW EXECUTE FUNCTION people_claim_history_immutable();
CREATE TRIGGER people_claim_handoff_batches_immutable
  BEFORE UPDATE OR DELETE ON people_claim_handoff_batches
  FOR EACH ROW EXECUTE FUNCTION people_claim_history_immutable();

CREATE FUNCTION people_claim_handoff_membership_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF OLD.handoff_batch_id IS NOT NULL AND
     (NEW.handoff_batch_id IS DISTINCT FROM OLD.handoff_batch_id OR
      NEW.approved_amount IS DISTINCT FROM OLD.approved_amount OR
      NEW.currency IS DISTINCT FROM OLD.currency) THEN
    RAISE EXCEPTION 'a handed-off claim cannot change its batch, amount or currency';
  END IF;
  IF NEW.handoff_batch_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM people_claim_handoff_batches b
    WHERE b.tenant_id = NEW.tenant_id AND b.company_id = NEW.company_id
      AND b.id = NEW.handoff_batch_id AND b.currency = NEW.currency
  ) THEN
    RAISE EXCEPTION 'a hand-off batch holds one company and currency';
  END IF;
  RETURN NEW;
END
$$;
CREATE TRIGGER people_claim_submissions_handoff_guard
  BEFORE UPDATE ON people_claim_submissions
  FOR EACH ROW EXECUTE FUNCTION people_claim_handoff_membership_guard();
