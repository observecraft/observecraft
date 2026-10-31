# A wider query.
resource "datadog_role" "billing_readers" {
  name = "tf-compat billing log readers"
}

resource "datadog_logs_restriction_query" "billing" {
  restriction_query = "service:(billing OR payment-service)"
  role_ids          = [datadog_role.billing_readers.id]
}
