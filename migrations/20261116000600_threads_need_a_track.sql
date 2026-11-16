-- D21: threads live only on tracks. The work tables are empty on the new
-- stack (D1), so no thread without a track needs a home.
ALTER TABLE threads ALTER COLUMN track_id SET NOT NULL;
