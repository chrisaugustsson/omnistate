-- US-110: agents delete and rename docs and features. A delete is soft: the API
-- role has no DELETE privilege, so a doc, a feature or a thread gets
-- `deleted_at` and every list and read leaves it out. Deleting a feature sets it
-- on the feature's docs and threads too. A deleted feature does not hold its
-- name, and a deleted design system doc does not hold the project's one place
-- for it. Features have no status any more (V7): the column goes with its
-- check constraint. The uniqueness of the name is kept over the features that
-- are not deleted. Nothing else about the tables changes.
ALTER TABLE features DROP COLUMN status;
ALTER TABLE features ADD COLUMN deleted_at timestamptz;
DROP INDEX features_name_idx;
CREATE UNIQUE INDEX features_name_idx ON features (workspace_id, project_id, lower(name))
    WHERE deleted_at IS NULL;

ALTER TABLE docs ADD COLUMN deleted_at timestamptz;
DROP INDEX docs_one_design_system_per_project;
CREATE UNIQUE INDEX docs_one_design_system_per_project
    ON docs (workspace_id, project_id)
    WHERE kind = 'design-system' AND deleted_at IS NULL;

ALTER TABLE threads ADD COLUMN deleted_at timestamptz;
