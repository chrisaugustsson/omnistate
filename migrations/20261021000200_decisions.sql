-- US-037: decisions. A thread ends in a decision record (Q20): the direction,
-- the options with their scores and why each lost, assumptions, open
-- questions, dissent and the posts it relies on. An agent proposes it, only
-- the human approves or rejects it, and a later proposal supersedes the
-- earlier decision. A change set can be linked to the decisions it carries
-- out, and may not be approved before they are (Q5). Nothing is deleted. Both
-- tables follow the checklist of migrations/README.md.

-- `content` is the decision file as it was proposed (docs/formats/decision.md);
-- the project and the change set of a decision are those of its thread.
-- `status` is the only column that changes, together with `rejection_reason`,
-- which is kept when a rejected decision is superseded.
CREATE TABLE decisions (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT decisions_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    thread_id uuid NOT NULL,
    status text NOT NULL DEFAULT 'proposed'
        CONSTRAINT decisions_status_valid CHECK (status IN ('proposed', 'approved', 'rejected', 'superseded')),
    content jsonb NOT NULL CONSTRAINT decisions_content_is_object CHECK (jsonb_typeof(content) = 'object'),
    -- The agent identity that proposed it, as for posts.
    author jsonb NOT NULL CONSTRAINT decisions_author_is_object CHECK (jsonb_typeof(author) = 'object'),
    rejection_reason text,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (workspace_id, id),
    FOREIGN KEY (workspace_id, thread_id) REFERENCES threads (workspace_id, id)
);
CREATE INDEX decisions_thread_idx ON decisions (workspace_id, thread_id, created_at, id);
-- A thread has at most one decision that is not superseded: its current one.
CREATE UNIQUE INDEX decisions_one_current_per_thread
    ON decisions (workspace_id, thread_id) WHERE status <> 'superseded';

-- The decisions that a change set carries out, in the order they were linked.
-- A link is never changed or removed: the API role gets SELECT and INSERT and
-- no UPDATE.
CREATE TABLE changeset_decisions (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT changeset_decisions_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    changeset_id uuid NOT NULL,
    decision_id uuid NOT NULL,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (workspace_id, id),
    UNIQUE (workspace_id, changeset_id, decision_id),
    FOREIGN KEY (workspace_id, changeset_id) REFERENCES changesets (workspace_id, id),
    FOREIGN KEY (workspace_id, decision_id) REFERENCES decisions (workspace_id, id)
);

ALTER TABLE decisions ENABLE ROW LEVEL SECURITY;
ALTER TABLE decisions FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON decisions
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

ALTER TABLE changeset_decisions ENABLE ROW LEVEL SECURITY;
ALTER TABLE changeset_decisions FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON changeset_decisions
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE ON decisions TO omni_api;
GRANT SELECT, INSERT ON changeset_decisions TO omni_api;
