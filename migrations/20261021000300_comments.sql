-- US-038: comments on nodes. A comment is pinned to a node address of a saved
-- version (Q19, FR-40): the ID of a node of the version, or, inside a component
-- instance of a design version, `<instanceId>/<componentNodeId>`. It records the
-- version it was made on, its author and its text, goes open, then addressed
-- (an agent names the version that fixes it), then resolved, and is never
-- deleted: a comment on a node that a later version removed stays and is shown
-- as such. The doc, the project and the change set of a comment are those of its
-- version; whether its node changed or was removed is worked out from the node
-- indexes of the versions (US-024), so nothing about it is stored twice. Both
-- tables follow the checklist of migrations/README.md.

-- `status` and `addressed_in_version_id` are the only columns that change.
-- `addressed_in_version_id` is the version that an agent named when it marked
-- the comment addressed, NULL before that and after a reopen.
CREATE TABLE comments (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT comments_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    version_id uuid NOT NULL,
    node text NOT NULL CONSTRAINT comments_node_not_blank CHECK (btrim(node) <> ''),
    -- {"kind": "human", "user_id"} or {"kind": "agent", "run", "role", "model", "harness"}.
    author jsonb NOT NULL CONSTRAINT comments_author_is_object CHECK (jsonb_typeof(author) = 'object'),
    body text NOT NULL CONSTRAINT comments_body_not_blank CHECK (btrim(body) <> ''),
    status text NOT NULL DEFAULT 'open'
        CONSTRAINT comments_status_valid CHECK (status IN ('open', 'addressed', 'resolved')),
    addressed_in_version_id uuid,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (workspace_id, id),
    FOREIGN KEY (workspace_id, version_id) REFERENCES versions (workspace_id, id),
    FOREIGN KEY (workspace_id, addressed_in_version_id) REFERENCES versions (workspace_id, id)
);
CREATE INDEX comments_version_idx ON comments (workspace_id, version_id, created_at, id);

-- The replies to a comment, oldest first. A reply is never changed: the API role
-- gets SELECT and INSERT and no UPDATE.
CREATE TABLE comment_replies (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT comment_replies_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    comment_id uuid NOT NULL,
    author jsonb NOT NULL CONSTRAINT comment_replies_author_is_object CHECK (jsonb_typeof(author) = 'object'),
    body text NOT NULL CONSTRAINT comment_replies_body_not_blank CHECK (btrim(body) <> ''),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (workspace_id, id),
    FOREIGN KEY (workspace_id, comment_id) REFERENCES comments (workspace_id, id)
);
CREATE INDEX comment_replies_comment_idx ON comment_replies (workspace_id, comment_id, created_at, id);

ALTER TABLE comments ENABLE ROW LEVEL SECURITY;
ALTER TABLE comments FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON comments
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

ALTER TABLE comment_replies ENABLE ROW LEVEL SECURITY;
ALTER TABLE comment_replies FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON comment_replies
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE ON comments TO omni_api;
GRANT SELECT, INSERT ON comment_replies TO omni_api;
