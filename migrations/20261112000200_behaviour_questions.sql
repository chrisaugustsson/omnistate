-- Questions on behaviour tree nodes. A question is anything asked on a node,
-- open or answered; a person's remark is a question too, there is no second
-- kind. `answered_by` says who gave the current answer. Every write adds 1 to
-- `version`. Nothing is deleted. The table follows the checklist of
-- migrations/README.md.

CREATE TABLE behaviour_questions (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT behaviour_questions_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    project_id uuid NOT NULL,
    feature_id uuid NOT NULL,
    node_id uuid NOT NULL,
    question text NOT NULL CONSTRAINT behaviour_questions_question_not_blank CHECK (btrim(question) <> ''),
    state text NOT NULL DEFAULT 'open' CONSTRAINT behaviour_questions_state_known
        CHECK (state IN ('open', 'answered')),
    -- Who gave the current answer; null while the question is open.
    answered_by text CONSTRAINT behaviour_questions_answered_by_known
        CHECK (answered_by IN ('human', 'agent')),
    -- The author object: {kind: "agent", run, role, model, harness} or {kind: "human", user_id}.
    asked_by jsonb NOT NULL CONSTRAINT behaviour_questions_asked_by_is_object CHECK (jsonb_typeof(asked_by) = 'object'),
    version integer NOT NULL DEFAULT 1 CONSTRAINT behaviour_questions_version_positive CHECK (version >= 1),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT behaviour_questions_open_has_no_answerer CHECK ((state = 'open') = (answered_by IS NULL)),
    UNIQUE (workspace_id, id),
    UNIQUE (workspace_id, feature_id, id),
    FOREIGN KEY (workspace_id, project_id) REFERENCES projects (workspace_id, id),
    FOREIGN KEY (workspace_id, project_id, feature_id) REFERENCES features (workspace_id, project_id, id),
    FOREIGN KEY (workspace_id, feature_id, node_id) REFERENCES behaviour_nodes (workspace_id, feature_id, id)
);
CREATE INDEX behaviour_questions_feature_idx ON behaviour_questions (workspace_id, feature_id, state, created_at);
CREATE INDEX behaviour_questions_node_idx ON behaviour_questions (node_id, created_at);

ALTER TABLE behaviour_questions ENABLE ROW LEVEL SECURITY;
ALTER TABLE behaviour_questions FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON behaviour_questions
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE ON behaviour_questions TO omni_api;
