# Migrations

The files in this folder are sqlx migrations, named `<version>_<description>.sql`. The API applies them in order when it starts. Never edit a migration that has been committed: add a new one.

Tenant isolation is enforced by the database, not by the code that queries it. Every new table follows this checklist; tick every item before the migration is committed. A table with a `workspace_id` column is a tenant table, and the isolation tests in `crates/omni-core/tests/tenancy.rs` check each of them for the rules below.

## Checklist for every new table

- [ ] The table has an `id uuid` primary key that defaults to `uuid_v7()` and has a `CHECK (is_uuid_v7(id))`, so every record ID is a UUIDv7.
- [ ] A tenant table has a `workspace_id uuid NOT NULL` column that references `workspaces (id)`.
- [ ] A tenant table has a unique key on `(workspace_id, id)`, so that other tables can reference it with the workspace.
- [ ] Every foreign key to another tenant table is a composite foreign key that includes `workspace_id`, for example `FOREIGN KEY (workspace_id, project_id) REFERENCES projects (workspace_id, id)`. The database then rejects a row that references another workspace's row.
- [ ] Row-level security is enabled and forced: `ALTER TABLE ... ENABLE ROW LEVEL SECURITY` and `ALTER TABLE ... FORCE ROW LEVEL SECURITY`.
- [ ] One policy `FOR ALL` has `USING` and `WITH CHECK` set to `workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid` (`id = ...` for `workspaces`).
- [ ] The API role `omni_api` gets `GRANT SELECT, INSERT, UPDATE` on the table and nothing else: no DELETE privilege and no TRUNCATE privilege on tenant tables (nothing inside a project is deleted).
- [ ] No UPDATE privilege on append-only tables (posts, operations, versions, node indexes): grant `SELECT, INSERT` only. The tenancy tests still require `UPDATE` on every table with a `workspace_id`, so until they tell append-only tables apart, such a table is protected by a `BEFORE UPDATE` trigger that raises an error (see `changeset_status_history`).
- [ ] Anything that must be looked up before the workspace is known goes through a SECURITY DEFINER function with a pinned `search_path` whose EXECUTE is revoked from PUBLIC and granted to `omni_api`; the API reads no tenant table without a workspace. The only two are `resolve_api_key(prefix)` and `user_memberships(user_id)`.
- [ ] The API role owns no table, so it cannot disable row-level security, change a policy or bypass it.

- [ ] Seed one row of the table in `fill_workspace()` in `crates/omni-api/tests/workspace_delete.rs`. The deletion tests check that the seeded workspace has a row in every table with a `workspace_id` column, so they fail until the new table is seeded.

## Notes

- The migrations run as the database owner (the `omni` superuser of the Compose stack), which is not subject to row-level security; that is what lets the SECURITY DEFINER functions see across workspaces. The API connects as `omni_api` for everything else. The API sets that role's password when it starts, so no password is in a migration.
- The API runs `SET LOCAL app.workspace_id = '<workspace id>'` at the start of each transaction (`set_config('app.workspace_id', ..., true)`), never on the connection. When the transaction ends, the setting is empty again and the policy matches no rows.
