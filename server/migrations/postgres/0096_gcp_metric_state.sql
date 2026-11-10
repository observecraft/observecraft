-- The GCP crawler's state per project job, beside its watermark (`cloud_crawl_jobs.collected_until`): `marks` holds
-- the latest published point of each series of a metric sampled less often than once a minute (Cloud Storage's daily
-- sizes and object counts, hourly and 5-minute metrics; series key → epoch ms), as those are re-read over their last
-- intervals and the metric tables keep every point they get (a re-published point would count twice). Removed with
-- the service account.
CREATE TABLE IF NOT EXISTS gcp_metric_state (
    org_id     TEXT NOT NULL,
    account_id UUID NOT NULL,
    project    TEXT NOT NULL,
    marks      JSONB NOT NULL DEFAULT '{}',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, account_id, project),
    FOREIGN KEY (org_id, account_id) REFERENCES gcp_accounts (org_id, id) ON DELETE CASCADE
);
