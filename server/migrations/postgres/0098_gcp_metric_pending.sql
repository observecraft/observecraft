-- The GCP crawler's failed one-minute reads (throttling, server errors: metric type, interval, the point after which
-- they publish), retried on the next runs for up to a day, beside the coarse-metric marks of `gcp_metric_state`.
ALTER TABLE gcp_metric_state ADD COLUMN IF NOT EXISTS pending JSONB NOT NULL DEFAULT '[]';
