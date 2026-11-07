-- Integrations → FleetDM: an org's connection to a Fleet server (fleetdm.com, self-hosted or Fleet's cloud), polled by
-- the server (`crate::fleetdm`), which sends its activities (logins, policies, queries, scripts, software, MDM and host
-- changes) as logs (`source:fleetdm`) and its hosts and policies as `fleetdm.*` metrics. One connection per org; its
-- `id` keys the poller's jobs (`cloud_crawl_jobs`, provider `fleetdm`, scopes `activities` and `hosts`; the activities
-- job's `collected_until` is their cursor). The API token is sealed.
CREATE TABLE IF NOT EXISTS fleetdm_connections (
    org_id     TEXT NOT NULL PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
    id         UUID NOT NULL DEFAULT gen_random_uuid() UNIQUE,
    -- The Fleet server's URL (https://fleet.example.com).
    base_url   TEXT NOT NULL,
    token_enc  TEXT NOT NULL,
    -- The admin who saved it last (a user or an application key's owner).
    updated_by UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
