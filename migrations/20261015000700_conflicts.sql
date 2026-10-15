-- US-026: rebase and conflicts. A rebase replays a change set's changes onto the
-- current mainline; every node that both sides changed is recorded as a
-- conflict with both versions of the node, and the agent resolves it with a
-- note once it has fixed the draft. Conflicts and their resolutions stay
-- stored, merged change sets included, so that the review screen can show them
-- (US-046). Nothing is deleted: resolving updates the row. The table follows
-- the checklist of migrations/README.md.

CREATE TABLE conflicts (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT conflicts_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    changeset_id uuid NOT NULL,
    doc_id uuid NOT NULL,
    -- The node that both sides changed.
    node text NOT NULL,
    -- The mainline version that the rebase found, and the change set's latest
    -- version of the doc: where each side's state of the node comes from.
    mainline_version_id uuid NOT NULL,
    changeset_version_id uuid NOT NULL,
    -- The node as HTML with data-node-id in each version; NULL when that side
    -- removed the node.
    mainline_html text,
    changeset_html text,
    -- The resolution; both are NULL until the agent resolves the conflict.
    note text,
    resolved_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (workspace_id, id),
    CONSTRAINT conflicts_resolution_complete CHECK ((note IS NULL) = (resolved_at IS NULL)),
    FOREIGN KEY (workspace_id, changeset_id) REFERENCES changesets (workspace_id, id),
    FOREIGN KEY (workspace_id, doc_id) REFERENCES docs (workspace_id, id),
    FOREIGN KEY (workspace_id, mainline_version_id) REFERENCES versions (workspace_id, id),
    FOREIGN KEY (workspace_id, changeset_version_id) REFERENCES versions (workspace_id, id)
);
CREATE INDEX conflicts_changeset_idx ON conflicts (workspace_id, changeset_id, created_at, id);

ALTER TABLE conflicts ENABLE ROW LEVEL SECURITY;
ALTER TABLE conflicts FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON conflicts
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE ON conflicts TO omni_api;

-- A rebase gives a draft new content under a new base: the rebased version
-- (`content_version_id`) holds the draft's content from then on, and only the
-- operations logged after `content_after_seq` apply to it, so the draft is
-- that version with those operations replayed. Until the draft is rebased
-- `content_version_id` is NULL and the content is the base version with the
-- whole log replayed. A plain column, not a foreign key: versions already reference
-- drafts, and a cycle would stop a workspace from being deleted.
ALTER TABLE drafts ADD COLUMN content_version_id uuid;
ALTER TABLE drafts ADD COLUMN content_after_seq bigint NOT NULL DEFAULT 0;
