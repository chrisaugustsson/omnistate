-- A doc may belong to a feature, a capability of a part, as well as to a track
-- or to the project. `feature_id` sits beside `track_id`; at most one of the
-- two is set, and with both null the doc is at the project level. The composite
-- foreign key ties a doc to a feature of its own project. A design system is
-- always global, so it takes neither.
ALTER TABLE features ADD UNIQUE (workspace_id, project_id, id);
ALTER TABLE docs ADD COLUMN feature_id uuid;
ALTER TABLE docs
    ADD FOREIGN KEY (workspace_id, project_id, feature_id)
    REFERENCES features (workspace_id, project_id, id);
ALTER TABLE docs
    ADD CONSTRAINT docs_one_scope CHECK (feature_id IS NULL OR track_id IS NULL);
ALTER TABLE docs
    ADD CONSTRAINT docs_design_system_is_global_of_feature CHECK (feature_id IS NULL OR kind <> 'design-system');
CREATE INDEX docs_feature_scope_idx ON docs (workspace_id, project_id, feature_id, created_at, id);
