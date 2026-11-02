-- GCP integration service accounts (Terraform `datadog_integration_gcp_sts`, `/api/v2/integration/gcp/accounts`).
-- One row per customer service account of an org, which this install impersonates with its own service
-- account (`OC_GCP_PRINCIPAL_EMAIL`, the STS delegate): no key is stored. `config` is the API's other
-- attributes, normalized. The Integrations → Google Cloud page reads and writes the same rows.
CREATE TABLE IF NOT EXISTS gcp_accounts (
    org_id       TEXT NOT NULL,
    id           UUID NOT NULL DEFAULT gen_random_uuid(),
    client_email TEXT NOT NULL,
    config       JSONB NOT NULL DEFAULT '{}',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    modified_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, id),
    UNIQUE (org_id, client_email)
);
