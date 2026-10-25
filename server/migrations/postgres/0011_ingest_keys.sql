-- API keys for sending telemetry (OTLP / logs intake). Each key belongs to an organization and is
-- what the intake gateway turns into that org's `oc.org_id`. Shown once; stored hashed.
CREATE TABLE ingest_keys (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id       TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    key_hash     BYTEA NOT NULL UNIQUE,
    -- First characters, to recognise a key in lists ("ocik_3f9a…").
    prefix       TEXT NOT NULL,
    created_by   UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ,
    revoked_at   TIMESTAMPTZ
);
CREATE INDEX ingest_keys_org ON ingest_keys (org_id);
