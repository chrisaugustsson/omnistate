-- Rules on behaviour tree nodes. A rule is a constraint a node imposes beyond
-- itself; `scope` says how far it reaches: the node and below, its whole story,
-- or the whole feature. Nothing is copied upwards. A rule is never deleted:
-- `withdrawn_at` marks one that is no longer in force. Every write adds 1 to
-- `version`. The table follows the checklist of migrations/README.md.

CREATE TABLE behaviour_rules (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT behaviour_rules_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    project_id uuid NOT NULL,
    feature_id uuid NOT NULL,
    node_id uuid NOT NULL,
    text text NOT NULL CONSTRAINT behaviour_rules_text_not_blank CHECK (btrim(text) <> ''),
    scope text NOT NULL DEFAULT 'node' CONSTRAINT behaviour_rules_scope_known
        CHECK (scope IN ('node', 'story', 'feature')),
    -- Null while the rule is in force.
    withdrawn_at timestamptz,
    -- The author object: {kind: "agent", run, role, model, harness} or {kind: "human", user_id}.
    author jsonb NOT NULL CONSTRAINT behaviour_rules_author_is_object CHECK (jsonb_typeof(author) = 'object'),
    version integer NOT NULL DEFAULT 1 CONSTRAINT behaviour_rules_version_positive CHECK (version >= 1),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (workspace_id, id),
    UNIQUE (workspace_id, feature_id, id),
    FOREIGN KEY (workspace_id, project_id) REFERENCES projects (workspace_id, id),
    FOREIGN KEY (workspace_id, project_id, feature_id) REFERENCES features (workspace_id, project_id, id),
    FOREIGN KEY (workspace_id, feature_id, node_id) REFERENCES behaviour_nodes (workspace_id, feature_id, id)
);
CREATE INDEX behaviour_rules_feature_idx ON behaviour_rules (workspace_id, feature_id, created_at);
CREATE INDEX behaviour_rules_node_idx ON behaviour_rules (node_id, created_at);

ALTER TABLE behaviour_rules ENABLE ROW LEVEL SECURITY;
ALTER TABLE behaviour_rules FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON behaviour_rules
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE ON behaviour_rules TO omni_api;
