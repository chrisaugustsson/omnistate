-- US-059: features (Q51). A project holds global docs and features; a feature
-- is the same thing one level deeper, with its own docs, change sets and
-- threads (later stories), so several features can be worked on at once. A
-- feature has a name that is unique in its project ignoring case, a
-- description and a status, `active` until the human marks it done. Features
-- are never deleted: the API role gets no DELETE. The unique key on
-- `(workspace_id, project_id, id)` lets later tables tie a feature to its own
-- project with a composite foreign key. The table follows the checklist of
-- migrations/README.md.
CREATE TABLE features (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT features_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    project_id uuid NOT NULL,
    name text NOT NULL CONSTRAINT features_name_not_blank CHECK (btrim(name) <> ''),
    description text NOT NULL DEFAULT '',
    status text NOT NULL DEFAULT 'active'
        CONSTRAINT features_status_valid CHECK (status IN ('active', 'done')),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (workspace_id, id),
    UNIQUE (workspace_id, project_id, id),
    FOREIGN KEY (workspace_id, project_id) REFERENCES projects (workspace_id, id)
);
CREATE UNIQUE INDEX features_name_idx ON features (workspace_id, project_id, lower(name));
CREATE INDEX features_project_idx ON features (workspace_id, project_id, created_at, id);

ALTER TABLE features ENABLE ROW LEVEL SECURITY;
ALTER TABLE features FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON features
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE ON features TO omni_api;
