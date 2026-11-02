# An Azure app registration (POST /api/v1/integration/azure): filters, automute, one resource provider off.
resource "datadog_integration_azure" "storefront" {
  tenant_name              = "11111111-2222-3333-4444-555555555555"
  client_id                = "66666666-7777-8888-9999-000000000000"
  client_secret            = "tf-compat-secret"
  host_filters             = "env:prod"
  app_service_plan_filters = "team:storefront"
  automute                 = true
  resource_provider_configs = [
    { namespace = "Microsoft.Compute", metrics_enabled = true },
    { namespace = "Microsoft.Web", metrics_enabled = false },
  ]
}
