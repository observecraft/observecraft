# A restriction query (POST /api/v2/logs/config/restriction_queries) granted to a role.
resource "datadog_role" "billing_readers" {
  name = "tf-compat billing log readers"
}

resource "datadog_logs_restriction_query" "billing" {
  restriction_query = "service:billing"
  role_ids          = [datadog_role.billing_readers.id]
}
