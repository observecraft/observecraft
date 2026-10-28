-- Developer Portal → Software Catalog and Reports (routes/portal_catalog.rs, routes/portal_reports.rs).
-- Telemetry entities (APM services, datastores and queues inferred from spans, endpoints) are
-- computed on request; only what users declare is stored here.

-- Entities users define in the UI, through the API or by importing entity definition documents
-- (apiVersion v3). `definition` is the validated, normalized document. A service's team, contacts,
-- code repository and docs live in service_metadata (shared with Scorecards), not here.
CREATE TABLE portal_entities (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id     TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    kind       TEXT NOT NULL CHECK (kind IN ('service', 'datastore', 'queue', 'api', 'system', 'frontend')),
    name       TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
    definition JSONB NOT NULL,
    -- How it was last written: ui | api | import.
    source     TEXT NOT NULL DEFAULT 'api' CHECK (source IN ('ui', 'api', 'import')),
    created_by UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_by UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (org_id, kind, name)
);

-- Reports → Custom Reports: the org's dashboards shown in the Reports tab, in order.
CREATE TABLE portal_report_dashboards (
    org_id       TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    dashboard_id TEXT NOT NULL REFERENCES dashboards(id) ON DELETE CASCADE,
    position     INT NOT NULL,
    added_by     UUID REFERENCES users(id) ON DELETE SET NULL,
    added_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, dashboard_id)
);

-- Reports → Scorecards Performance: each service's overall scorecard level per day, written by
-- the scorecard evaluations (the last evaluation of a day wins).
CREATE TABLE portal_scorecard_levels (
    org_id  TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    day     DATE NOT NULL,
    service TEXT NOT NULL,
    team    TEXT NOT NULL DEFAULT '',
    level   SMALLINT NOT NULL CHECK (level BETWEEN 0 AND 3),
    PRIMARY KEY (org_id, day, service)
);
