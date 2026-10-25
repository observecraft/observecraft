-- Control-plane metadata. Telemetry lives in ClickHouse; everything users create lives here.
-- Every row is scoped by org_id (multi-tenant from day one).

CREATE TABLE orgs (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE users (
    id          UUID PRIMARY KEY,
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    email       TEXT NOT NULL,
    name        TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (org_id, email)
);

CREATE TABLE teams (
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    handle      TEXT NOT NULL,
    name        TEXT NOT NULL,
    PRIMARY KEY (org_id, handle)
);

-- Dashboards are stored as the JSON document the UI edits (layoutType, templateVariables, widgets).
CREATE TABLE dashboards (
    id          TEXT PRIMARY KEY,
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    title       TEXT NOT NULL,
    description TEXT,
    kind        TEXT NOT NULL CHECK (kind IN ('custom', 'integration')),
    logo        TEXT,
    author_id   UUID REFERENCES users(id) ON DELETE SET NULL,
    teams       TEXT[] NOT NULL DEFAULT '{}',
    definition  JSONB NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    modified_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at  TIMESTAMPTZ
);
CREATE INDEX dashboards_org_modified ON dashboards (org_id, modified_at DESC) WHERE deleted_at IS NULL;

CREATE TABLE dashboard_user_state (
    dashboard_id TEXT NOT NULL REFERENCES dashboards(id) ON DELETE CASCADE,
    user_id      UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    starred      BOOLEAN NOT NULL DEFAULT false,
    viewed_at    TIMESTAMPTZ,
    PRIMARY KEY (dashboard_id, user_id)
);

CREATE TABLE dashboard_lists (
    id          TEXT PRIMARY KEY,
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    dashboard_ids TEXT[] NOT NULL DEFAULT '{}'
);

CREATE TABLE monitors (
    id          BIGSERIAL PRIMARY KEY,
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    query       TEXT NOT NULL,
    definition  JSONB NOT NULL,
    status      TEXT NOT NULL DEFAULT 'nodata' CHECK (status IN ('ok', 'warn', 'alert', 'nodata')),
    muted       BOOLEAN NOT NULL DEFAULT false,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    modified_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE integration_installs (
    org_id         TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    integration_id TEXT NOT NULL,
    settings       JSONB NOT NULL DEFAULT '{}',
    installed_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, integration_id)
);

-- Development org until authentication lands.
INSERT INTO orgs (id, name) VALUES ('default', 'Default organization');
