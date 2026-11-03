-- Assist's background runs: investigations of alerts, verdicts on security signals, custom agents' runs
-- and coding sessions. A queue claimed with FOR UPDATE SKIP LOCKED by any server; `heartbeat_at` lets
-- another server fail a run whose server died.
CREATE TABLE IF NOT EXISTS assist_runs (
    org_id        TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    id            UUID NOT NULL DEFAULT gen_random_uuid(),
    -- investigation | security | agent | code
    kind          TEXT NOT NULL,
    title         TEXT NOT NULL,
    -- queued | running | concluded | failed | cancelled
    status        TEXT NOT NULL DEFAULT 'queued',
    -- manual | monitor | signal | schedule
    trigger       TEXT NOT NULL DEFAULT 'manual',
    -- What the run is about: {monitorId, groupKey, alertAt, value} | {signalId} | {agentId} | {repo} | {question}
    subject       JSONB NOT NULL DEFAULT '{}',
    -- The member whose access the run has (who started it, or who turned on automatic runs).
    run_as        UUID REFERENCES users(id) ON DELETE SET NULL,
    created_by    TEXT NOT NULL DEFAULT '',
    steps         JSONB NOT NULL DEFAULT '[]',
    messages      JSONB NOT NULL DEFAULT '[]',
    conclusion    JSONB,
    verdict       TEXT,
    error         TEXT,
    -- helpful | not_helpful
    feedback      TEXT,
    input_tokens  BIGINT NOT NULL DEFAULT 0,
    output_tokens BIGINT NOT NULL DEFAULT 0,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at    TIMESTAMPTZ,
    heartbeat_at  TIMESTAMPTZ,
    finished_at   TIMESTAMPTZ,
    PRIMARY KEY (org_id, id)
);
CREATE INDEX IF NOT EXISTS assist_runs_kind ON assist_runs (org_id, kind, created_at DESC);
CREATE INDEX IF NOT EXISTS assist_runs_queue ON assist_runs (created_at) WHERE status = 'queued';
CREATE INDEX IF NOT EXISTS assist_runs_monitor ON assist_runs (org_id, ((subject->>'monitorId'))) WHERE kind = 'investigation';

-- Monitors whose alerts are investigated automatically, as the member who turned it on.
CREATE TABLE IF NOT EXISTS assist_monitor_settings (
    org_id           TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    monitor_id       BIGINT NOT NULL,
    auto_investigate BOOLEAN NOT NULL DEFAULT false,
    enabled_by       UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, monitor_id)
);

-- What investigations should keep in mind: for one monitor, or for the whole org.
CREATE TABLE IF NOT EXISTS assist_memories (
    org_id     TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    id         UUID NOT NULL DEFAULT gen_random_uuid(),
    monitor_id BIGINT,
    text       TEXT NOT NULL,
    source_run UUID,
    created_by TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, id)
);

-- Per-org settings of background runs: the knowledge base every run reads, rate limits, who is told
-- when a run concludes, and which security signals are judged automatically.
CREATE TABLE IF NOT EXISTS assist_run_settings (
    org_id                  TEXT PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
    knowledge               TEXT NOT NULL DEFAULT '',
    runs_per_hour           INT NOT NULL DEFAULT 30,
    automatic_runs_per_day  INT NOT NULL DEFAULT 100,
    notify                  TEXT[] NOT NULL DEFAULT '{}',
    security_auto_severities TEXT[] NOT NULL DEFAULT '{}',
    security_run_as         UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT now()
);
