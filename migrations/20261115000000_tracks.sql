-- Tracks. What the schema called a feature is the unit of work, and the name
-- "feature" is wanted for a capability of a part. The table `features` becomes
-- `tracks` and the column `feature_id` becomes `track_id` on every table that
-- has it. Constraints and indexes that carry the old name are renamed with
-- them, and a behaviour rule's scope `feature` becomes `track`. Nothing about
-- the shape of the tables changes: the checklist of migrations/README.md was
-- ticked for `features` and holds for `tracks` as it stands.
ALTER TABLE features RENAME TO tracks;
ALTER TABLE docs RENAME COLUMN feature_id TO track_id;
ALTER TABLE threads RENAME COLUMN feature_id TO track_id;
ALTER TABLE stories RENAME COLUMN feature_id TO track_id;
ALTER TABLE behaviour_nodes RENAME COLUMN feature_id TO track_id;
ALTER TABLE behaviour_questions RENAME COLUMN feature_id TO track_id;
ALTER TABLE behaviour_answers RENAME COLUMN feature_id TO track_id;
ALTER TABLE behaviour_rules RENAME COLUMN feature_id TO track_id;

ALTER TABLE behaviour_rules DROP CONSTRAINT behaviour_rules_scope_known;
UPDATE behaviour_rules SET scope = 'track' WHERE scope = 'feature';
ALTER TABLE behaviour_rules ADD CONSTRAINT behaviour_rules_scope_known
    CHECK (scope IN ('node', 'story', 'track'));

DO $$
DECLARE
    item record;
BEGIN
    FOR item IN
        SELECT conrelid::regclass::text AS table_name, conname
        FROM pg_constraint
        WHERE connamespace = 'public'::regnamespace AND conname LIKE '%feature%'
    LOOP
        EXECUTE format('ALTER TABLE %s RENAME CONSTRAINT %I TO %I',
            item.table_name, item.conname, replace(item.conname, 'feature', 'track'));
    END LOOP;
    FOR item IN
        SELECT indexname FROM pg_indexes
        WHERE schemaname = 'public' AND indexname LIKE '%feature%'
    LOOP
        EXECUTE format('ALTER INDEX %I RENAME TO %I',
            item.indexname, replace(item.indexname, 'feature', 'track'));
    END LOOP;
END $$;
