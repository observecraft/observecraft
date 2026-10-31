# A log-based distribution (POST /api/v2/logs/config/metrics).
resource "datadog_logs_metric" "checkout_latency" {
  name = "tf_compat.checkout.latency"
  compute {
    aggregation_type    = "distribution"
    path                = "@duration"
    include_percentiles = true
  }
  filter {
    query = "service:checkout-service"
  }
  group_by {
    path     = "@http.status_code"
    tag_name = "status_code"
  }
}
