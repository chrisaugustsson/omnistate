-- A member is a plain member or an admin. Admins may unstick a track whose
-- owner is away. Existing memberships keep the default; the setup command
-- makes the first member of a new workspace an admin.
ALTER TABLE memberships ADD COLUMN role text NOT NULL DEFAULT 'member'
    CONSTRAINT memberships_role_known CHECK (role IN ('member', 'admin'));
