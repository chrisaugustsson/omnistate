-- US-033: threads and phases. A thread is a question that agents discuss in a
-- change set, with the criteria for a good answer; its phases, numbered from
-- 1, are open or sealed (Q7, Q21) and the last one is the current phase. Posts
-- belong to a thread and carry the number of the phase they were posted in. A
-- thread outlives its change set: it keeps taking posts and phases after the
-- change set merges, so builders can raise problems with the plan. Nothing is
-- deleted. Every table follows the checklist of migrations/README.md.

CREATE TABLE threads (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT threads_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    project_id uuid NOT NULL,
    changeset_id uuid NOT NULL,
    question text NOT NULL CONSTRAINT threads_question_not_blank CHECK (btrim(question) <> ''),
    -- The criteria, a JSON list of texts in the order the agent gave them.
    criteria jsonb NOT NULL CONSTRAINT threads_criteria_is_list CHECK (jsonb_typeof(criteria) = 'array'),
    -- The run of the agent that started the thread.
    run_tag text,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (workspace_id, id),
    FOREIGN KEY (workspace_id, project_id) REFERENCES projects (workspace_id, id),
    FOREIGN KEY (workspace_id, changeset_id) REFERENCES changesets (workspace_id, id)
);
CREATE INDEX threads_changeset_idx ON threads (workspace_id, changeset_id, created_at, id);
CREATE INDEX threads_project_idx ON threads (workspace_id, project_id, created_at, id);

CREATE TABLE thread_phases (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT thread_phases_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    thread_id uuid NOT NULL,
    number integer NOT NULL CONSTRAINT thread_phases_number_positive CHECK (number >= 1),
    sealed boolean NOT NULL,
    started_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (workspace_id, id),
    UNIQUE (workspace_id, thread_id, number),
    FOREIGN KEY (workspace_id, thread_id) REFERENCES threads (workspace_id, id)
);

-- Posts are never edited or deleted: the API role gets SELECT and
-- INSERT and no UPDATE. `phase` is the number of the thread's phase that the
-- post was made in; it is not a foreign key, since a phase is only a number.
-- `author` is the agent identity of the request (run, role, model, harness) or
-- the human.
CREATE TABLE posts (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT posts_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    thread_id uuid NOT NULL,
    phase integer NOT NULL CONSTRAINT posts_phase_positive CHECK (phase >= 1),
    type text NOT NULL CONSTRAINT posts_type_not_blank CHECK (btrim(type) <> ''),
    summary text NOT NULL CONSTRAINT posts_summary_not_blank CHECK (btrim(summary) <> ''),
    body text NOT NULL,
    author jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (workspace_id, id),
    FOREIGN KEY (workspace_id, thread_id) REFERENCES threads (workspace_id, id)
);
CREATE INDEX posts_thread_idx ON posts (workspace_id, thread_id, created_at, id);

ALTER TABLE threads ENABLE ROW LEVEL SECURITY;
ALTER TABLE threads FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON threads
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

ALTER TABLE thread_phases ENABLE ROW LEVEL SECURITY;
ALTER TABLE thread_phases FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON thread_phases
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

ALTER TABLE posts ENABLE ROW LEVEL SECURITY;
ALTER TABLE posts FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON posts
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE ON threads, thread_phases TO omni_api;
GRANT SELECT, INSERT ON posts TO omni_api;
