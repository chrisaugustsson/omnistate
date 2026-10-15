-- US-011: the tenancy schema. Tenant isolation is enforced by Postgres itself:
-- every tenant row carries its workspace, row-level security is forced on every
-- tenant table, and the API role cannot get around either. migrations/README.md
-- is the checklist that every later table follows.

-- The API's database role. Roles are cluster-wide, so it may exist already (the
-- tests apply these migrations to a scratch database of the same cluster). It
-- has no password here: the API sets it when it starts.
DO $$
BEGIN
    IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'omni_api') THEN
        CREATE ROLE omni_api LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
    END IF;
END
$$;

-- Record IDs are UUIDv7: 48 bits of Unix time in milliseconds, then random
-- bits. Postgres 17 has no uuidv7(), so this builds one from a random UUIDv4 by
-- overwriting its first 6 bytes with the time and its version nibble with 7.
CREATE FUNCTION uuid_v7() RETURNS uuid
    LANGUAGE sql VOLATILE PARALLEL SAFE
    AS $$
        SELECT encode(
            set_bit(set_bit(
                overlay(
                    uuid_send(gen_random_uuid())
                    PLACING substring(int8send((extract(epoch FROM clock_timestamp()) * 1000)::bigint) FROM 3)
                    FROM 1 FOR 6),
                52, 1), 53, 1),
            'hex')::uuid
    $$;

-- Every table's id has a CHECK on this, so a record can never carry another
-- kind of UUID, whoever inserts it.
CREATE FUNCTION is_uuid_v7(id uuid) RETURNS boolean
    LANGUAGE sql IMMUTABLE STRICT PARALLEL SAFE
    AS $$
        SELECT substr(id::text, 15, 1) = '7' AND substr(id::text, 20, 1) IN ('8', '9', 'a', 'b')
    $$;

-- Global tables: not scoped to a workspace.

CREATE TABLE users (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT users_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    email text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX users_email_key ON users (lower(email));

-- How a user proves who they are; the AuthProvider implementations read these.
-- secret_hash holds an Argon2id hash for the password provider.
CREATE TABLE login_identities (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT login_identities_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    user_id uuid NOT NULL REFERENCES users (id),
    provider text NOT NULL DEFAULT 'password',
    subject text NOT NULL,
    secret_hash text,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (provider, subject)
);

-- Server-side browser sessions; token_hash is the SHA-256 of the session token.
CREATE TABLE sessions (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT sessions_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    user_id uuid NOT NULL REFERENCES users (id),
    token_hash bytea NOT NULL UNIQUE,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    last_seen_at timestamptz,
    revoked_at timestamptz
);

-- Tenant tables.

CREATE TABLE workspaces (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT workspaces_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    name text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE memberships (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT memberships_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    user_id uuid NOT NULL REFERENCES users (id),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (workspace_id, id),
    UNIQUE (workspace_id, user_id)
);

-- An API key is always one agent of one workspace. prefix is the lookup handle
-- and is unique across workspaces, because resolve_api_key() finds a key by it
-- before the workspace is known; key_hash is the SHA-256 of the whole key.
CREATE TABLE api_keys (
    id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT api_keys_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
    workspace_id uuid NOT NULL REFERENCES workspaces (id),
    prefix text NOT NULL UNIQUE,
    key_hash text NOT NULL,
    scopes text[] NOT NULL DEFAULT '{}',
    created_by_membership_id uuid,
    created_at timestamptz NOT NULL DEFAULT now(),
    last_used_at timestamptz,
    revoked_at timestamptz,
    UNIQUE (workspace_id, id),
    FOREIGN KEY (workspace_id, created_by_membership_id) REFERENCES memberships (workspace_id, id)
);

-- Row-level security, forced so that it applies to the table owner too. The
-- policy is the same on every tenant table apart from the column: a row is
-- visible and writable only while app.workspace_id (which the API sets for each
-- transaction with SET LOCAL) names its workspace. NULLIF turns the empty string
-- that a pooled connection reports after a transaction into NULL, so no rows
-- match instead of the cast failing.

ALTER TABLE workspaces ENABLE ROW LEVEL SECURITY;
ALTER TABLE workspaces FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON workspaces
    USING (id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

ALTER TABLE memberships ENABLE ROW LEVEL SECURITY;
ALTER TABLE memberships FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON memberships
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

ALTER TABLE api_keys ENABLE ROW LEVEL SECURITY;
ALTER TABLE api_keys FORCE ROW LEVEL SECURITY;
CREATE POLICY workspace_isolation ON api_keys
    USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
    WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);

-- The only lookups that run before the workspace is known. They are owned by the
-- migrating role, which is not subject to the policies (a superuser), and pin
-- their search_path so that nothing can shadow the objects they use.

CREATE FUNCTION resolve_api_key(key_prefix text) RETURNS SETOF api_keys
    LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public
    AS $$
        SELECT * FROM api_keys WHERE prefix = key_prefix
    $$;

CREATE FUNCTION user_memberships(member_user_id uuid) RETURNS SETOF memberships
    LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public
    AS $$
        SELECT * FROM memberships WHERE user_id = member_user_id
    $$;

REVOKE ALL ON FUNCTION resolve_api_key(text) FROM PUBLIC;
REVOKE ALL ON FUNCTION user_memberships(uuid) FROM PUBLIC;

-- What the API role may do. It owns nothing, has no DELETE or TRUNCATE on any
-- table (records are never deleted through the API), and no privilege to change
-- the tables, their policies or row-level security.
GRANT USAGE ON SCHEMA public TO omni_api;
GRANT EXECUTE ON FUNCTION uuid_v7(), is_uuid_v7(uuid) TO omni_api;
GRANT EXECUTE ON FUNCTION resolve_api_key(text), user_memberships(uuid) TO omni_api;
GRANT SELECT, INSERT, UPDATE ON users, login_identities, sessions, workspaces, memberships, api_keys TO omni_api;
