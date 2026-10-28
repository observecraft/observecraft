-- Log pipelines (Logs → Configuration → Pipelines): the indexer runs each org's enabled pipelines,
-- in order, on every log whose filter matches, before index routing. Processors are JSON in
-- Datadog's pipeline format (grok parser, remappers, category processor, …).

CREATE TABLE log_pipelines (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    -- Processing order (lowest first).
    position    INTEGER NOT NULL,
    name        TEXT NOT NULL,
    enabled     BOOLEAN NOT NULL DEFAULT true,
    -- Search syntax; '' = every log.
    filter      TEXT NOT NULL DEFAULT '',
    -- [{ "type": "grok-parser", "name", "enabled", … }], run in order.
    processors  JSONB NOT NULL DEFAULT '[]',
    -- 'custom', or 'integration:<name>' for a pipeline installed from an integration (read-only
    -- apart from its enabled flag and position; clone it to edit).
    source      TEXT NOT NULL DEFAULT 'custom',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX log_pipelines_org ON log_pipelines (org_id, position);
-- An integration's pipeline is installed once per org.
CREATE UNIQUE INDEX log_pipelines_integration ON log_pipelines (org_id, source) WHERE source <> 'custom';
