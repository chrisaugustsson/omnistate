-- A decision written by concluding a thread names that thread, so a second
-- conclusion of the same thread can replace the one before it (US-323).
ALTER TABLE decisions ADD COLUMN thread_id uuid;
ALTER TABLE decisions ADD FOREIGN KEY (workspace_id, thread_id) REFERENCES threads (workspace_id, id);
CREATE INDEX decisions_thread_idx ON decisions (workspace_id, thread_id, created_at, id)
    WHERE thread_id IS NOT NULL;
