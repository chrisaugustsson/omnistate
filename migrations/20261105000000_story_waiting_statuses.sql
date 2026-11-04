-- US-160: a story can wait between phases, in `implemented` (waiting for
-- review) or `reviewed` (waiting for the test lane). No table is added, so the
-- checklist of migrations/README.md does not apply.
ALTER TABLE stories DROP CONSTRAINT stories_status_known;
ALTER TABLE stories ADD CONSTRAINT stories_status_known
    CHECK (status IN ('backlog', 'implementing', 'implemented', 'review', 'reviewed', 'testing', 'done', 'cancelled'));
