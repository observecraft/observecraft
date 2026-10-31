# A log index (POST /api/v1/logs/config/indexes) with a daily limit and an exclusion filter.
resource "datadog_logs_index" "checkout" {
  name           = "tf-compat-checkout"
  retention_days = 7
  daily_limit    = 500000

  daily_limit_reset {
    reset_time       = "14:00"
    reset_utc_offset = "+02:00"
  }
  daily_limit_warning_threshold_percentage = 80

  filter {
    query = "service:checkout-service"
  }

  exclusion_filter {
    name       = "debug logs"
    is_enabled = true
    filter {
      query       = "status:debug"
      sample_rate = 0.9
    }
  }
}
