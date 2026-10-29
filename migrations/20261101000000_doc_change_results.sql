-- US-111: the activity list shows the HTML of each element an edit acted on. An
-- insert names no node in its operation (the server assigns the ID), so a write
-- to an HTML doc keeps what each operation did as `results`: [{"node",
-- "revision", "created", "removed"}], one per operation, as the edit answered
-- it. A write to any other doc leaves the column null. The table stays
-- append-only (migrations/README.md).
ALTER TABLE doc_changes ADD COLUMN results jsonb;
