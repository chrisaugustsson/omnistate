-- US-060: a change set belongs to the project or to one of its features (FR-53),
-- fixed at creation. A null `feature_id` is a project-level change set, which is
-- what every existing change set stays. The composite foreign key ties a change
-- set to a feature of its own project; with a null `feature_id` it is not checked.
ALTER TABLE changesets ADD COLUMN feature_id uuid;
ALTER TABLE changesets
    ADD FOREIGN KEY (workspace_id, project_id, feature_id)
    REFERENCES features (workspace_id, project_id, id);
CREATE INDEX changesets_feature_idx ON changesets (workspace_id, project_id, feature_id, created_at, id);
