-- Log data access (Logs → Configuration → Data Access): restriction queries are log searches
-- assigned to roles and teams. While an org has none, everyone reads all of its logs. Once it has
-- one, org admins still read everything and every other user reads only the logs matching a query
-- assigned to one of their roles or teams (none assigned: no logs). See crate::log_access.

CREATE TABLE log_restriction_queries (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    -- Log search syntax (`service:billing env:prod`); never empty.
    query       TEXT NOT NULL CHECK (query <> ''),
    created_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (org_id, name),
    -- Target of the assignments' (org, query) keys.
    UNIQUE (org_id, id)
);

-- Role ids are unique on their own; the pair lets assignments reference a role of the same org only.
ALTER TABLE roles ADD CONSTRAINT roles_org_id_id_key UNIQUE (org_id, id);

CREATE TABLE log_restriction_roles (
    org_id         TEXT NOT NULL,
    restriction_id UUID NOT NULL,
    role_id        TEXT NOT NULL,
    PRIMARY KEY (restriction_id, role_id),
    FOREIGN KEY (org_id, restriction_id) REFERENCES log_restriction_queries(org_id, id) ON DELETE CASCADE,
    FOREIGN KEY (org_id, role_id) REFERENCES roles(org_id, id) ON DELETE CASCADE
);

CREATE TABLE log_restriction_teams (
    org_id         TEXT NOT NULL,
    restriction_id UUID NOT NULL,
    team_handle    TEXT NOT NULL,
    PRIMARY KEY (restriction_id, team_handle),
    FOREIGN KEY (org_id, restriction_id) REFERENCES log_restriction_queries(org_id, id) ON DELETE CASCADE,
    FOREIGN KEY (org_id, team_handle) REFERENCES teams(org_id, handle) ON DELETE CASCADE ON UPDATE CASCADE
);

CREATE INDEX log_restriction_roles_org ON log_restriction_roles (org_id);
CREATE INDEX log_restriction_teams_org ON log_restriction_teams (org_id);
