-- Datadog-compatible management API for users, roles, teams and keys (docs/adr/0003-datadog-compatible-management-api.md,
-- src/routes/datadog_api/{users,roles,teams,keys}.rs). The compatible routes work on our own tables; this adds what
-- Datadog's tools set and read back that our model doesn't hold.

-- The Datadog permissions (by name, see datadog_api/permissions.rs) a custom role was given through the compatible API
-- (`datadog_role` `permission` blocks), returned as set. Access still comes from the built-in roles.
CREATE TABLE role_permissions (
    org_id     TEXT NOT NULL,
    role_id    TEXT NOT NULL,
    permission TEXT NOT NULL,
    PRIMARY KEY (role_id, permission),
    FOREIGN KEY (org_id, role_id) REFERENCES roles(org_id, id) ON DELETE CASCADE
);

-- A team's stable id (Datadog's team id), independent of its handle.
ALTER TABLE teams ADD COLUMN id UUID NOT NULL DEFAULT gen_random_uuid();
CREATE UNIQUE INDEX teams_id ON teams (id);
