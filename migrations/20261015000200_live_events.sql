-- US-017: live events. Every change of a project's data adds one row here under
-- the next sequence number of the project, in the same transaction as the
-- change, and WebSocket clients read their events from this table: that is how
-- a client that reconnects gets what it missed. The table follows the checklist
-- of migrations/README.md.

-- The newest sequence number of the project. An event takes the next number
-- with an UPDATE of this row, so writers of one project queue up on the row
-- lock until their transaction ends: numbers have no gap, and the events become
-- visible in the order of their numbers.
ALTER TABLE projects ADD COLUMN last_event_seq bigint NOT NULL DEFAULT 0;

-- Append-only: rows are inserted and never changed. created_at is the time of
-- the insert itself (clock_timestamp), as in changeset_status_history.
CREATE TABLE live_events (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT live_events_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    project_id uuid NOT NULL,
    seq bigint NOT NULL CONSTRAINT live_events_seq_positive CHECK (seq > 0),
    -- <entity>.<action>, such as changeset.in_review
    event text NOT NULL CONSTRAINT live_events_event_not_blank CHECK (btrim(event) <> ''),
    data jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (workspace_id, id),
    UNIQUE (workspace_id, project_id, seq),
    FOREIGN KEY (workspace_id, project_id) REFERENCES projects (workspace_id, id)
);

CREATE TRIGGER live_events_append_only
    BEFORE UPDATE ON live_events
    FOR EACH ROW EXECUTE FUNCTION reject_history_update();

ALTER TABLE live_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE live_events FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON live_events
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE ON live_events TO omni_api;
