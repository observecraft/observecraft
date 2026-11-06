-- Integrations → authentik: an org's connection to an authentik instance, polled by the server
-- (`crate::authentik`), which sends the instance's events (logins, access requests, users and models changed,
-- policy and system errors) as logs (`source:authentik`). One connection per org; its `id` keys the poller's
-- job (`cloud_crawl_jobs`, provider `authentik`, scope `events`, whose `collected_until` is the events cursor).
-- The API token is sealed.
CREATE TABLE IF NOT EXISTS authentik_connections (
    org_id     TEXT NOT NULL PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
    id         UUID NOT NULL DEFAULT gen_random_uuid() UNIQUE,
    -- The instance's URL (https://auth.example.com).
    base_url   TEXT NOT NULL,
    token_enc  TEXT NOT NULL,
    -- The admin who saved it last (a user or an application key's owner).
    updated_by UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
