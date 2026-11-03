-- Assist Agent Builder: custom agents, each with its own instructions and skills (groups of tools),
-- run by hand or on a schedule as the member who owns them. Their runs are assist_runs of kind 'agent'.
CREATE TABLE IF NOT EXISTS assist_agents (
    org_id        TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    id            UUID NOT NULL DEFAULT gen_random_uuid(),
    name          TEXT NOT NULL,
    description   TEXT NOT NULL DEFAULT '',
    instructions  TEXT NOT NULL,
    skills        TEXT[] NOT NULL DEFAULT '{}',
    -- May use tools that change something (create a monitor, declare an incident) without asking.
    allow_actions BOOLEAN NOT NULL DEFAULT false,
    tags          TEXT[] NOT NULL DEFAULT '{}',
    -- Five-field cron in `timezone`; none: run by hand only.
    schedule      TEXT,
    timezone      TEXT NOT NULL DEFAULT 'UTC',
    enabled       BOOLEAN NOT NULL DEFAULT true,
    next_run_at   TIMESTAMPTZ,
    owner         UUID REFERENCES users(id) ON DELETE SET NULL,
    created_by    TEXT NOT NULL DEFAULT '',
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, id)
);
CREATE INDEX IF NOT EXISTS assist_agents_due ON assist_agents (next_run_at) WHERE enabled AND schedule IS NOT NULL;
