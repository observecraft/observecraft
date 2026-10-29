-- Progress of the metric rollup job (docs/adr/0002-storage-at-saas-scale.md, src/metric_rollups.rs):
-- per org and tier, every bucket before `done_until` is rolled up ('5m': from the samples, '1h':
-- from the 5-minute rollups). Queries read rollups before it and raw samples after it. Servers
-- claim rows with FOR UPDATE SKIP LOCKED and hold `claimed_until` while they work; moving
-- `done_until` back re-rolls from there (a re-rolled bucket replaces its rows).
CREATE TABLE metric_rollups (
    org_id        TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    tier          TEXT NOT NULL CHECK (tier IN ('5m', '1h')),
    done_until    TIMESTAMPTZ NOT NULL,
    claimed_until TIMESTAMPTZ,
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, tier)
);
