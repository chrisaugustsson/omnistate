-- US-107: a comment keeps a snapshot of the node it points at, as the node was
-- when the comment was made, so that a comment whose node an agent later
-- removes (an orphan, V18) still shows what it was about. Whether a comment is
-- orphaned is computed against the doc's head when it is read, so nothing else
-- is stored. Comments made before this migration have no snapshot.
ALTER TABLE comments ADD COLUMN snapshot text;
