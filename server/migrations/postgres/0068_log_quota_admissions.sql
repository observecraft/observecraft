-- Shared quota metadata for quota-bearing indexes only. Reserve before ClickHouse delivery;
-- Kafka retains ownership until delivery, while retry/restart reuses the durable admission.
CREATE TABLE log_quota_epochs (
    org_id TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    index_name TEXT NOT NULL,
    bootstrap_day BIGINT NOT NULL,
    PRIMARY KEY (org_id, index_name)
);
CREATE TABLE log_quota_counters (
    org_id TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    index_name TEXT NOT NULL,
    day_start BIGINT NOT NULL,
    n BIGINT NOT NULL CHECK (n >= 0),
    PRIMARY KEY (org_id, index_name, day_start)
);
CREATE TABLE log_quota_admissions (
    org_id TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    event_id UUID NOT NULL,
    index_name TEXT NOT NULL,
    retention_days INTEGER NOT NULL CHECK (retention_days BETWEEN 1 AND 65535),
    expires_at TIMESTAMPTZ NOT NULL DEFAULT now() + INTERVAL '7 days',
    PRIMARY KEY (org_id, event_id)
);
CREATE INDEX log_quota_admissions_expiry ON log_quota_admissions (org_id, expires_at);
