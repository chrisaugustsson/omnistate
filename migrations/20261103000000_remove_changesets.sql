-- US-115: the change-set machinery goes (V3, V4, V23). Docs, tokens, components
-- and threads are written straight to their heads and logged in `doc_changes`,
-- so change sets, drafts, versions, approvals, rebase conflicts, decisions and
-- directives have nothing left to do. Local data may be reset (V23): nothing is
-- migrated. No table is added, so the checklist of migrations/README.md does
-- not apply.
--
-- A column on a table that stays is dropped with the table it pointed to:
-- `docs.mainline_version_id`, `comments.version_id`, `threads.changeset_id` and
-- the type of a post, which only the posts of change-set threads had. The
-- threads that were started in a change set are left in place without it.

ALTER TABLE docs DROP COLUMN mainline_version_id;
ALTER TABLE comments DROP COLUMN version_id;
ALTER TABLE threads DROP COLUMN changeset_id;
ALTER TABLE posts DROP COLUMN type;

DROP TABLE directive_receipts;
DROP TABLE changeset_decisions;
DROP TABLE decisions;
DROP TABLE conflicts;
DROP TABLE approvals;
DROP TABLE draft_ops;
DROP TABLE versions;
DROP TABLE drafts;
DROP TABLE changeset_status_history;
DROP TABLE changesets;
