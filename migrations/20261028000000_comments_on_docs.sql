-- US-106: a comment belongs to a doc and a node address of its head (V17), not to
-- a saved version, and has no status: the human points and comments, an agent
-- replies, and the human deletes the comment when satisfied. A deletion is soft
-- (`deleted_at`), since the API role has no DELETE privilege; the row and its
-- text stay. A comment whose node later leaves the head stays listed (V18).
--
-- Comments written before this migration take the doc of their version.
-- `version_id` stays, nullable, for the comments made through the transitional
-- route POST .../versions/{version_id}/comments that the web app still uses
-- until it moves to the live routes (US-113); the live route leaves it null.
-- The status and the version that addressed a comment are dropped (V23).
ALTER TABLE comments ADD COLUMN doc_id uuid;
UPDATE comments c SET doc_id = v.doc_id
    FROM versions v WHERE v.workspace_id = c.workspace_id AND v.id = c.version_id;
ALTER TABLE comments ALTER COLUMN doc_id SET NOT NULL;
ALTER TABLE comments ADD FOREIGN KEY (workspace_id, doc_id) REFERENCES docs (workspace_id, id);
ALTER TABLE comments ALTER COLUMN version_id DROP NOT NULL;
ALTER TABLE comments DROP COLUMN status;
ALTER TABLE comments DROP COLUMN addressed_in_version_id;
ALTER TABLE comments ADD COLUMN deleted_at timestamptz;
CREATE INDEX comments_doc_idx ON comments (workspace_id, doc_id, created_at, id);
