-- Azure integration app registrations (Terraform `datadog_integration_azure`, `/api/v1/integration/azure`).
-- One row per app registration (tenant + client id) of an org; the Integrations → Azure page reads and
-- writes the same rows. `config` is the API's other fields, normalized. The client secret is sealed with
-- the server's secret key.
CREATE TABLE IF NOT EXISTS azure_accounts (
    org_id            TEXT NOT NULL,
    id                UUID NOT NULL DEFAULT gen_random_uuid(),
    tenant_name       TEXT NOT NULL,
    client_id         TEXT NOT NULL,
    client_secret_enc TEXT,
    config            JSONB NOT NULL DEFAULT '{}',
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    modified_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, id),
    UNIQUE (org_id, tenant_name, client_id)
);
