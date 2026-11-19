-- The member who owns a track (D10): the creator, set once when the track is
-- created. Nothing but the hand-over (US-315) changes it. Null for a track made
-- by an API key that records no creator.
ALTER TABLE tracks ADD COLUMN owner_membership_id uuid;
ALTER TABLE tracks ADD FOREIGN KEY (workspace_id, owner_membership_id)
    REFERENCES memberships (workspace_id, id);
CREATE INDEX tracks_owner_idx ON tracks (workspace_id, owner_membership_id);
