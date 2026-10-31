-- Stories. A story is a work item of a feature: what to build, the criteria
-- for it being done, the stories that block it and the files it changes. The
-- key a person reads is `US-<number>`, with the number counted per project.
-- Every write adds 1 to `version`, so a client that writes from a stale read
-- can be refused. `holder` is the run that works on the story, or null. Nothing
-- is deleted: a story that is dropped gets the status `cancelled`. The table
-- follows the checklist of migrations/README.md.

CREATE TABLE stories (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT stories_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    project_id uuid NOT NULL,
    feature_id uuid NOT NULL,
    number integer NOT NULL CONSTRAINT stories_number_positive CHECK (number >= 1),
    title text NOT NULL CONSTRAINT stories_title_not_blank CHECK (btrim(title) <> ''),
    text text NOT NULL,
    topic text NOT NULL,
    -- A JSON list of {text, ticked, human_check, pending}.
    criteria jsonb NOT NULL CONSTRAINT stories_criteria_is_list CHECK (jsonb_typeof(criteria) = 'array'),
    -- A JSON list of story ids.
    blocked_by jsonb NOT NULL DEFAULT '[]' CONSTRAINT stories_blocked_by_is_list CHECK (jsonb_typeof(blocked_by) = 'array'),
    -- JSON lists of paths.
    files_planned jsonb NOT NULL DEFAULT '[]' CONSTRAINT stories_files_planned_is_list CHECK (jsonb_typeof(files_planned) = 'array'),
    files_changed jsonb NOT NULL DEFAULT '[]' CONSTRAINT stories_files_changed_is_list CHECK (jsonb_typeof(files_changed) = 'array'),
    priority integer NOT NULL DEFAULT 3 CONSTRAINT stories_priority_range CHECK (priority BETWEEN 0 AND 3),
    status text NOT NULL DEFAULT 'backlog' CONSTRAINT stories_status_known
        CHECK (status IN ('backlog', 'implementing', 'review', 'testing', 'done', 'cancelled')),
    blocked boolean NOT NULL DEFAULT false,
    -- {run, stage, at} while a run holds the story, otherwise null.
    holder jsonb,
    findings jsonb NOT NULL DEFAULT '[]' CONSTRAINT stories_findings_is_list CHECK (jsonb_typeof(findings) = 'array'),
    decisions jsonb NOT NULL DEFAULT '[]' CONSTRAINT stories_decisions_is_list CHECK (jsonb_typeof(decisions) = 'array'),
    links jsonb NOT NULL DEFAULT '[]' CONSTRAINT stories_links_is_list CHECK (jsonb_typeof(links) = 'array'),
    version integer NOT NULL DEFAULT 1 CONSTRAINT stories_version_positive CHECK (version >= 1),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (workspace_id, id),
    UNIQUE (workspace_id, project_id, number),
    FOREIGN KEY (workspace_id, project_id) REFERENCES projects (workspace_id, id),
    FOREIGN KEY (workspace_id, project_id, feature_id) REFERENCES features (workspace_id, project_id, id)
);
CREATE INDEX stories_feature_idx ON stories (workspace_id, feature_id, priority, number);

ALTER TABLE stories ENABLE ROW LEVEL SECURITY;
ALTER TABLE stories FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON stories
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE ON stories TO omni_api;
