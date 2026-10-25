-- Log archives: hourly Parquet files in the org's own S3-compatible bucket, searchable with DuckDB
-- and rehydratable into ClickHouse.

CREATE TABLE log_archives (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id         TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name           TEXT NOT NULL,
    -- S3 API endpoint as the server reaches it (https://s3.eu-west-1.amazonaws.com, http://rustfs:9000, …).
    endpoint       TEXT NOT NULL,
    region         TEXT NOT NULL DEFAULT 'us-east-1',
    bucket         TEXT NOT NULL,
    prefix         TEXT NOT NULL DEFAULT '',
    -- Path-style URLs (endpoint/bucket/key): needed by most S3-compatible stores.
    path_style     BOOLEAN NOT NULL DEFAULT true,
    access_key     TEXT NOT NULL,
    -- AES-256-GCM(OC_SECRET_KEY).
    secret_key_enc TEXT NOT NULL,
    -- Which logs to archive (search syntax); '' = all.
    query          TEXT NOT NULL DEFAULT '',
    enabled        BOOLEAN NOT NULL DEFAULT true,
    -- First hour to archive (logs older than this stay out).
    start_hour     TIMESTAMPTZ NOT NULL,
    created_by     UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (org_id, name)
);

-- One row per archived hour (done) or per failing hour (retried until it succeeds or gives up).
CREATE TABLE archive_runs (
    archive_id  UUID NOT NULL REFERENCES log_archives(id) ON DELETE CASCADE,
    hour        TIMESTAMPTZ NOT NULL,
    status      TEXT NOT NULL CHECK (status IN ('done', 'failed')),
    rows        BIGINT NOT NULL DEFAULT 0,
    bytes       BIGINT NOT NULL DEFAULT 0,
    object_key  TEXT,
    error       TEXT,
    attempts    INTEGER NOT NULL DEFAULT 1,
    finished_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (archive_id, hour)
);

CREATE TABLE archive_searches (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id        TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    archive_id    UUID NOT NULL REFERENCES log_archives(id) ON DELETE CASCADE,
    query         TEXT NOT NULL,
    range_from    TIMESTAMPTZ NOT NULL,
    range_to      TIMESTAMPTZ NOT NULL,
    max_results   INTEGER NOT NULL,
    status        TEXT NOT NULL DEFAULT 'running' CHECK (status IN ('running', 'done', 'failed', 'canceled')),
    files_total   INTEGER NOT NULL DEFAULT 0,
    files_scanned INTEGER NOT NULL DEFAULT 0,
    bytes_scanned BIGINT NOT NULL DEFAULT 0,
    matches       BIGINT NOT NULL DEFAULT 0,
    error         TEXT,
    created_by    UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at   TIMESTAMPTZ
);
CREATE INDEX archive_searches_org ON archive_searches (org_id, created_at DESC);

-- Matching events of a search (newest first), capped at max_results.
CREATE TABLE archive_search_hits (
    search_id UUID NOT NULL REFERENCES archive_searches(id) ON DELETE CASCADE,
    seq       INTEGER NOT NULL,
    event     JSONB NOT NULL,
    PRIMARY KEY (search_id, seq)
);

CREATE TABLE rehydrations (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    archive_id  UUID NOT NULL REFERENCES log_archives(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    query       TEXT NOT NULL,
    range_from  TIMESTAMPTZ NOT NULL,
    range_to    TIMESTAMPTZ NOT NULL,
    status      TEXT NOT NULL DEFAULT 'running' CHECK (status IN ('running', 'done', 'failed', 'expired')),
    rows        BIGINT NOT NULL DEFAULT 0,
    error       TEXT,
    expires_at  TIMESTAMPTZ NOT NULL,
    created_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at TIMESTAMPTZ
);
CREATE INDEX rehydrations_org ON rehydrations (org_id, created_at DESC);
