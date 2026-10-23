-- US-102: a doc has a head, the content it has now. The head is the name of a
-- chunk of RustFS (`ws/<workspace>/chunks/<head>`): for an HTML doc the chunk of
-- its HTML, for a design the chunk of its manifest of pages and boards. Reads
-- of a doc go through its head and no longer through `mainline_version_id`,
-- which only the change sets that are still there write. A doc made before the
-- head existed has the chunk of its mainline version; one that has none (a doc
-- started in a change set that never merged) has a null head, which reads as
-- the empty doc.
ALTER TABLE docs ADD COLUMN head text;
ALTER TABLE docs
    ADD CONSTRAINT docs_head_is_chunk_name CHECK (head IS NULL OR head ~ '^[0-9a-f]{64}$');

UPDATE docs
SET head = v.chunk
FROM versions v
WHERE v.workspace_id = docs.workspace_id AND v.id = docs.mainline_version_id
  AND v.chunk IS NOT NULL;

-- Every write to a doc adds one row (FR-11): who wrote it, when, the operations
-- and the head before and after. Creating a doc is its first row, `seq` 1 with
-- no head before. The log is append-only: the API role gets SELECT and INSERT
-- and no UPDATE. Every table follows the checklist of migrations/README.md.
CREATE TABLE doc_changes (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT doc_changes_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    doc_id uuid NOT NULL,
    -- 1 for the creation of the doc, rising by one with every write.
    seq bigint NOT NULL CHECK (seq >= 1),
    -- {"run", "role", "model", "harness"}, as the agent declared them.
    agent jsonb NOT NULL DEFAULT '{}',
    operations jsonb NOT NULL,
    head_before text,
    head_after text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (workspace_id, id),
    UNIQUE (workspace_id, doc_id, seq),
    FOREIGN KEY (workspace_id, doc_id) REFERENCES docs (workspace_id, id)
);

ALTER TABLE doc_changes ENABLE ROW LEVEL SECURITY;
ALTER TABLE doc_changes FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON doc_changes
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

GRANT SELECT, INSERT ON doc_changes TO omni_api;
