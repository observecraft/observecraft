-- APM → Settings: retention filters (Terraform `datadog_apm_retention_filter` / `_order`), span-based
-- metrics (`datadog_spans_metric`), span pipelines, and the APM defaults of an org.

-- Retention filters, in execution order. The defaults (errors, App & API Protection) are rows too,
-- with fixed ids, installed per org; they can be changed but not deleted.
CREATE TABLE IF NOT EXISTS apm_retention_filters (
    org_id          TEXT NOT NULL,
    id              TEXT NOT NULL,
    name            TEXT NOT NULL,
    query           TEXT NOT NULL DEFAULT '*',
    -- spans-sampling-processor | spans-errors-sampling-processor | spans-appsec-sampling-processor
    filter_type     TEXT NOT NULL DEFAULT 'spans-sampling-processor',
    rate            DOUBLE PRECISION NOT NULL,
    trace_rate      DOUBLE PRECISION NOT NULL DEFAULT 0,
    enabled         BOOLEAN NOT NULL DEFAULT true,
    execution_order INTEGER NOT NULL DEFAULT 0,
    created_by      TEXT NOT NULL DEFAULT '',
    created_at      BIGINT NOT NULL DEFAULT 0,
    modified_by     TEXT NOT NULL DEFAULT '',
    modified_at     BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (org_id, id)
);

-- Span-based metrics; the id is the metric name.
CREATE TABLE IF NOT EXISTS apm_spans_metrics (
    org_id              TEXT NOT NULL,
    id                  TEXT NOT NULL,
    query               TEXT NOT NULL DEFAULT '*',
    -- count | distribution
    aggregation_type    TEXT NOT NULL,
    path                TEXT,
    include_percentiles BOOLEAN,
    -- [{"path": "resource_name", "tag_name": "resource_name"}]; tag_name only when given
    group_by            JSONB NOT NULL DEFAULT '[]',
    created_by_name     TEXT NOT NULL DEFAULT '',
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, id)
);

-- Up to where (epoch ms, a minute boundary) an org's span-based metrics have been computed.
CREATE TABLE IF NOT EXISTS apm_spans_metrics_progress (
    org_id     TEXT PRIMARY KEY,
    done_until BIGINT NOT NULL
);

-- Span pipelines (processors on span attributes), in order.
CREATE TABLE IF NOT EXISTS apm_span_pipelines (
    org_id          TEXT NOT NULL,
    id              UUID NOT NULL DEFAULT gen_random_uuid(),
    name            TEXT NOT NULL,
    query           TEXT NOT NULL DEFAULT '',
    enabled         BOOLEAN NOT NULL DEFAULT true,
    processors      JSONB NOT NULL DEFAULT '[]',
    position        INTEGER NOT NULL DEFAULT 0,
    updated_by_name TEXT NOT NULL DEFAULT '',
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, id)
);

-- APM defaults: the default environment (NULL = automatically detected) and primary tags.
CREATE TABLE IF NOT EXISTS apm_org_settings (
    org_id              TEXT PRIMARY KEY,
    default_env         TEXT,
    primary_tags        JSONB NOT NULL DEFAULT '[]',
    default_primary_tag TEXT,
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);
