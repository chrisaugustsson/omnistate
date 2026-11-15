-- A track belongs to one part of its project. `tracks.part_id` is NOT NULL and
-- the composite key `(workspace_id, project_id, part_id)` ties it to a part of
-- the track's own project and workspace. The work tables are empty when this
-- runs (nothing is migrated), so the column needs no default.
ALTER TABLE tracks ADD COLUMN part_id uuid NOT NULL;
ALTER TABLE tracks ADD CONSTRAINT tracks_part_fk
    FOREIGN KEY (workspace_id, project_id, part_id) REFERENCES parts (workspace_id, project_id, id);
CREATE INDEX tracks_part_idx ON tracks (workspace_id, project_id, part_id);
