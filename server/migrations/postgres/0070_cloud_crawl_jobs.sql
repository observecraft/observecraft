-- The cloud integrations' crawlers (crates/oc-api/src/cloud/): one job per provider, account and scope (an
-- AWS region, an Azure subscription, a GCP project; the empty scope is an account's discovery job, which
-- lists its subscriptions or projects). Servers claim due jobs with FOR UPDATE SKIP LOCKED and hold them
-- with `claimed_until`, so adding servers adds crawl capacity and a job runs once. `collected_until` is the
-- watermark: it advances only after Kafka acknowledged the window's points. The last error is the
-- account's issue. Jobs of removed accounts are deleted by the planner (accounts live in one table per
-- provider, so there is no foreign key).
CREATE TABLE IF NOT EXISTS cloud_crawl_jobs (
    provider          TEXT NOT NULL,
    org_id            TEXT NOT NULL,
    account_id        UUID NOT NULL,
    scope             TEXT NOT NULL,
    next_run_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    claimed_until     TIMESTAMPTZ,
    collected_until   TIMESTAMPTZ,
    last_run_at       TIMESTAMPTZ,
    last_success_at   TIMESTAMPTZ,
    failures          INTEGER NOT NULL DEFAULT 0,
    -- access_denied | auth | throttled | disabled | other
    error_kind        TEXT,
    error_message     TEXT,
    missing_action    TEXT,
    error_at          TIMESTAMPTZ,
    metrics_last_run  INTEGER NOT NULL DEFAULT 0,
    points_last_run   INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (provider, org_id, account_id, scope)
);

CREATE INDEX IF NOT EXISTS cloud_crawl_jobs_due ON cloud_crawl_jobs (provider, next_run_at);
