-- US-030: the chunk index. Every chunk that the API stores in RustFS, at
-- `ws/<workspace>/chunks/<hash>`, has one row here, keyed by the workspace and
-- the BLAKE3 hash of its bytes. A design version is a manifest of the chunks of
-- its pages and boards, and an unchanged page or board keeps its chunk from
-- version to version, so a chunk is indexed once however many versions list it
-- (and `node_index` lists the nodes of a chunk once, for all of them). The
-- same hash may be indexed by another workspace: chunks are never shared
-- across workspaces. Every table follows the checklist of migrations/README.md.
--
-- The table is append-only (FR-18): the API role gets SELECT and INSERT and no
-- UPDATE, since a chunk's name follows from its content and never changes.

CREATE TABLE chunks (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT chunks_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    -- 64 lowercase hexadecimal characters.
    hash text NOT NULL CONSTRAINT chunks_hash_is_blake3_hex CHECK (hash ~ '^[0-9a-f]{64}$'),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (workspace_id, id),
    UNIQUE (workspace_id, hash)
);

ALTER TABLE chunks ENABLE ROW LEVEL SECURITY;
ALTER TABLE chunks FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON chunks
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

GRANT SELECT, INSERT ON chunks TO omni_api;

-- A design version also keeps its manifest as JSON, `{"pages": [{"id", "chunk",
-- "boards": [{"id", "chunk"}]}]}`, so that listing a version's nodes or
-- comparing two versions reads the node index of its chunks and nothing from
-- RustFS. It is NULL for the versions of other docs. The manifest is also the
-- content of the version's own chunk (`versions.chunk`). A version is
-- append-only, so the column is written once, with the version.
ALTER TABLE versions ADD COLUMN manifest jsonb;
