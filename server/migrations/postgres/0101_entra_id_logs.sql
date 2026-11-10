-- Integrations → Microsoft Entra ID: the Azure integration's app registrations (`azure_accounts`) whose tenant's
-- sign-in, audit and risk detection logs the server reads from Microsoft Graph (`crate::entra`) and sends as logs
-- shaped as Azure's diagnostic settings export them (`source:azure.activedirectory`). A row is an app registration
-- with Entra ID logs on; each stream on is a poller job (`cloud_crawl_jobs`, provider `entra`, scope `signins`,
-- `audits` or `risk`, whose `collected_until` is the stream's cursor). Removed with the app registration.
CREATE TABLE IF NOT EXISTS entra_log_settings (
    org_id     TEXT NOT NULL,
    account_id UUID NOT NULL,
    sign_ins   BOOLEAN NOT NULL DEFAULT true,
    audits     BOOLEAN NOT NULL DEFAULT true,
    -- Identity Protection's risk detections (Entra ID P2, IdentityRiskEvent.Read.All).
    risk       BOOLEAN NOT NULL DEFAULT false,
    updated_by UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, account_id),
    FOREIGN KEY (org_id, account_id) REFERENCES azure_accounts (org_id, id) ON DELETE CASCADE
);

-- The poller's state per stream, beside its cursor: the ids of the events it sent from the overlap it reads again
-- (Graph makes events visible minutes after their time), so each is sent once (id → event time, epoch ms).
CREATE TABLE IF NOT EXISTS entra_log_state (
    org_id     TEXT NOT NULL,
    account_id UUID NOT NULL,
    stream     TEXT NOT NULL,
    seen       JSONB NOT NULL DEFAULT '{}',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, account_id, stream),
    FOREIGN KEY (org_id, account_id) REFERENCES azure_accounts (org_id, id) ON DELETE CASCADE
);
