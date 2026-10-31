# Orders before payments.
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
  filter_ids = ["errors-default", "appsec-default", datadog_apm_retention_filter.orders.id, datadog_apm_retention_filter.payments.id]
}
