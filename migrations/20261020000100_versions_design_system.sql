-- US-029: a version of a design doc records the version of the design system
-- that its instances resolve against (FR-31), so that every view of the
-- version, every comparison and every export shows the components as they
-- were. The column is NULL for the versions of other docs, and for a design
-- version saved before the project had a design system version. A version is
-- append-only, so the column is written once, with the version.
ALTER TABLE versions ADD COLUMN design_system_version_id uuid;
ALTER TABLE versions ADD FOREIGN KEY (workspace_id, design_system_version_id)
    REFERENCES versions (workspace_id, id);
