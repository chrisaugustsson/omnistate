-- US-112: a scope's last activity counts a thread's conclusion as a write, so
-- the thread says when it was concluded. A thread that is concluded already
-- gets the time it was started, the closest time there is.
ALTER TABLE threads ADD COLUMN concluded_at timestamptz;

UPDATE threads SET concluded_at = created_at WHERE conclusion IS NOT NULL;
