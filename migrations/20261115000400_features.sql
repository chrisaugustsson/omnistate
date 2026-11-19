-- Features. The name "feature" was freed when the old table became `tracks`;
-- it now means a capability of a part, what the part can do, apart from the
-- work on it. A feature belongs to one part of its project and has a name that
-- is unique in its part ignoring case among the features that are not
-- deleted, and a description. A delete is soft: `deleted_at` is set, every list
-- and read leaves the feature out and its name can be used again. The unique
-- key on `(workspace_id, part_id, id)` lets later tables tie a record to a
-- feature of its own part. The table follows the checklist of
-- migrations/README.md.
CREATE TABLE features (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT features_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    project_id uuid NOT NULL,
    part_id uuid NOT NULL,
    name text NOT NULL CONSTRAINT features_name_not_blank CHECK (btrim(name) <> ''),
    description text NOT NULL DEFAULT '',
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    deleted_at timestamptz,
    UNIQUE (workspace_id, id),
    UNIQUE (workspace_id, part_id, id),
    FOREIGN KEY (workspace_id, project_id, part_id) REFERENCES parts (workspace_id, project_id, id)
);
CREATE UNIQUE INDEX features_name_idx ON features (workspace_id, part_id, lower(name))
    WHERE deleted_at IS NULL;
CREATE INDEX features_part_idx ON features (workspace_id, part_id, created_at, id);

ALTER TABLE features ENABLE ROW LEVEL SECURITY;
ALTER TABLE features FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON features
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE ON features TO omni_api;
