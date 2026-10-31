-- Logs → Configuration → Migration: imports of another platform's saved searches, alerts,
-- dashboards and macros, translated to ObserveCraft searches and migrated to monitors and dashboards.
CREATE TABLE log_imports (
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    id          UUID NOT NULL DEFAULT gen_random_uuid(),
    name        TEXT NOT NULL,
    -- splunk
    source      TEXT NOT NULL,
    created_by  UUID,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Macro definitions used while translating: {"name(1)": {"args": ["x"], "definition": "…"}}.
    macros      JSONB NOT NULL DEFAULT '{}',
    PRIMARY KEY (org_id, id)
);

CREATE TABLE log_import_assets (
    org_id      TEXT NOT NULL,
    import_id   UUID NOT NULL,
    id          UUID NOT NULL DEFAULT gen_random_uuid(),
    -- alert | dashboard | search
    kind        TEXT NOT NULL,
    title       TEXT NOT NULL,
    -- The asset as exported (the CSV row), and its translation: {"searches": [...]}.
    original    JSONB NOT NULL,
    translation JSONB NOT NULL,
    -- ready | partial | unsupported | migrated | failed
    status      TEXT NOT NULL,
    -- monitor | dashboard, and the id of what it was migrated to.
    target_kind TEXT,
    target_id   TEXT,
    error       TEXT,
    migrated_at TIMESTAMPTZ,
    position    INT NOT NULL DEFAULT 0,
    PRIMARY KEY (org_id, import_id, id),
    FOREIGN KEY (org_id, import_id) REFERENCES log_imports(org_id, id) ON DELETE CASCADE
);

-- Personal Settings: per-user preferences (home page, time format, accessibility), and email verification.
CREATE TABLE user_preferences (
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    prefs       JSONB NOT NULL DEFAULT '{}',
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, user_id)
);

ALTER TABLE users ADD COLUMN IF NOT EXISTS modified_at TIMESTAMPTZ;
ALTER TABLE users ADD COLUMN IF NOT EXISTS email_verified_at TIMESTAMPTZ;

CREATE TABLE email_verifications (
    token_hash  BYTEA PRIMARY KEY,
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    email       TEXT NOT NULL,
    expires_at  TIMESTAMPTZ NOT NULL
);
