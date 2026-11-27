-- Vanta (Integrations → Vanta, crate::vanta): an org's private Vanta app, to which the server sends its compliance
-- evidence every hour on the cloud crawlers' jobs (provider `vanta`).
CREATE TABLE vanta_connections (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id            TEXT NOT NULL UNIQUE REFERENCES orgs(id) ON DELETE CASCADE,
    -- https://api.vanta.com or https://api.vanta-gov.com
    api_url           TEXT NOT NULL,
    client_id         TEXT NOT NULL,
    -- AES-256-GCM(OC_SECRET_KEY)
    client_secret_enc TEXT NOT NULL,
    -- The Developer Console's resourceId of each resource type to send ({"user_account": "…"}).
    resource_ids      JSONB NOT NULL DEFAULT '{}',
    updated_by        UUID,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Each resource type's last sync.
CREATE TABLE vanta_sync_results (
    connection_id UUID NOT NULL REFERENCES vanta_connections(id) ON DELETE CASCADE,
    resource_type TEXT NOT NULL,
    synced_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    sent          INTEGER NOT NULL DEFAULT 0,
    accepted      INTEGER NOT NULL DEFAULT 0,
    rejected      INTEGER NOT NULL DEFAULT 0,
    -- Why the type wasn't sent, or what was left out (the 10,000 cap).
    error         TEXT,
    PRIMARY KEY (connection_id, resource_type)
);
