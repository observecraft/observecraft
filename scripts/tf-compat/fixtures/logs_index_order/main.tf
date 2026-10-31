# The routing order of every index (PUT /api/v1/logs/config/index-order): the new one last.
resource "datadog_logs_index" "payments" {
  name           = "tf-compat-payments"
  retention_days = 7
  filter {
    query = "service:payment-service"
  }
}

data "datadog_logs_indexes" "all" {
  depends_on = [datadog_logs_index.payments]
}

resource "datadog_logs_index_order" "order" {
  name = "tf-compat-order"
  indexes = concat(
    [for i in data.datadog_logs_indexes.all.logs_indexes : i.name if i.name != datadog_logs_index.payments.name],
    [datadog_logs_index.payments.name],
  )
}
