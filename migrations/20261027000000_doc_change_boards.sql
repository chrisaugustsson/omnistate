-- US-104: a write to a design lists the boards it changed. Each entry is
-- {"board", "before", "after"}: the board's node ID and the name of its chunk
-- in the manifest of the head before and after the write, `before` being null
-- for a new board and `after` null for a removed one. A write to any other doc
-- leaves the column null. The table stays append-only (migrations/README.md).
ALTER TABLE doc_changes ADD COLUMN boards jsonb;
