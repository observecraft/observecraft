-- Generated metrics (Logs → Configuration → Generate Metrics): the indexer counts every log
-- matching a definition's query, after pipelines and before index routing, and writes the counts
-- (or a distribution of a numeric attribute) as metric points, one series per group-by value set.

CREATE TABLE log_metrics (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    -- The metric's name (`logs.checkout.errors`); it can't be changed.
    name        TEXT NOT NULL,
    -- Search syntax; '' = every log.
    query       TEXT NOT NULL DEFAULT '',
    -- 'count' of matching logs, or 'distribution' of the numeric attribute `path` (`@duration`).
    aggregation TEXT NOT NULL CHECK (aggregation IN ('count', 'distribution')),
    path        TEXT,
    -- [{ "path": "@http.status_code", "tagName": "http.status_code" }]
    group_by    JSONB NOT NULL DEFAULT '[]',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (org_id, name),
    CHECK ((aggregation = 'distribution') = (path IS NOT NULL))
);
