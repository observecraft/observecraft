-- The Azure crawler's state per subscription job, beside its watermark (`cloud_crawl_jobs.collected_until`):
-- `marks` holds the latest published point of each series read at a coarser grain than a minute (series key → epoch
-- ms), as those are re-read over a few intervals and the metric tables keep every point they get (a re-published
-- point would count twice); `pending` holds the one-minute reads that failed (throttling, server errors: type,
-- region, resources, metrics, window), retried on the next runs. Removed with the app registration.
CREATE TABLE IF NOT EXISTS azure_metric_state (
    org_id       TEXT NOT NULL,
    account_id   UUID NOT NULL,
    subscription TEXT NOT NULL,
    marks        JSONB NOT NULL DEFAULT '{}',
    pending      JSONB NOT NULL DEFAULT '[]',
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, account_id, subscription),
    FOREIGN KEY (org_id, account_id) REFERENCES azure_accounts (org_id, id) ON DELETE CASCADE
);
