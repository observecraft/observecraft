-- Feature flags (Software Delivery → Feature Flags). Flags are managed through the API
-- (routes/feature_flags.rs) and evaluated by the intake for OpenFeature SDKs over OFREP
-- (oc-intake/src/ofrep.rs), which reads these tables with a short cache. The flag model and its
-- evaluation live in the oc-flags crate.

-- Environments group the runtime `env` values SDKs send (`production`, `staging-eu`, `dev-local`).
CREATE TABLE feature_flag_environments (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id            TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name              TEXT NOT NULL,
    -- The env values this environment answers for; `*` matches any run of characters.
    queries           TEXT[] NOT NULL DEFAULT '{}',
    is_production     BOOLEAN NOT NULL DEFAULT false,
    requires_approval BOOLEAN NOT NULL DEFAULT false,
    created_by        UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX feature_flag_environments_name ON feature_flag_environments (org_id, lower(name));

CREATE TABLE feature_flags (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    -- What the code asks for (`checkout-new-payment-flow`); it can't be changed.
    key         TEXT NOT NULL CHECK (key ~ '^[a-z0-9][a-z0-9._-]{0,99}$'),
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    flag_type   TEXT NOT NULL CHECK (flag_type IN ('boolean', 'string', 'integer', 'number', 'json')),
    -- [{"key": "on", "value": true}, …]
    variants    JSONB NOT NULL,
    -- What a new environment serves (one of the variants).
    default_variant TEXT NOT NULL,
    -- Which OFREP endpoints answer for it: bulk (client SDKs), single (server SDKs) or both.
    sdk_access  TEXT NOT NULL DEFAULT 'all' CHECK (sdk_access IN ('all', 'client', 'server')),
    lifecycle   TEXT NOT NULL DEFAULT 'temporary' CHECK (lifecycle IN ('temporary', 'permanent')),
    teams       TEXT[] NOT NULL DEFAULT '{}',
    tags        TEXT[] NOT NULL DEFAULT '{}',
    -- Optimistic concurrency: every change (details or an environment) increments it.
    version     INTEGER NOT NULL DEFAULT 1,
    created_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (org_id, key)
);

-- A flag's configuration per environment; a missing row means off.
CREATE TABLE feature_flag_env_configs (
    flag_id        UUID NOT NULL REFERENCES feature_flags(id) ON DELETE CASCADE,
    environment_id UUID NOT NULL REFERENCES feature_flag_environments(id) ON DELETE CASCADE,
    org_id         TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    enabled        BOOLEAN NOT NULL DEFAULT false,
    -- {"rules": [...], "defaultServe": {"variant": "off"}, "rollout": {...}} (oc_flags::FlagEnvConfig)
    config         JSONB NOT NULL,
    updated_by     UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (flag_id, environment_id)
);
CREATE INDEX feature_flag_env_configs_org ON feature_flag_env_configs (org_id);

-- Flag History: every change, kept after the flag is deleted.
CREATE TABLE feature_flag_changes (
    id               BIGSERIAL PRIMARY KEY,
    org_id           TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    flag_id          UUID REFERENCES feature_flags(id) ON DELETE SET NULL,
    flag_key         TEXT NOT NULL,
    environment_name TEXT NOT NULL DEFAULT '',
    -- created, updated, deleted, enabled, disabled, targeting
    action           TEXT NOT NULL,
    summary          TEXT NOT NULL DEFAULT '',
    actor_id         UUID REFERENCES users(id) ON DELETE SET NULL,
    actor            TEXT NOT NULL DEFAULT '',
    version          INTEGER NOT NULL DEFAULT 0,
    at               TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX feature_flag_changes_org ON feature_flag_changes (org_id, at DESC);
CREATE INDEX feature_flag_changes_flag ON feature_flag_changes (org_id, flag_id, at DESC);

-- The environment an API key's evaluations use when the context names none.
ALTER TABLE ingest_keys ADD COLUMN flag_environment_id UUID REFERENCES feature_flag_environments(id) ON DELETE SET NULL;
