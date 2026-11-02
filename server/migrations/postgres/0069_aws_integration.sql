-- AWS integration accounts (Terraform `datadog_integration_aws_account`,
-- `/api/v2/integration/aws/accounts`). One row per AWS account of an org; the Integrations → AWS page
-- reads and writes the same rows. Each configuration section is stored as the API's normalized
-- document. With key authentication, the secret access key is sealed with the server's secret key.
CREATE TABLE IF NOT EXISTS aws_accounts (
    org_id                TEXT NOT NULL,
    id                    UUID NOT NULL DEFAULT gen_random_uuid(),
    aws_account_id        TEXT NOT NULL,
    aws_partition         TEXT NOT NULL DEFAULT 'aws',
    account_tags          JSONB NOT NULL DEFAULT '[]',
    -- role: role_name + external_id; keys: access_key_id + secret_access_key_enc
    role_name             TEXT,
    external_id           TEXT,
    access_key_id         TEXT,
    secret_access_key_enc TEXT,
    aws_regions           JSONB NOT NULL DEFAULT '{"include_all": true}',
    logs_config           JSONB NOT NULL DEFAULT '{}',
    metrics_config        JSONB NOT NULL DEFAULT '{}',
    resources_config      JSONB NOT NULL DEFAULT '{}',
    traces_config         JSONB NOT NULL DEFAULT '{}',
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    modified_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, id),
    UNIQUE (org_id, aws_account_id)
);
