-- Automation → Workflow Automation and App Builder. See routes/workflows.rs, routes/workflow_engine.rs
-- and routes/apps.rs.

-- A workflow: form inputs, triggers and steps. `spec` is the saved draft; `published_spec` is what
-- monitors, schedules, incidents, the API and apps run (NULL until it is published).
CREATE TABLE workflows (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id           TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name             TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    -- Monitors start it by mentioning `@workflow-<handle>` in their message.
    handle           TEXT NOT NULL,
    description      TEXT NOT NULL DEFAULT '',
    tags             TEXT[] NOT NULL DEFAULT '{}',
    owner_team       TEXT NOT NULL DEFAULT '',
    service          TEXT NOT NULL DEFAULT '',
    -- Blueprint the workflow started from; '' when built from scratch.
    blueprint        TEXT NOT NULL DEFAULT '',
    -- {"inputs": [...], "triggers": [...], "steps": [...]}
    spec             JSONB NOT NULL,
    published_spec   JSONB,
    published_at     TIMESTAMPTZ,
    -- Triggered runs act as this member (their access and log restrictions apply).
    published_by     UUID REFERENCES users(id) ON DELETE SET NULL,
    -- AES-GCM sealed JSON object {"name": "value"} of the credentials `{{secrets.name}}` refers to.
    secrets_enc      TEXT,
    -- SHA-256 of the token API triggers send in `X-Workflow-Token`.
    api_token_hash   BYTEA,
    -- When the published schedule fires next (NULL without a schedule).
    next_schedule_at TIMESTAMPTZ,
    created_by       UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (org_id, name),
    UNIQUE (org_id, handle),
    UNIQUE (org_id, id)
);
CREATE INDEX workflows_schedule ON workflows (next_schedule_at) WHERE next_schedule_at IS NOT NULL;

-- One execution. `state` is the run state machine (blocks being run, step results) kept after every
-- step, so a run resumes after a wait or on another server once `lease_until` has passed.
CREATE TABLE workflow_runs (
    id            BIGSERIAL PRIMARY KEY,
    org_id        TEXT NOT NULL,
    workflow_id   UUID NOT NULL,
    -- manual | monitor | schedule | incident | api | app
    trigger_kind  TEXT NOT NULL,
    -- Who or what started it ("Alex Morgan", "Monitor #12", "Schedule").
    triggered_by  TEXT NOT NULL,
    run_as        UUID REFERENCES users(id) ON DELETE SET NULL,
    -- The `Trigger` data templates read.
    trigger       JSONB NOT NULL DEFAULT '{}',
    inputs        JSONB NOT NULL DEFAULT '{}',
    -- The spec as it was when the run started.
    spec          JSONB NOT NULL,
    status        TEXT NOT NULL DEFAULT 'queued' CHECK (status IN ('queued', 'running', 'waiting', 'succeeded', 'failed', 'canceled')),
    state         JSONB,
    error         TEXT,
    -- Runnable from then on (queued, or waiting until a wait step is over).
    next_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- A server is running it until then.
    lease_until   TIMESTAMPTZ,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at    TIMESTAMPTZ,
    finished_at   TIMESTAMPTZ,
    UNIQUE (org_id, id),
    FOREIGN KEY (org_id, workflow_id) REFERENCES workflows(org_id, id) ON DELETE CASCADE
);
CREATE INDEX workflow_runs_due ON workflow_runs (next_at) WHERE status IN ('queued', 'running', 'waiting');
CREATE INDEX workflow_runs_workflow ON workflow_runs (workflow_id, created_at DESC);

-- What each step of a run did (control steps record the branch taken or the item count).
CREATE TABLE workflow_run_steps (
    id          BIGSERIAL PRIMARY KEY,
    org_id      TEXT NOT NULL,
    run_id      BIGINT NOT NULL,
    step_id     TEXT NOT NULL,
    kind        TEXT NOT NULL,
    name        TEXT NOT NULL DEFAULT '',
    -- The innermost "for each" item, when inside one.
    iteration   INT,
    status      TEXT NOT NULL CHECK (status IN ('succeeded', 'failed')),
    -- Rendered inputs and the output, credentials redacted.
    inputs      JSONB NOT NULL DEFAULT '{}',
    output      JSONB NOT NULL DEFAULT '{}',
    error       TEXT,
    attempts    INT NOT NULL DEFAULT 1,
    started_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    duration_ms DOUBLE PRECISION NOT NULL DEFAULT 0,
    FOREIGN KEY (org_id, run_id) REFERENCES workflow_runs(org_id, id) ON DELETE CASCADE
);
CREATE INDEX workflow_run_steps_run ON workflow_run_steps (run_id, id);

-- An app: a page of components (text, inputs, buttons, tables, metric values) whose buttons run
-- workflows and whose values come from runs and metric queries.
CREATE TABLE automation_apps (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name        TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    description TEXT NOT NULL DEFAULT '',
    tags        TEXT[] NOT NULL DEFAULT '{}',
    blueprint   TEXT NOT NULL DEFAULT '',
    -- {"components": [{"id", "kind", "label", "props": {...}}]}
    spec        JSONB NOT NULL,
    published   BOOLEAN NOT NULL DEFAULT false,
    created_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (org_id, name)
);
