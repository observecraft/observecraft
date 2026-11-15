-- Tagging policies on the cloud resources the integrations' crawlers already know (routes/provenance.rs), not only
-- those the account scans evaluated: the AWS crawler's tagged resources (Resource Groups Tagging API) and the Azure
-- crawler's subscription resources, with their tags, written at most once an hour per job (cloud/resources.rs).
CREATE TABLE IF NOT EXISTS cloud_resources (
    org_id        TEXT NOT NULL,
    provider      TEXT NOT NULL,                     -- aws | azure
    resource_id   TEXT NOT NULL,                     -- the ARN, or the Azure resource id (lower case)
    account_id    TEXT NOT NULL,                     -- AWS account id, Azure subscription id
    name          TEXT NOT NULL DEFAULT '',
    resource_type TEXT NOT NULL DEFAULT '',          -- AwsS3Bucket, Microsoft.Compute/virtualMachines
    region        TEXT NOT NULL DEFAULT '',
    tags          JSONB NOT NULL DEFAULT '[]',       -- ["key:value", …]
    last_seen     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, provider, resource_id)
);
CREATE INDEX IF NOT EXISTS cloud_resources_seen ON cloud_resources (org_id, last_seen);

-- The recommended Argo CD tracking policy leaves out what the managed Kubernetes services run themselves
-- (tagging_rules.rs PROVIDER_MANAGED_NAMESPACES / _LABELS); policies installed before keep their scope and gain the
-- exclusions unless they already set some.
UPDATE tagging_policies SET scope = scope || jsonb_build_object(
        'excludeNamespaces', 'kube-system, kube-public, kube-node-lease, gatekeeper-system, gke-managed-*, gmp-system, gmp-public',
        'excludeLabels', 'kubernetes.azure.com/managedby, addonmanager.kubernetes.io/mode, eks.amazonaws.com/component, components.gke.io/component-name'),
    updated_at = now()
WHERE recommended = 'kubernetes_argocd_tracking' AND COALESCE(scope->>'excludeNamespaces', '') = '' AND COALESCE(scope->>'excludeLabels', '') = '';
