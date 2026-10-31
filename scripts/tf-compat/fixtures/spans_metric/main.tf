# A span-based metric (POST /api/v2/apm/config/metrics): checkout latency per resource.
resource "datadog_spans_metric" "checkout_latency" {
  name = "tf_compat.storefront.checkout.duration"
  compute {
    aggregation_type    = "distribution"
    include_percentiles = false
    path                = "@duration"
  }
  filter {
    query = "service:storefront-api @http.status_code:200"
  }
  group_by {
    path     = "resource_name"
    tag_name = "resource_name"
  }
}
