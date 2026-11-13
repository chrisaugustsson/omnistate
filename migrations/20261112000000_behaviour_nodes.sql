-- Behaviour tree nodes. A feature has one tree of nodes: the root (no parent,
-- no slug) stands for the feature, its children are user stories and the
-- nodes below are situations. The level and the path are not stored: they
-- follow from the parents and the slugs, so a move changes one row. Every
-- write adds 1 to `version`. Nothing is deleted. The table follows the
-- checklist of migrations/README.md.

CREATE TABLE behaviour_nodes (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT behaviour_nodes_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    project_id uuid NOT NULL,
    feature_id uuid NOT NULL,
    -- Null for the root only.
    parent_id uuid,
    -- Null for the root only; unique among the children of one parent.
    slug text CONSTRAINT behaviour_nodes_slug_shape
        CHECK (slug ~ '^[a-z0-9]+(-[a-z0-9]+)*$' AND char_length(slug) <= 80),
    -- The order among siblings.
    position integer NOT NULL DEFAULT 0,
    title text NOT NULL CONSTRAINT behaviour_nodes_title_not_blank CHECK (btrim(title) <> ''),
    summary text NOT NULL DEFAULT '',
    situation text NOT NULL DEFAULT '',
    behaviour text NOT NULL DEFAULT '',
    status text NOT NULL DEFAULT 'open' CONSTRAINT behaviour_nodes_status_known
        CHECK (status IN ('open', 'done', 'out_of_scope')),
    version integer NOT NULL DEFAULT 1 CONSTRAINT behaviour_nodes_version_positive CHECK (version >= 1),
    -- The author object: {kind: "agent", run, role, model, harness} or {kind: "human", user_id}.
    created_by jsonb NOT NULL CONSTRAINT behaviour_nodes_created_by_is_object CHECK (jsonb_typeof(created_by) = 'object'),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT behaviour_nodes_root_has_no_slug CHECK ((parent_id IS NULL) = (slug IS NULL)),
    UNIQUE (workspace_id, id),
    UNIQUE (workspace_id, feature_id, id),
    FOREIGN KEY (workspace_id, project_id) REFERENCES projects (workspace_id, id),
    FOREIGN KEY (workspace_id, project_id, feature_id) REFERENCES features (workspace_id, project_id, id),
    FOREIGN KEY (workspace_id, feature_id, parent_id) REFERENCES behaviour_nodes (workspace_id, feature_id, id)
);
CREATE UNIQUE INDEX behaviour_nodes_one_root_idx ON behaviour_nodes (feature_id) WHERE parent_id IS NULL;
CREATE UNIQUE INDEX behaviour_nodes_slug_idx ON behaviour_nodes (parent_id, slug) WHERE parent_id IS NOT NULL;
CREATE INDEX behaviour_nodes_feature_idx ON behaviour_nodes (workspace_id, feature_id, parent_id, position);

ALTER TABLE behaviour_nodes ENABLE ROW LEVEL SECURITY;
ALTER TABLE behaviour_nodes FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON behaviour_nodes
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE ON behaviour_nodes TO omni_api;
