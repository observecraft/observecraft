# The app registration updated: new filters, custom metrics, every resource provider on.
resource "datadog_integration_azure" "storefront" {
  tenant_name            = "11111111-2222-3333-4444-555555555555"
  client_id              = "66666666-7777-8888-9999-000000000000"
  client_secret          = "tf-compat-secret"
  host_filters           = "env:staging"
  container_app_filters  = "team:checkout"
  automute               = false
  custom_metrics_enabled = true
}
