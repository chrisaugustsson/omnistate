-- An API key belongs to a user and reaches every workspace that user is a
-- member of, as the memberships stand at each request: leaving a workspace
-- takes the key out of it at once. Until now a key was one agent of one
-- workspace. It becomes a global table, like sessions: no workspace_id and no
-- row-level security. The API finds a key by its prefix (resolve_api_key) and
-- checks the path's workspace against user_memberships() of the key's user.

ALTER TABLE api_keys ADD COLUMN user_id uuid REFERENCES users (id);

-- A key made through the API names the member who created it. It now belongs
-- to that member's user, so it keeps working in the workspace it was made in,
-- and reaches the user's other workspaces too.
UPDATE api_keys k SET user_id = m.user_id
    FROM memberships m
    WHERE m.workspace_id = k.workspace_id AND m.id = k.created_by_membership_id;

-- A key without a creator was never made through the API (only inserted by
-- hand), so there is no user it could belong to. It is removed.
DO $$
DECLARE
    orphans bigint;
BEGIN
    DELETE FROM api_keys WHERE user_id IS NULL;
    GET DIAGNOSTICS orphans = ROW_COUNT;
    IF orphans > 0 THEN
        RAISE NOTICE 'removed % API key(s) that no member created', orphans;
    END IF;
END
$$;

ALTER TABLE api_keys ALTER COLUMN user_id SET NOT NULL;

-- No longer a tenant table. Dropping the columns also drops the composite
-- foreign key to memberships and the unique key on (workspace_id, id); no
-- table references either.
DROP POLICY workspace_isolation ON api_keys;
ALTER TABLE api_keys NO FORCE ROW LEVEL SECURITY;
ALTER TABLE api_keys DISABLE ROW LEVEL SECURITY;
ALTER TABLE api_keys DROP COLUMN created_by_membership_id;
ALTER TABLE api_keys DROP COLUMN workspace_id;

-- The account page lists a user's keys, oldest first.
CREATE INDEX api_keys_user_id_created_at ON api_keys (user_id, created_at, id);

-- The same lookup by prefix, recreated so its body is checked against the
-- table as it is now. The grants of the function are kept.
CREATE OR REPLACE FUNCTION resolve_api_key(key_prefix text) RETURNS SETOF api_keys
    LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public
    AS $$
        SELECT * FROM api_keys WHERE prefix = key_prefix
    $$;
