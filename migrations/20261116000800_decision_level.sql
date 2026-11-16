-- A decision is about the level where it is true (D24): `track` shows only on
-- its track, `feature` and `part` also on that feature or part, `project` on
-- the project's own list. `part_id` sits beside `feature_id`; at most one of
-- the two is set, and `about` says which. The composite foreign key ties a
-- decision to a part of its own project.
ALTER TABLE decisions ADD COLUMN part_id uuid;
ALTER TABLE decisions ADD COLUMN about text NOT NULL DEFAULT 'track';
UPDATE decisions SET about = 'feature' WHERE feature_id IS NOT NULL;
ALTER TABLE decisions
    ADD FOREIGN KEY (workspace_id, project_id, part_id)
    REFERENCES parts (workspace_id, project_id, id);
ALTER TABLE decisions
    ADD CONSTRAINT decisions_about_is_a_level CHECK (about IN ('track', 'feature', 'part', 'project'));
ALTER TABLE decisions
    ADD CONSTRAINT decisions_one_scope CHECK (num_nonnulls(feature_id, part_id) <= 1);
ALTER TABLE decisions
    ADD CONSTRAINT decisions_about_matches_scope CHECK (
        (about = 'feature') = (feature_id IS NOT NULL) AND (about = 'part') = (part_id IS NOT NULL)
    );
CREATE INDEX decisions_part_idx ON decisions (workspace_id, part_id, created_at, id)
    WHERE part_id IS NOT NULL;
CREATE INDEX decisions_project_level_idx ON decisions (workspace_id, project_id, created_at, id)
    WHERE about = 'project';
