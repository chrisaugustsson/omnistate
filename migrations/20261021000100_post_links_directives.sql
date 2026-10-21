-- US-034: what a post carries besides its text, and which directives an agent
-- identity has been given. Posts stay append-only: the API role has no UPDATE
-- privilege on them (US-033), so the new columns are written with the row.
-- Every column has a default or may be NULL, so a post can be recorded without
-- them.

-- `links` is a JSON list of {kind, ...} objects (a post, a web source, a node
-- address or an attachment) and `attachments` a JSON list of attachment IDs,
-- both in the order the agent gave them; the API checks that what they name
-- exists, since a JSON list cannot carry a foreign key. `replaces` is the
-- earlier post of the same author that this one supersedes, which stays as it
-- was. `confidence` is the author's own, from 0 to 1.
ALTER TABLE posts
    ADD COLUMN links jsonb NOT NULL DEFAULT '[]'::jsonb
        CONSTRAINT posts_links_is_list CHECK (jsonb_typeof(links) = 'array'),
    ADD COLUMN attachments jsonb NOT NULL DEFAULT '[]'::jsonb
        CONSTRAINT posts_attachments_is_list CHECK (jsonb_typeof(attachments) = 'array'),
    ADD COLUMN confidence double precision
        CONSTRAINT posts_confidence_in_range CHECK (confidence BETWEEN 0 AND 1),
    ADD COLUMN replaces uuid,
    ADD CONSTRAINT posts_replaces_fk FOREIGN KEY (workspace_id, replaces) REFERENCES posts (workspace_id, id);

-- The directive posts that an agent identity (its run and its role, as for
-- claims) was given with a request in a change set. A directive is given to
-- each identity once, so a row is never changed; like the status history it is
-- protected by a trigger, and the API role cannot delete it.
CREATE TABLE directive_receipts (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT directive_receipts_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    post_id uuid NOT NULL,
    -- {"run": ..., "role": ...}; either may be null.
    agent jsonb NOT NULL CONSTRAINT directive_receipts_agent_is_object CHECK (jsonb_typeof(agent) = 'object'),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (workspace_id, id),
    UNIQUE (workspace_id, post_id, agent),
    FOREIGN KEY (workspace_id, post_id) REFERENCES posts (workspace_id, id)
);

CREATE TRIGGER directive_receipts_append_only
    BEFORE UPDATE ON directive_receipts
    FOR EACH ROW EXECUTE FUNCTION reject_history_update();

ALTER TABLE directive_receipts ENABLE ROW LEVEL SECURITY;
ALTER TABLE directive_receipts FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON directive_receipts
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE ON directive_receipts TO omni_api;
