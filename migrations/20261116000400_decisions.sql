-- Decisions (D7, D16). A decision is a record on a track of what was decided,
-- why, who decided it and when, optionally about one feature of the track's
-- project. A later decision can replace an earlier one of the same project,
-- which keeps its row. The records are append-only: the API role gets SELECT
-- and INSERT and no UPDATE. The table follows the checklist of
-- migrations/README.md.
CREATE TABLE decisions (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT decisions_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    project_id uuid NOT NULL,
    track_id uuid NOT NULL,
    feature_id uuid,
    question text NOT NULL,
    decision text NOT NULL,
    why text NOT NULL,
    -- The agent identity or the human that recorded it, as for comments.
    author jsonb NOT NULL CONSTRAINT decisions_author_is_object CHECK (jsonb_typeof(author) = 'object'),
    replaces_decision_id uuid,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (workspace_id, id),
    FOREIGN KEY (workspace_id, project_id) REFERENCES projects (workspace_id, id),
    FOREIGN KEY (workspace_id, track_id) REFERENCES tracks (workspace_id, id),
    FOREIGN KEY (workspace_id, feature_id) REFERENCES features (workspace_id, id),
    FOREIGN KEY (workspace_id, replaces_decision_id) REFERENCES decisions (workspace_id, id)
);
CREATE INDEX decisions_track_idx ON decisions (workspace_id, track_id, created_at, id);
CREATE INDEX decisions_feature_idx ON decisions (workspace_id, feature_id, created_at, id)
    WHERE feature_id IS NOT NULL;
CREATE INDEX decisions_replaces_idx ON decisions (workspace_id, replaces_decision_id)
    WHERE replaces_decision_id IS NOT NULL;

ALTER TABLE decisions ENABLE ROW LEVEL SECURITY;
ALTER TABLE decisions FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON decisions
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

GRANT SELECT, INSERT ON decisions TO omni_api;
