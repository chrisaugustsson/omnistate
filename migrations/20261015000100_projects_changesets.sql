-- US-016: projects and change sets. A project belongs to a workspace and holds
-- change sets; a change set has a status that moves along the lifecycle of
-- FR-20, and every move is kept in an append-only history. Every table follows
-- the checklist of migrations/README.md.

CREATE TABLE projects (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT projects_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    name text NOT NULL CONSTRAINT projects_name_not_blank CHECK (btrim(name) <> ''),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (workspace_id, id),
    -- A project is addressed by name from the omni CLI (.omni.toml), so a name
    -- is unique in its workspace.
    UNIQUE (workspace_id, name)
);

-- All FR-20 statuses are allowed, so US-025 and US-026 add transitions and no
-- new status. Nothing is deleted: an abandoned change set keeps its row.
CREATE TABLE changesets (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT changesets_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    project_id uuid NOT NULL,
    title text NOT NULL CONSTRAINT changesets_title_not_blank CHECK (btrim(title) <> ''),
    description text NOT NULL DEFAULT '',
    status text NOT NULL DEFAULT 'open' CONSTRAINT changesets_status_known CHECK (
        status IN ('open', 'in_review', 'approved', 'rejected', 'needs_rebase', 'merged', 'abandoned')
    ),
    run_tag text,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (workspace_id, id),
    FOREIGN KEY (workspace_id, project_id) REFERENCES projects (workspace_id, id)
);
CREATE INDEX changesets_project_idx ON changesets (workspace_id, project_id, created_at, id);

-- One row per status change, the first one (from_status NULL) being the
-- creation. Append-only: rows are inserted and never changed. created_at is the
-- time of the insert itself (clock_timestamp), not of the transaction start, so
-- the order of the rows is the order of the changes.
CREATE TABLE changeset_status_history (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT changeset_status_history_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    changeset_id uuid NOT NULL,
    from_status text,
    to_status text NOT NULL,
    actor text NOT NULL CONSTRAINT changeset_status_history_actor_known CHECK (actor IN ('agent', 'human')),
    reason text,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (workspace_id, id),
    FOREIGN KEY (workspace_id, changeset_id) REFERENCES changesets (workspace_id, id)
);
CREATE INDEX changeset_status_history_changeset_idx
    ON changeset_status_history (workspace_id, changeset_id, created_at, id);

-- The tenancy tests require UPDATE on every tenant table, so a history row is
-- protected from being changed by this trigger instead of by a missing
-- privilege. Deleting is still impossible for the API role (no DELETE
-- privilege); only the admin command that erases a workspace deletes.
CREATE FUNCTION reject_history_update() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
    BEGIN
        RAISE EXCEPTION 'the % table is append-only', TG_TABLE_NAME USING ERRCODE = '42501';
    END
    $$;

CREATE TRIGGER changeset_status_history_append_only
    BEFORE UPDATE ON changeset_status_history
    FOR EACH ROW EXECUTE FUNCTION reject_history_update();

ALTER TABLE projects ENABLE ROW LEVEL SECURITY;
ALTER TABLE projects FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON projects
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

ALTER TABLE changesets ENABLE ROW LEVEL SECURITY;
ALTER TABLE changesets FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON changesets
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

ALTER TABLE changeset_status_history ENABLE ROW LEVEL SECURITY;
ALTER TABLE changeset_status_history FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON changeset_status_history
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE ON projects, changesets, changeset_status_history TO omni_api;
