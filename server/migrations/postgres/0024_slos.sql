-- Service level objectives (Monitoring → Service Level Objectives) and their corrections. The SLI,
-- error budget and state are computed on request from metrics or monitor history (routes/slos.rs).

CREATE TABLE slos (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id         TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name           TEXT NOT NULL,
    description    TEXT NOT NULL DEFAULT '',
    type           TEXT NOT NULL CHECK (type IN ('metric', 'monitor', 'time_slice')),
    -- Metric SLOs: good and total events (metric queries or formulas).
    good_query     TEXT,
    total_query    TEXT,
    -- Monitor SLOs: up while none of these monitors (of the same org) alerts.
    monitor_ids    BIGINT[] NOT NULL DEFAULT '{}',
    -- Time-slice SLOs: a slice is good when the query's value passes `comparator threshold`.
    slice_query    TEXT,
    comparator     TEXT CHECK (comparator IN ('<', '<=', '>', '>=')),
    threshold      DOUBLE PRECISION,
    slice_minutes  INT CHECK (slice_minutes IN (1, 5)),
    -- [{"window": "7d", "target": 99.9, "warning": 99.95}, …]
    targets        JSONB NOT NULL,
    primary_window TEXT NOT NULL CHECK (primary_window IN ('1d', '7d', '30d', '90d')),
    tags           TEXT[] NOT NULL DEFAULT '{}',
    teams          TEXT[] NOT NULL DEFAULT '{}',
    created_by     UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Target of the corrections' (org, SLO) key.
    UNIQUE (org_id, id),
    CHECK (type <> 'metric' OR (good_query IS NOT NULL AND total_query IS NOT NULL)),
    CHECK (type <> 'monitor' OR cardinality(monitor_ids) > 0),
    CHECK (type <> 'time_slice' OR (slice_query IS NOT NULL AND comparator IS NOT NULL AND threshold IS NOT NULL AND slice_minutes IS NOT NULL))
);
CREATE INDEX slos_org ON slos (org_id);

-- Time excluded from SLO calculations (maintenance, deployments): for one SLO, or every SLO of the
-- org when slo_id is NULL.
CREATE TABLE slo_corrections (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    slo_id      UUID,
    category    TEXT NOT NULL CHECK (category IN ('scheduled_maintenance', 'outside_business_hours', 'deployment', 'other')),
    description TEXT NOT NULL DEFAULT '',
    start_at    TIMESTAMPTZ NOT NULL,
    end_at      TIMESTAMPTZ NOT NULL,
    created_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (end_at > start_at),
    FOREIGN KEY (org_id, slo_id) REFERENCES slos(org_id, id) ON DELETE CASCADE
);
CREATE INDEX slo_corrections_org ON slo_corrections (org_id, end_at);
