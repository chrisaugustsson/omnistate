-- The features a track changes (D15). A row ties a track to a feature of the
-- track's own part; the API refuses any other feature, and a move of the
-- track to another part ends the links to the old part's features. Nothing is
-- erased: an unlink sets `deleted_at`, and linking again adds a new row. At
-- most one live link joins a track and a feature. The table follows the
-- checklist of migrations/README.md.
CREATE TABLE track_features (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT track_features_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    track_id uuid NOT NULL,
    feature_id uuid NOT NULL,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    deleted_at timestamptz,
    UNIQUE (workspace_id, id),
    FOREIGN KEY (workspace_id, track_id) REFERENCES tracks (workspace_id, id),
    FOREIGN KEY (workspace_id, feature_id) REFERENCES features (workspace_id, id)
);
CREATE UNIQUE INDEX track_features_link_idx ON track_features (workspace_id, track_id, feature_id)
    WHERE deleted_at IS NULL;
CREATE INDEX track_features_feature_idx ON track_features (workspace_id, feature_id)
    WHERE deleted_at IS NULL;

ALTER TABLE track_features ENABLE ROW LEVEL SECURITY;
ALTER TABLE track_features FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON track_features
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE ON track_features TO omni_api;
