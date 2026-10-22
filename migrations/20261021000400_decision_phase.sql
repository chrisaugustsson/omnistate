-- US-056: sealed decisions (Q50). A decision belongs to the phase that its
-- thread was in when the agent proposed it, and an agent reads it by the rule
-- of the posts of that phase. The column is set when the decision is
-- proposed and never changes; the default only covers a row inserted without
-- it (the deletion test seeds one).
ALTER TABLE decisions
    ADD COLUMN phase integer NOT NULL DEFAULT 1 CONSTRAINT decisions_phase_positive CHECK (phase >= 1);

-- A decision that exists already was proposed in the phase that was current at
-- its creation.
UPDATE decisions d SET phase = COALESCE((
    SELECT max(ph.number) FROM thread_phases ph
    WHERE ph.workspace_id = d.workspace_id AND ph.thread_id = d.thread_id
      AND ph.started_at <= d.created_at
), 1);
