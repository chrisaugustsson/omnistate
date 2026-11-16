-- The track a track was split off from (D4, D19): set when the track is
-- created, never changed. Both tracks show the link.
ALTER TABLE tracks ADD COLUMN split_from_track_id uuid;
ALTER TABLE tracks ADD FOREIGN KEY (workspace_id, split_from_track_id)
    REFERENCES tracks (workspace_id, id);
CREATE INDEX tracks_split_from_idx ON tracks (workspace_id, split_from_track_id);
