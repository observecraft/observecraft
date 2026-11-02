# The service account updated: tags, resource collection, Cloud Run revisions filtered.
resource "datadog_integration_gcp_sts" "storefront" {
  client_email                = "observecraft-integration@example-project.iam.gserviceaccount.com"
  automute                    = false
  account_tags                = ["team:storefront", "env:staging"]
  resource_collection_enabled = true
  monitored_resource_configs = [
    { type = "gce_instance", filters = ["env:staging"] },
    { type = "cloud_run_revision", filters = ["service:checkout"] },
  ]
}
