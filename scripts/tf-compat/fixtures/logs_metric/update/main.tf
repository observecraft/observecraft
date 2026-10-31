# Another filter and group-by; percentiles off.
resource "datadog_logs_metric" "checkout_latency" {
  name = "tf_compat.checkout.latency"
  compute {
    aggregation_type    = "distribution"
    path                = "@duration"
    include_percentiles = false
  }
  filter {
    query = "service:checkout-service env:prod"
  }
  group_by {
    path     = "@http.method"
    tag_name = "method"
  }
}
