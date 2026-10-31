-- Aggregated metric exports, committed before acknowledging their source log request.
-- This is a bounded retry spool, not a per-log journal. Org deletion removes pending exports.
CREATE TABLE log_metric_outbox (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    sums BYTEA NOT NULL,
    histograms BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (octet_length(sums) + octet_length(histograms) > 0)
);
CREATE INDEX log_metric_outbox_org ON log_metric_outbox (org_id, created_at, id);
