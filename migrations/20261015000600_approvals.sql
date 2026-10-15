-- US-025: approvals. When the human approves a change set, the exact version of
-- each changed doc that they reviewed is recorded here (FR-21): the latest
-- version saved in the change set for the doc, a removal included. A merge
-- publishes these versions and nothing else, so an edit to a draft after its
-- last save is never part of an approval. Saving a version while the change set
-- is approved or needs_rebase clears the approval, which marks its rows
-- `cleared` instead of deleting them; a later approval adds new rows. The
-- table follows the checklist of migrations/README.md.

CREATE TABLE approvals (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT approvals_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    changeset_id uuid NOT NULL,
    doc_id uuid NOT NULL,
    version_id uuid NOT NULL,
    -- True once saving a version has withdrawn the approval.
    cleared boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (workspace_id, id),
    FOREIGN KEY (workspace_id, changeset_id) REFERENCES changesets (workspace_id, id),
    FOREIGN KEY (workspace_id, doc_id) REFERENCES docs (workspace_id, id),
    FOREIGN KEY (workspace_id, version_id) REFERENCES versions (workspace_id, id)
);
CREATE INDEX approvals_changeset_idx ON approvals (workspace_id, changeset_id, created_at, id);
-- A change set has at most one standing approval of a doc.
CREATE UNIQUE INDEX approvals_standing_idx ON approvals (workspace_id, changeset_id, doc_id)
    WHERE NOT cleared;

ALTER TABLE approvals ENABLE ROW LEVEL SECURITY;
ALTER TABLE approvals FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON approvals
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE ON approvals TO omni_api;
