-- Log indexes (Datadog model): each org routes incoming logs to the first index whose filter matches.
-- An index decides how long its logs are kept, can cap them per day and drop some with exclusion
-- filters. Logs that no index keeps still reach live tail and archives.

CREATE TABLE log_indexes (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id            TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    -- Lowercase slug; searched with `index:<name>`.
    name              TEXT NOT NULL,
    -- Search syntax; '' = every log.
    filter            TEXT NOT NULL DEFAULT '',
    retention_days    INTEGER NOT NULL CHECK (retention_days BETWEEN 1 AND 3650),
    -- Logs per day; NULL = unlimited. Resets daily at quota_reset_hour (UTC).
    daily_quota       BIGINT CHECK (daily_quota IS NULL OR daily_quota > 0),
    quota_reset_hour  SMALLINT NOT NULL DEFAULT 0 CHECK (quota_reset_hour BETWEEN 0 AND 23),
    -- [{ "name", "query", "sampleRate" (0–100 % of matches excluded), "enabled" }], first match wins.
    exclusion_filters JSONB NOT NULL DEFAULT '[]',
    -- Routing order (lowest first).
    position          INTEGER NOT NULL,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (org_id, name)
);
CREATE INDEX log_indexes_org ON log_indexes (org_id, position);

-- What each org may configure (set by the instance admin), and its own data settings.
CREATE TABLE org_data_settings (
    org_id                   TEXT PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
    max_log_retention_days   INTEGER NOT NULL DEFAULT 30 CHECK (max_log_retention_days BETWEEN 1 AND 3650),
    max_span_retention_days  INTEGER NOT NULL DEFAULT 30 CHECK (max_span_retention_days BETWEEN 1 AND 3650),
    max_indexes              INTEGER NOT NULL DEFAULT 20 CHECK (max_indexes BETWEEN 1 AND 1000),
    span_retention_days      INTEGER NOT NULL DEFAULT 15 CHECK (span_retention_days BETWEEN 1 AND 3650),
    updated_at               TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Existing orgs: one catch-all index, as before.
INSERT INTO org_data_settings (org_id) SELECT id FROM orgs ON CONFLICT DO NOTHING;
INSERT INTO log_indexes (org_id, name, filter, retention_days, position) SELECT id, 'main', '', 15, 0 FROM orgs ON CONFLICT DO NOTHING;
