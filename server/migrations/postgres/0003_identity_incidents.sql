-- Identity (users, roles, teams, SAML group mappings) and incident management.

-- ---------------------------------------------------------------- users
ALTER TABLE users
    ADD COLUMN title          TEXT,
    ADD COLUMN status         TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'pending', 'deactivated')),
    ADD COLUMN login_methods  TEXT[] NOT NULL DEFAULT '{password}',
    ADD COLUMN mfa_enabled    BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN timezone       TEXT,
    ADD COLUMN last_login_at  TIMESTAMPTZ;
CREATE INDEX users_org_name ON users (org_id, lower(name));

CREATE TABLE roles (
    id          TEXT PRIMARY KEY,
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    -- Built-in roles can't be renamed or deleted.
    managed     BOOLEAN NOT NULL DEFAULT false,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (org_id, name)
);

CREATE TABLE user_roles (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role_id TEXT NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, role_id)
);

-- ---------------------------------------------------------------- teams
ALTER TABLE teams
    ADD COLUMN description   TEXT NOT NULL DEFAULT '',
    ADD COLUMN created_via   TEXT NOT NULL DEFAULT 'ui' CHECK (created_via IN ('ui', 'api', 'saml', 'terraform')),
    -- Only team admins can add members.
    ADD COLUMN restricted    BOOLEAN NOT NULL DEFAULT true,
    ADD COLUMN notifications TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN parent_handle TEXT,
    ADD COLUMN created_at    TIMESTAMPTZ NOT NULL DEFAULT now();

CREATE TABLE team_members (
    org_id  TEXT NOT NULL,
    handle  TEXT NOT NULL,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role    TEXT NOT NULL DEFAULT 'member' CHECK (role IN ('admin', 'member')),
    PRIMARY KEY (org_id, handle, user_id),
    FOREIGN KEY (org_id, handle) REFERENCES teams(org_id, handle) ON DELETE CASCADE
);

-- IdP group claim → team membership or role.
CREATE TABLE saml_mappings (
    id              BIGSERIAL PRIMARY KEY,
    org_id          TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    kind            TEXT NOT NULL CHECK (kind IN ('team', 'role')),
    target          TEXT NOT NULL,
    attribute_key   TEXT NOT NULL,
    attribute_value TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX saml_mappings_org ON saml_mappings (org_id, kind);

-- ---------------------------------------------------------------- incidents
CREATE TABLE incidents (
    id               BIGSERIAL PRIMARY KEY,
    org_id           TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    -- The IR-<n> number users see, per org.
    public_id        INTEGER NOT NULL,
    title            TEXT NOT NULL,
    status           TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'stable', 'resolved')),
    severity         TEXT NOT NULL DEFAULT 'UNKNOWN' CHECK (severity IN ('SEV-1', 'SEV-2', 'SEV-3', 'SEV-4', 'SEV-5', 'UNKNOWN')),
    incident_type    TEXT NOT NULL DEFAULT 'Production Incident',
    detection_method TEXT NOT NULL DEFAULT 'unknown' CHECK (detection_method IN ('monitor', 'customer', 'employee', 'automation', 'unknown')),
    commander_id     UUID REFERENCES users(id) ON DELETE SET NULL,
    declared_by      TEXT NOT NULL,
    customer_impact  BOOLEAN NOT NULL DEFAULT false,
    summary          TEXT NOT NULL DEFAULT '',
    root_cause       TEXT NOT NULL DEFAULT '',
    services         TEXT[] NOT NULL DEFAULT '{}',
    teams            TEXT[] NOT NULL DEFAULT '{}',
    test             BOOLEAN NOT NULL DEFAULT false,
    detected_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    declared_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    acknowledged_at  TIMESTAMPTZ,
    stable_at        TIMESTAMPTZ,
    resolved_at      TIMESTAMPTZ,
    modified_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (org_id, public_id)
);
CREATE INDEX incidents_org_declared ON incidents (org_id, declared_at DESC);

CREATE TABLE incident_responders (
    incident_id BIGINT NOT NULL REFERENCES incidents(id) ON DELETE CASCADE,
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role        TEXT NOT NULL DEFAULT 'responder',
    PRIMARY KEY (incident_id, user_id)
);

-- Timeline: notes and automatic entries (status/severity/commander changes, notifications).
CREATE TABLE incident_events (
    id          BIGSERIAL PRIMARY KEY,
    incident_id BIGINT NOT NULL REFERENCES incidents(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL CHECK (kind IN ('declared', 'note', 'status', 'severity', 'commander', 'notification', 'impact', 'followup')),
    author      TEXT NOT NULL,
    body        TEXT NOT NULL DEFAULT '',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX incident_events_incident ON incident_events (incident_id, created_at);

CREATE TABLE incident_impacts (
    id          BIGSERIAL PRIMARY KEY,
    incident_id BIGINT NOT NULL REFERENCES incidents(id) ON DELETE CASCADE,
    description TEXT NOT NULL,
    start_at    TIMESTAMPTZ NOT NULL,
    end_at      TIMESTAMPTZ
);

CREATE TABLE incident_followups (
    id          BIGSERIAL PRIMARY KEY,
    incident_id BIGINT NOT NULL REFERENCES incidents(id) ON DELETE CASCADE,
    title       TEXT NOT NULL,
    done        BOOLEAN NOT NULL DEFAULT false,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Built-in roles for the default org; the development user is an admin.
INSERT INTO roles (id, org_id, name, description, managed) VALUES
    ('default-admin', 'default', 'ObserveCraft Admin Role', 'Full access, including organization settings, users and billing.', true),
    ('default-standard', 'default', 'ObserveCraft Standard Role', 'Create and edit dashboards, monitors and incidents; read all telemetry.', true),
    ('default-readonly', 'default', 'ObserveCraft Read Only Role', 'Read-only access to telemetry and assets.', true);
INSERT INTO user_roles (user_id, role_id) VALUES ('00000000-0000-0000-0000-000000000001', 'default-admin');
UPDATE users SET title = 'Platform Engineer', login_methods = '{saml}', mfa_enabled = true, timezone = 'Europe/Berlin', last_login_at = now()
WHERE id = '00000000-0000-0000-0000-000000000001';
