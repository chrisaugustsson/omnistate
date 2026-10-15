-- US-024: versions and their node index. A version is an unchangeable snapshot
-- of a change set's draft of a doc, or, when `removed`, the marker that the
-- change set removes the doc (nothing is deleted; the doc and every earlier
-- version stay). The content of a version is stored once, as a chunk of RustFS
-- named by the hash of its bytes (`ws/<workspace>/chunks/<chunk>`), so two
-- versions with the same content share it. `node_index` lists the nodes of a
-- chunk: the ID, type, parent and own-content fingerprint of each, in document
-- order, which is what a comparison of two versions reads. Every table follows
-- the checklist of migrations/README.md.
--
-- Both tables are append-only (FR-18): the API role gets SELECT and INSERT and
-- no UPDATE, and so does `draft_ops`, the operation log of US-022, which until
-- now was only protected by a trigger.

CREATE TABLE versions (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT versions_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    doc_id uuid NOT NULL,
    changeset_id uuid NOT NULL,
    -- The draft that was saved.
    draft_id uuid,
    removed boolean NOT NULL DEFAULT false,
    -- The stored content, NULL for a removal.
    chunk text,
    -- {"run", "role", "model", "harness"}, as the agent declared them.
    agent jsonb NOT NULL DEFAULT '{}',
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (workspace_id, id),
    FOREIGN KEY (workspace_id, doc_id) REFERENCES docs (workspace_id, id),
    FOREIGN KEY (workspace_id, changeset_id) REFERENCES changesets (workspace_id, id),
    FOREIGN KEY (workspace_id, draft_id) REFERENCES drafts (workspace_id, id)
);
CREATE INDEX versions_doc_idx ON versions (workspace_id, doc_id, created_at, id);

-- A chunk's nodes are written once, together, when the chunk is first saved;
-- a later version with the same content finds them there.
CREATE TABLE node_index (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT node_index_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    chunk text NOT NULL,
    node text NOT NULL,
    type text NOT NULL,
    -- The parent's node ID, NULL at the top level.
    parent text,
    fingerprint text NOT NULL,
    -- The node's place in document order.
    ordinal integer NOT NULL DEFAULT 0,
    UNIQUE (workspace_id, id),
    UNIQUE (workspace_id, chunk, node)
);

-- The history of a change set also records what the server itself does: saving
-- a version while approved or needs_rebase clears the approval.
ALTER TABLE changeset_status_history DROP CONSTRAINT changeset_status_history_actor_known;
ALTER TABLE changeset_status_history ADD CONSTRAINT changeset_status_history_actor_known
    CHECK (actor IN ('agent', 'human', 'system'));

ALTER TABLE versions ENABLE ROW LEVEL SECURITY;
ALTER TABLE versions FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON versions
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

ALTER TABLE node_index ENABLE ROW LEVEL SECURITY;
ALTER TABLE node_index FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON node_index
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

GRANT SELECT, INSERT ON versions, node_index TO omni_api;
REVOKE UPDATE ON draft_ops FROM omni_api;
