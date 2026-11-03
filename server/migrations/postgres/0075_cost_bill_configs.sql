-- Cloud Cost Management's provider bills (Terraform `datadog_aws_cur_config`, `datadog_azure_uc_config`;
-- `/api/v2/cost/aws_cur_config`, `/api/v2/cost/azure_uc_config`): where an org's AWS Cost and Usage Report or
-- Azure cost exports are delivered. One row per cloud account: an Azure pair (its actual and amortized exports)
-- is one row. `id` is Datadog's numeric cloud account id; `uid` keys the import jobs (`cloud_crawl_jobs`).
-- `config` holds the provider's fields (AWS: bucket_name, bucket_region, report_name, report_prefix,
-- account_filters; Azure: client_id, actual / amortized: export_name, export_path, storage_account,
-- storage_container). Deleting archives a config: it leaves the API and stops importing; its costs stay.
CREATE TABLE IF NOT EXISTS cost_bill_configs (
    org_id            TEXT NOT NULL,
    id                BIGSERIAL NOT NULL,
    uid               UUID NOT NULL DEFAULT gen_random_uuid() UNIQUE,
    -- aws | azure
    provider          TEXT NOT NULL,
    -- The AWS (management) account, or the Azure tenant.
    account_id        TEXT NOT NULL,
    -- Azure: the exports' scope (subscription, resource group, billing account); AWS: ''.
    scope             TEXT NOT NULL DEFAULT '',
    config            JSONB NOT NULL DEFAULT '{}',
    months            INTEGER NOT NULL DEFAULT 15,
    -- What the import loaded: billing month (YYYY-MM) → the run of the bill it read (AWS assemblyId or
    -- executionId, Azure runId), so a month is read again only when the provider rewrites it.
    imported          JSONB NOT NULL DEFAULT '{}',
    -- active | disabled | archived
    status            TEXT NOT NULL DEFAULT 'active',
    status_updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, id)
);

CREATE UNIQUE INDEX IF NOT EXISTS cost_bill_configs_account ON cost_bill_configs (org_id, provider, account_id, scope) WHERE status <> 'archived';
