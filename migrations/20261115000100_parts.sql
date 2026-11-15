-- Parts. A project is made of parts, the apps and services it builds, and work
-- and capabilities hang off the part they belong to. A part has a name that is
-- unique in its project ignoring case among the parts that are not deleted, a
-- kind (`app` or `service`) and a description. A delete is soft: `deleted_at`
-- is set, every list and read leaves the part out and its name can be used
-- again. The unique key on `(workspace_id, project_id, id)` lets later tables
-- tie a record to a part of its own project. The table follows the checklist of
-- migrations/README.md.
CREATE TABLE parts (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT parts_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    project_id uuid NOT NULL,
    name text NOT NULL CONSTRAINT parts_name_not_blank CHECK (btrim(name) <> ''),
    kind text NOT NULL CONSTRAINT parts_kind_valid CHECK (kind IN ('app', 'service')),
    description text NOT NULL DEFAULT '',
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    deleted_at timestamptz,
    UNIQUE (workspace_id, id),
    UNIQUE (workspace_id, project_id, id),
    FOREIGN KEY (workspace_id, project_id) REFERENCES projects (workspace_id, id)
);
CREATE UNIQUE INDEX parts_name_idx ON parts (workspace_id, project_id, lower(name))
    WHERE deleted_at IS NULL;
CREATE INDEX parts_project_idx ON parts (workspace_id, project_id, created_at, id);

ALTER TABLE parts ENABLE ROW LEVEL SECURITY;
ALTER TABLE parts FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON parts
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE ON parts TO omni_api;
