-- US-061: a doc belongs to the project (a null `feature_id`, which is what every
-- existing doc stays) or to one of its features (FR-53). It takes the scope of
-- the change set that creates it and never moves. The composite foreign key ties
-- a doc to a feature of its own project; with a null `feature_id` it is not
-- checked. The design system is always global (FR-54).
ALTER TABLE docs ADD COLUMN feature_id uuid;
ALTER TABLE docs
    ADD FOREIGN KEY (workspace_id, project_id, feature_id)
    REFERENCES features (workspace_id, project_id, id);
ALTER TABLE docs
    ADD CONSTRAINT docs_design_system_is_global CHECK (feature_id IS NULL OR kind <> 'design-system');
CREATE INDEX docs_feature_idx ON docs (workspace_id, project_id, feature_id, created_at, id);
