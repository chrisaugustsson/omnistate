-- US-052: a conflict has a kind. A node that both sides changed (US-026) is
-- `both_changed`. A node that the change set added and that a rebase found no
-- place for on the mainline is `unplaced`: the mainline never had a state of
-- it, so its mainline side is empty. Existing conflicts are `both_changed`.

ALTER TABLE conflicts ADD COLUMN kind text NOT NULL DEFAULT 'both_changed'
    CONSTRAINT conflicts_kind_known CHECK (kind IN ('both_changed', 'unplaced'));
