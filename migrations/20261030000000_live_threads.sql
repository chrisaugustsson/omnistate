-- US-108: threads without change sets. A thread now belongs to the app level of
-- a project or to one of its features, and it may be concluded. Until the
-- change sets go (US-115), a thread that was started in one keeps its
-- `changeset_id`, so the column may be null. `feature_id` used to be read from
-- the thread's change set; it is stored on the thread now, and the threads that
-- exist get the feature of their change set. A post no longer has a type; the
-- column stays for the posts of change-set threads. Nothing is deleted.
ALTER TABLE threads
    ALTER COLUMN changeset_id DROP NOT NULL,
    ADD COLUMN feature_id uuid,
    -- The text an agent wrote when the thread was worked out; null until then.
    ADD COLUMN conclusion text CONSTRAINT threads_conclusion_not_blank CHECK (btrim(conclusion) <> ''),
    ADD CONSTRAINT threads_feature_fk
        FOREIGN KEY (workspace_id, project_id, feature_id) REFERENCES features (workspace_id, project_id, id);

UPDATE threads
SET feature_id = changesets.feature_id
FROM changesets
WHERE changesets.workspace_id = threads.workspace_id AND changesets.id = threads.changeset_id;

CREATE INDEX threads_feature_idx ON threads (workspace_id, feature_id, created_at, id);

ALTER TABLE posts ALTER COLUMN type DROP NOT NULL;
