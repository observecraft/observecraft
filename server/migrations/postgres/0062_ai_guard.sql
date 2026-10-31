-- AI Guard: per-organization evaluation settings (the evaluations themselves are telemetry, in
-- ClickHouse otel.ai_guard_evaluations).
CREATE TABLE IF NOT EXISTS ai_guard_settings (
    org_id               TEXT PRIMARY KEY,
    -- Whether SDKs should block unsafe steps (the response's is_blocking_enabled): for every
    -- service, or only for the services listed in blocking_services.
    blocking_enabled     BOOLEAN NOT NULL DEFAULT false,
    blocking_services    TEXT[] NOT NULL DEFAULT '{}',
    -- Confidence at or above which a category is reported (0..1).
    threshold            DOUBLE PRECISION NOT NULL DEFAULT 0.5,
    -- Categories the organization turned off.
    disabled_categories  TEXT[] NOT NULL DEFAULT '{}',
    sds_enabled          BOOLEAN NOT NULL DEFAULT true,
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);
