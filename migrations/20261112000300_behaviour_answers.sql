-- Answers to questions on behaviour tree nodes. Append-only: a question's
-- current answer is its newest row, and nothing is updated or deleted.
-- `given_by` says whether a person or an agent on its own gave the answer;
-- `author` is whoever made the call, so an agent that wrote down what a person
-- said has `given_by` human and an agent as `author`. The table follows the
-- checklist of migrations/README.md.

CREATE TABLE behaviour_answers (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT behaviour_answers_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    feature_id uuid NOT NULL,
    question_id uuid NOT NULL,
    answer text NOT NULL CONSTRAINT behaviour_answers_answer_not_blank CHECK (btrim(answer) <> ''),
    given_by text NOT NULL CONSTRAINT behaviour_answers_given_by_known CHECK (given_by IN ('human', 'agent')),
    -- The author object: {kind: "agent", run, role, model, harness} or {kind: "human", user_id}.
    author jsonb NOT NULL CONSTRAINT behaviour_answers_author_is_object CHECK (jsonb_typeof(author) = 'object'),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (workspace_id, id),
    FOREIGN KEY (workspace_id, feature_id, question_id) REFERENCES behaviour_questions (workspace_id, feature_id, id)
);
CREATE INDEX behaviour_answers_question_idx ON behaviour_answers (question_id, created_at);

ALTER TABLE behaviour_answers ENABLE ROW LEVEL SECURITY;
ALTER TABLE behaviour_answers FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON behaviour_answers
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

GRANT SELECT, INSERT ON behaviour_answers TO omni_api;
