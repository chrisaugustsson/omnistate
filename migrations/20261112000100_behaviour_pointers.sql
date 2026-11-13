-- A node may point to another node of the same feature: `same_as_id` to the
-- node that already describes the same situation, `handoff_id` to the story
-- (or the node in one) that the user carries on in. The composite foreign keys
-- keep a pointer inside the feature; at most one pointer is set. A node with a
-- pointer has no children: the API checks that.

ALTER TABLE behaviour_nodes
    ADD COLUMN same_as_id uuid,
    ADD COLUMN handoff_id uuid,
    ADD CONSTRAINT behaviour_nodes_at_most_one_pointer CHECK (same_as_id IS NULL OR handoff_id IS NULL),
    ADD FOREIGN KEY (workspace_id, feature_id, same_as_id) REFERENCES behaviour_nodes (workspace_id, feature_id, id),
    ADD FOREIGN KEY (workspace_id, feature_id, handoff_id) REFERENCES behaviour_nodes (workspace_id, feature_id, id);
