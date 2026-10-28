-- Infrastructure → Resources → Policies: rules on the resource catalog's attributes (hosts,
-- Kubernetes and ECS resources), evaluated against the current catalog, with a daily compliance
-- snapshot per policy. See routes/resource_policies.rs and routes/resource_policy_rules.rs.

CREATE TABLE resource_policies (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id        TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name          TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    description   TEXT NOT NULL DEFAULT '',
    -- P0 (most urgent) … P4.
    priority      SMALLINT NOT NULL CHECK (priority BETWEEN 0 AND 4),
    category      TEXT NOT NULL CHECK (category IN ('cost_optimization', 'reliability', 'security', 'versioning', 'tagging')),
    -- A catalog resource type (`kube_deployment`, `host`, …).
    resource_type TEXT NOT NULL,
    -- Catalog query narrowing the resources (`env:prod cluster:shop-*`); '' for all of the type.
    scope         TEXT NOT NULL DEFAULT '',
    -- [{"attribute": "replicas", "operator": "gte", "values": ["2"]}, …], all must hold.
    conditions    JSONB NOT NULL,
    -- The template the policy was created from, if any.
    template      TEXT,
    created_by    UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (org_id, name),
    UNIQUE (org_id, id)
);

-- One row per org with policies; schedules the snapshots. Workers claim due orgs with
-- FOR UPDATE SKIP LOCKED and hold `claimed_until` while they evaluate.
CREATE TABLE resource_policy_settings (
    org_id           TEXT PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
    next_snapshot_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    claimed_until    TIMESTAMPTZ,
    snapshot_at      TIMESTAMPTZ
);
CREATE INDEX resource_policy_settings_due ON resource_policy_settings (next_snapshot_at);

-- A policy's compliance per UTC day (the day's latest evaluation).
CREATE TABLE resource_policy_snapshots (
    org_id       TEXT NOT NULL,
    policy_id    UUID NOT NULL,
    day          DATE NOT NULL,
    evaluated    INT NOT NULL,
    noncompliant INT NOT NULL,
    taken_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (policy_id, day),
    FOREIGN KEY (org_id, policy_id) REFERENCES resource_policies(org_id, id) ON DELETE CASCADE
);
CREATE INDEX resource_policy_snapshots_org ON resource_policy_snapshots (org_id, day);
