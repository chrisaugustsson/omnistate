-- A doc may belong to a part, an app or a service, as well as to a feature, a
-- track or the project: what is true for a whole part has a home of its own.
-- `part_id` sits beside `feature_id` and `track_id`; at most one of the three
-- is set, and with all null the doc is at the project level. The composite
-- foreign key ties a doc to a part of its own project. A design system is
-- always global, so it takes none of them.
ALTER TABLE docs ADD COLUMN part_id uuid;
ALTER TABLE docs
    ADD FOREIGN KEY (workspace_id, project_id, part_id)
    REFERENCES parts (workspace_id, project_id, id);
ALTER TABLE docs DROP CONSTRAINT docs_one_scope;
ALTER TABLE docs
    ADD CONSTRAINT docs_one_scope CHECK (num_nonnulls(part_id, feature_id, track_id) <= 1);
ALTER TABLE docs
    ADD CONSTRAINT docs_design_system_is_global_of_part CHECK (part_id IS NULL OR kind <> 'design-system');
CREATE INDEX docs_part_scope_idx ON docs (workspace_id, project_id, part_id, created_at, id);
