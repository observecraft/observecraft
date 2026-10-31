-- Logs → Configuration through Datadog's API (`datadog_logs_*` Terraform resources) on the same
-- records the UI edits, plus Flex Logs and custom destinations.

-- Indexes: Datadog's fields we had no column for. `flex_retention_days` is the total time logs stay
-- in the index (standard + flex tier); the indexer keeps rows for the longer of the two.
ALTER TABLE log_indexes
    ADD COLUMN flex_retention_days INTEGER CHECK (flex_retention_days IS NULL OR flex_retention_days BETWEEN 1 AND 3650),
    ADD COLUMN tags JSONB NOT NULL DEFAULT '[]',
    ADD COLUMN daily_limit_warning_threshold DOUBLE PRECISION CHECK (daily_limit_warning_threshold IS NULL OR daily_limit_warning_threshold BETWEEN 50 AND 99.99),
    -- The reset time as Datadog writes it (`+02:00`); quota_reset_hour stays the UTC hour the indexer uses.
    ADD COLUMN reset_utc_offset TEXT NOT NULL DEFAULT '+00:00';

-- Archives: Datadog's destination document (secrets left out), its options, and an order.
ALTER TABLE log_archives
    ADD COLUMN destination JSONB,
    ADD COLUMN include_tags BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN rehydration_tags JSONB NOT NULL DEFAULT '[]',
    ADD COLUMN rehydration_max_scan_size_gb BIGINT,
    ADD COLUMN position INTEGER NOT NULL DEFAULT 0;
UPDATE log_archives a SET position = r.n FROM (SELECT id, row_number() OVER (PARTITION BY org_id ORDER BY created_at) - 1 AS n FROM log_archives) r WHERE r.id = a.id;

ALTER TABLE rehydrations ADD COLUMN bytes_scanned BIGINT NOT NULL DEFAULT 0;

ALTER TABLE log_pipelines
    ADD COLUMN description TEXT NOT NULL DEFAULT '',
    ADD COLUMN tags JSONB NOT NULL DEFAULT '[]',
    ADD COLUMN updated_by UUID REFERENCES users(id) ON DELETE SET NULL;

ALTER TABLE log_metrics ADD COLUMN include_percentiles BOOLEAN NOT NULL DEFAULT false;

-- Custom destinations: logs matching `query` (after pipelines) are forwarded to an HTTP endpoint,
-- Splunk HEC, Elasticsearch or Microsoft Sentinel.
CREATE TABLE log_custom_destinations (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id              TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name                TEXT NOT NULL,
    query               TEXT NOT NULL DEFAULT '',
    enabled             BOOLEAN NOT NULL DEFAULT true,
    forward_tags        BOOLEAN NOT NULL DEFAULT true,
    -- Tag keys kept (ALLOW_LIST) or dropped (BLOCK_LIST) when tags are forwarded.
    tags_list           JSONB NOT NULL DEFAULT '[]',
    tags_list_type      TEXT NOT NULL DEFAULT 'ALLOW_LIST' CHECK (tags_list_type IN ('ALLOW_LIST', 'BLOCK_LIST')),
    -- Datadog's forwarder_destination without its secrets (`type`: http, splunk_hec, elasticsearch, microsoft_sentinel).
    destination         JSONB NOT NULL,
    -- The secrets (password, header value, access token) as JSON, AES-256-GCM(OC_SECRET_KEY).
    secrets_enc         TEXT NOT NULL DEFAULT '',
    position            INTEGER NOT NULL DEFAULT 0,
    -- Forwarding state, written by the forwarder.
    forwarded           BIGINT NOT NULL DEFAULT 0,
    last_error          TEXT,
    last_error_at       TIMESTAMPTZ,
    created_by          UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (org_id, name)
);
CREATE INDEX log_custom_destinations_org ON log_custom_destinations (org_id, position);

-- Flex Logs: an org turns the flex tier on (indexes may then keep logs longer than their standard
-- retention) and picks a compute size for flex queries.
CREATE TABLE org_logs_settings (
    org_id        TEXT PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
    flex_enabled  BOOLEAN NOT NULL DEFAULT false,
    -- 'starter' or 'scalable'; with scalable, the size (xs, xs+, s, m, l).
    flex_compute  TEXT NOT NULL DEFAULT 'starter' CHECK (flex_compute IN ('starter', 'scalable')),
    flex_size     TEXT CHECK (flex_size IS NULL OR flex_size IN ('xs', 'xs+', 's', 'm', 'l')),
    updated_by    UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
