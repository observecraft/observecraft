-- The AWS crawler's daily metrics (S3's storage metrics, Storage Lens: one point a day, published for the previous
-- day, sometimes a day late). Each job (account and region) re-reads their last 3 days once an hour and publishes
-- only the days it hasn't yet: `marks` holds each series' latest published day (series key → epoch ms), as the metric
-- tables keep every point they get (a re-published day would count twice). Removed with the account.
CREATE TABLE IF NOT EXISTS aws_daily_metric_marks (
    org_id     TEXT NOT NULL,
    account_id UUID NOT NULL,
    region     TEXT NOT NULL,
    marks      JSONB NOT NULL DEFAULT '{}',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, account_id, region),
    FOREIGN KEY (org_id, account_id) REFERENCES aws_accounts (org_id, id) ON DELETE CASCADE
);
