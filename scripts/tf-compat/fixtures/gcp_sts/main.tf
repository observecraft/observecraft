# A GCP service account impersonated through this install's STS delegate (POST /api/v2/integration/gcp/accounts).
resource "datadog_integration_gcp_sts" "storefront" {
  client_email = "observecraft-integration@example-project.iam.gserviceaccount.com"
  automute     = true
  account_tags = ["team:storefront"]
  monitored_resource_configs = [
    { type = "gce_instance", filters = ["env:prod"] },
  ]
}
