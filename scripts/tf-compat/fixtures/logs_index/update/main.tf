# Longer standard retention, a flex tier, no daily limit and a sampled exclusion.
resource "datadog_logs_index" "checkout" {
  name                = "tf-compat-checkout"
  retention_days      = 15
  flex_retention_days = 90
  disable_daily_limit = true

  filter {
    query = "service:(checkout-service OR order-service)"
  }

  exclusion_filter {
    name       = "debug logs"
    is_enabled = false
    filter {
      query            = "status:debug"
      sample_rate      = 0.5
      sample_attribute = "@request_id"
    }
  }
}
