-- Display names of the scopes a cloud account's discovery finds: Azure subscriptions ("Storefront
-- Production" for a subscription GUID) and GCP projects. Lists of cloud accounts show them
-- (Cloud Security's summary).
CREATE TABLE IF NOT EXISTS cloud_scope_names (
    org_id     TEXT NOT NULL,
    provider   TEXT NOT NULL,                      -- azure | gcp
    scope      TEXT NOT NULL,                      -- subscription id | project id
    name       TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, provider, scope)
);
