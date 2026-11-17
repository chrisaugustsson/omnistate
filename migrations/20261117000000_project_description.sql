-- A project's intro text, shown on its Overview (decision U8). Nullable: a
-- project without one shows an "Add a description" link. Any member or agent
-- sets it through PATCH /projects/{id}.
ALTER TABLE projects ADD COLUMN description text;
