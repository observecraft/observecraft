-- Integrations → NetBird: an org's connection to a NetBird management API (NetBird Cloud or self-hosted), polled by
-- the server (`crate::netbird`), which sends the account's audit events (peers added, policies, routes, setup keys,
-- users and logins changed) as logs (`source:netbird`) and its peers' state as `netbird.peer.*` metrics. One
-- connection per org; its `id` keys the poller's jobs (`cloud_crawl_jobs`, provider `netbird`, scopes `events` and
-- `peers`; the events job's `collected_until` is the events cursor). The personal access token is sealed.
CREATE TABLE IF NOT EXISTS netbird_connections (
    org_id     TEXT NOT NULL PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
    id         UUID NOT NULL DEFAULT gen_random_uuid() UNIQUE,
    -- The management API's URL (https://api.netbird.io, https://netbird.example.com).
    base_url   TEXT NOT NULL,
    token_enc  TEXT NOT NULL,
    -- The admin who saved it last (a user or an application key's owner).
    updated_by UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
