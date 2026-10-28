-- Synthetic API tests (Digital Experience → Synthetic Tests): HTTP, SSL, DNS and TCP checks the
-- server runs on a schedule. Each test alerts through a monitor of type 'synthetics' that the
-- test runner keeps up to date (so notifications, downtimes, monitor SLOs and the monitor list
-- work as for any monitor); the evaluator leaves those monitors alone.

ALTER TABLE monitors DROP CONSTRAINT IF EXISTS monitors_type_check;
ALTER TABLE monitors ADD CONSTRAINT monitors_type_check CHECK (type IN ('metric', 'apm', 'log', 'synthetics'));

CREATE TABLE synthetic_tests (
    id                     UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id                 TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name                   TEXT NOT NULL,
    type                   TEXT NOT NULL CHECK (type IN ('http', 'ssl', 'dns', 'tcp')),
    -- {"method", "url", "host", "port", "headers", "body", "basicAuth", "timeoutSeconds", …}
    request                JSONB NOT NULL,
    -- [{"type": "statusCode", "operator": "is", "property": "", "target": "200"}, …]
    assertions             JSONB NOT NULL DEFAULT '[]',
    locations              TEXT[] NOT NULL,
    frequency_s            INT NOT NULL CHECK (frequency_s BETWEEN 60 AND 604800),
    retry_count            INT NOT NULL DEFAULT 0 CHECK (retry_count BETWEEN 0 AND 3),
    retry_interval_ms      INT NOT NULL DEFAULT 300 CHECK (retry_interval_ms BETWEEN 0 AND 60000),
    -- Alert once the test has failed for this long (0: on the first failed run) …
    min_failure_duration_s INT NOT NULL DEFAULT 0 CHECK (min_failure_duration_s >= 0),
    -- … from at least this many locations.
    min_location_failed    INT NOT NULL DEFAULT 1 CHECK (min_location_failed >= 1),
    env                    TEXT NOT NULL DEFAULT '',
    team                   TEXT NOT NULL DEFAULT '',
    tags                   TEXT[] NOT NULL DEFAULT '{}',
    -- Notification message of the test's monitor; its @handles are the recipients.
    message                TEXT NOT NULL DEFAULT '',
    priority               SMALLINT CHECK (priority BETWEEN 1 AND 5),
    paused                 BOOLEAN NOT NULL DEFAULT false,
    monitor_id             BIGINT REFERENCES monitors(id) ON DELETE SET NULL,
    -- Scheduling: a runner claims due tests (FOR UPDATE SKIP LOCKED), moves next_run_at to the
    -- next slot and holds claimed_until while the run (and its retries) lasts.
    next_run_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    claimed_until          TIMESTAMPTZ,
    -- First failed scheduled run of the current failure streak.
    failing_since          TIMESTAMPTZ,
    last_run_at            TIMESTAMPTZ,
    created_by             UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_by             UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at             TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX synthetic_tests_org ON synthetic_tests (org_id);
CREATE INDEX synthetic_tests_due ON synthetic_tests (next_run_at) WHERE NOT paused;

-- One row per run (after its retries). Kept 30 days.
CREATE TABLE synthetic_results (
    id          BIGSERIAL PRIMARY KEY,
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    test_id     UUID NOT NULL REFERENCES synthetic_tests(id) ON DELETE CASCADE,
    location    TEXT NOT NULL,
    started_at  TIMESTAMPTZ NOT NULL,
    duration_ms DOUBLE PRECISION NOT NULL,
    passed      BOOLEAN NOT NULL,
    status_code INT,
    error       TEXT,
    attempts    INT NOT NULL DEFAULT 1,
    -- scheduled | manual (run now: recorded, but doesn't change the alert state)
    trigger     TEXT NOT NULL CHECK (trigger IN ('scheduled', 'manual')),
    -- Request and response, assertion results, certificate or DNS records.
    details     JSONB NOT NULL
);
CREATE INDEX synthetic_results_test ON synthetic_results (test_id, started_at DESC);
CREATE INDEX synthetic_results_started ON synthetic_results (started_at);
