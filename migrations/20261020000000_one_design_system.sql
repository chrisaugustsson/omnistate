-- US-028: a project has at most one design system doc (FR-26). The API checks
-- this before it inserts a doc; the index settles two requests that start a
-- design system in the same project at once. `design` docs are not limited.
CREATE UNIQUE INDEX docs_one_design_system_per_project
    ON docs (workspace_id, project_id)
    WHERE kind = 'design-system';
