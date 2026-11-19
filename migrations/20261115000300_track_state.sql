-- A track has a state: it starts in planning, moves to building, and ends done
-- or dropped. The moves between them are the API's to check; the column only
-- keeps the value to one of the four. The work tables are empty when this
-- runs, and the default covers any row there is.
ALTER TABLE tracks ADD COLUMN state text NOT NULL DEFAULT 'planning';
ALTER TABLE tracks ADD CONSTRAINT tracks_state_known
    CHECK (state IN ('planning', 'building', 'done', 'dropped'));
