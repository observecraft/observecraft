# Every status, with percentiles, per resource and environment.
resource "datadog_spans_metric" "checkout_latency" {
  name = "tf_compat.storefront.checkout.duration"
  compute {
    aggregation_type    = "distribution"
    include_percentiles = true
    path                = "@duration"
  }
  filter {
    query = "service:storefront-api"
  }
  group_by {
    path     = "resource_name"
    tag_name = "resource_name"
  }
  group_by {
    path     = "env"
    tag_name = "env"
  }
}
