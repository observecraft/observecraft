-- Connector packages (docs/adr/0007-connectors.md): an org's connections to the vendor APIs packages poll
-- (crates/oc-api/src/connectors/). A connection has its configuration (the package's non-secret fields), its secrets
-- (each field sealed on its own, never answered) and the streams it collects; its id keys the jobs
-- (`cloud_crawl_jobs`, provider = connector id, scope = stream id).
CREATE TABLE IF NOT EXISTS connector_connections (
    id           UUID NOT NULL DEFAULT gen_random_uuid() PRIMARY KEY,
    org_id       TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    connector_id TEXT NOT NULL,
    name         TEXT NOT NULL,
    config       JSONB NOT NULL DEFAULT '{}',
    -- Field → sealed value.
    secrets      JSONB NOT NULL DEFAULT '{}',
    -- The streams collected (ids of the package's streams).
    streams      TEXT[] NOT NULL DEFAULT '{}',
    enabled      BOOLEAN NOT NULL DEFAULT true,
    -- The admin who saved it last (a user or an application key's owner).
    updated_by   UUID,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (org_id, connector_id, name)
);

CREATE INDEX IF NOT EXISTS connector_connections_connector ON connector_connections (connector_id);

-- Each stream's state: cursors, checkpoints, the backlog of a window not read to its end, the ids already sent
-- (oc_connectors::engine::StreamState). Written only after the run's records were stored.
CREATE TABLE IF NOT EXISTS connector_stream_state (
    connection_id UUID NOT NULL REFERENCES connector_connections(id) ON DELETE CASCADE,
    org_id        TEXT NOT NULL,
    stream        TEXT NOT NULL,
    state         JSONB NOT NULL DEFAULT '{}',
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (connection_id, stream)
);

-- Request quotas (a package's `budget`): requests used in the current period, for the connection ('') or a stream.
CREATE TABLE IF NOT EXISTS connector_request_budgets (
    connection_id UUID NOT NULL REFERENCES connector_connections(id) ON DELETE CASCADE,
    org_id        TEXT NOT NULL,
    scope         TEXT NOT NULL,
    period_start  TIMESTAMPTZ NOT NULL,
    used          INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (connection_id, scope)
);

-- NetBird moves from its own table to its package (integrations/connectors/netbird): the same ids, so its jobs
-- (provider `netbird`, scopes `events` and `peers`) carry on, and the events job's watermark becomes the stream's
-- cursor. The sealed token is kept as it is.
INSERT INTO connector_connections (id, org_id, connector_id, name, config, secrets, streams, updated_by, created_at, updated_at)
SELECT id, org_id, 'netbird', regexp_replace(base_url, '^https?://([^/:]+).*$', '\1'), jsonb_build_object('base_url', base_url),
       jsonb_build_object('token', token_enc), ARRAY['events', 'peers'], updated_by, created_at, updated_at
FROM netbird_connections
ON CONFLICT DO NOTHING;

INSERT INTO connector_stream_state (connection_id, org_id, stream, state)
SELECT j.account_id, j.org_id, 'events',
       jsonb_build_object('partitions', jsonb_build_object('', jsonb_build_object('cursor', (extract(epoch FROM j.collected_until) * 1000)::bigint)))
FROM cloud_crawl_jobs j JOIN connector_connections c ON c.id = j.account_id
WHERE j.provider = 'netbird' AND j.scope = 'events' AND j.collected_until IS NOT NULL
ON CONFLICT DO NOTHING;

DROP TABLE IF EXISTS netbird_connections;
