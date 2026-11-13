-- A node the next grill run should revisit has `needs_pass_since` set. The
-- API sets it inside the write that triggers it and clears it on request of an
-- agent; setting it changes nothing else, not even `version`.

ALTER TABLE behaviour_nodes ADD COLUMN needs_pass_since timestamptz;
