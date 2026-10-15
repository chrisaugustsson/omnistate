-- US-022: docs, their drafts and the drafts' operation logs. A doc belongs to a
-- project; a draft is a change set's working copy of a doc, edited with
-- operation batches (docs/formats/operations.md), and every applied operation
-- is one row of the draft's log. Every table follows the checklist of
-- migrations/README.md.

CREATE TABLE docs (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT docs_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    project_id uuid NOT NULL,
    -- html for PRD, architecture and DB docs, design and design-system for UI
    -- (FR-13); the API checks which kind goes with which module.
    kind text NOT NULL CONSTRAINT docs_kind_known CHECK (kind IN ('html', 'design', 'design-system')),
    module text NOT NULL CONSTRAINT docs_module_known CHECK (module IN ('prd', 'architecture', 'db', 'ui')),
    title text NOT NULL CONSTRAINT docs_title_not_blank CHECK (btrim(title) <> ''),
    -- The mainline version of the doc; NULL until a change set that holds its
    -- first version merges (Q18). The versions table, and the foreign key to
    -- it, come with the stories that save and merge versions.
    mainline_version_id uuid,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (workspace_id, id),
    FOREIGN KEY (workspace_id, project_id) REFERENCES projects (workspace_id, id)
);
CREATE INDEX docs_project_idx ON docs (workspace_id, project_id, created_at, id);

-- A change set's draft of a doc: one per doc and change set. base_version_id is
-- the version it started from, NULL for a new doc whose first version comes
-- through this change set.
CREATE TABLE drafts (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT drafts_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    doc_id uuid NOT NULL,
    changeset_id uuid NOT NULL,
    base_version_id uuid,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (workspace_id, id),
    UNIQUE (workspace_id, doc_id, changeset_id),
    FOREIGN KEY (workspace_id, doc_id) REFERENCES docs (workspace_id, id),
    FOREIGN KEY (workspace_id, changeset_id) REFERENCES changesets (workspace_id, id)
);
CREATE INDEX drafts_changeset_idx ON drafts (workspace_id, changeset_id);

-- The log of a draft: one row per applied operation, in the order they were
-- applied (seq rises by one). A draft's content is its base version with these
-- operations replayed in order, so a row also records what the operation did:
-- `created` holds the node IDs that the server assigned, which a replay must
-- hand out again. Append-only: rows are inserted and never changed.
CREATE TABLE draft_ops (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT draft_ops_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    draft_id uuid NOT NULL,
    seq bigint NOT NULL CONSTRAINT draft_ops_seq_positive CHECK (seq > 0),
    -- The operation as the agent sent it.
    op jsonb NOT NULL,
    -- {"run", "role", "model", "harness"}, as the agent declared them.
    agent jsonb NOT NULL,
    applied_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    node text,
    revision bigint,
    created text[] NOT NULL DEFAULT '{}',
    removed text[] NOT NULL DEFAULT '{}',
    UNIQUE (workspace_id, id),
    UNIQUE (workspace_id, draft_id, seq),
    FOREIGN KEY (workspace_id, draft_id) REFERENCES drafts (workspace_id, id)
);

CREATE TRIGGER draft_ops_append_only
    BEFORE UPDATE ON draft_ops
    FOR EACH ROW EXECUTE FUNCTION reject_history_update();

ALTER TABLE docs ENABLE ROW LEVEL SECURITY;
ALTER TABLE docs FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON docs
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

ALTER TABLE drafts ENABLE ROW LEVEL SECURITY;
ALTER TABLE drafts FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON drafts
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

ALTER TABLE draft_ops ENABLE ROW LEVEL SECURITY;
ALTER TABLE draft_ops FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON draft_ops
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE ON docs, drafts, draft_ops TO omni_api;
