# The execution order of every retention filter of the org (PUT
# /api/v2/apm/config/retention-filters-execution-order): the two defaults, then ours.
resource "datadog_apm_retention_filter" "payments" {
  name        = "tf-compat payment errors"
  rate        = "1.0"
  filter_type = "spans-sampling-processor"
  enabled     = true
  filter {
    query = "service:billing status:error"
  }
}

resource "datadog_apm_retention_filter" "orders" {
  name        = "tf-compat orders"
  rate        = "0.1"
  filter_type = "spans-sampling-processor"
  enabled     = true
  filter {
    query = "service:order-service"
  }
}

resource "datadog_apm_retention_filter_order" "order" {
  filter_ids = ["errors-default", "appsec-default", datadog_apm_retention_filter.payments.id, datadog_apm_retention_filter.orders.id]
}
