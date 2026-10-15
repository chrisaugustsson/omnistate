-- US-019: attachments. The metadata of an uploaded file; the bytes live in
-- RustFS at ws/{workspace_id}/attachments/{id}. The table follows the checklist
-- of migrations/README.md.
CREATE TABLE attachments (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT attachments_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    -- The name the agent gave the file, without control characters.
    filename text NOT NULL CONSTRAINT attachments_filename_not_blank CHECK (btrim(filename) <> ''),
    -- Decided from the file's bytes, never from what the sender claimed:
    -- image/png, image/jpeg, image/gif, image/webp, font/woff2, font/ttf,
    -- font/otf, or application/octet-stream for any other file.
    content_type text NOT NULL,
    -- At most 20 MiB, the limit of the upload route.
    size_bytes bigint NOT NULL CONSTRAINT attachments_size_in_range CHECK (size_bytes BETWEEN 0 AND 20971520),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (workspace_id, id)
);

ALTER TABLE attachments ENABLE ROW LEVEL SECURITY;
ALTER TABLE attachments FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON attachments
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE ON attachments TO omni_api;
