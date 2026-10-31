# A custom destination (POST /api/v2/logs/config/custom-destinations): HTTP with basic auth.
resource "datadog_logs_custom_destination" "siem" {
  name    = "tf-compat siem"
  query   = "service:checkout-service status:error"
  enabled = false

  forward_tags                       = true
  forward_tags_restriction_list      = ["env", "service"]
  forward_tags_restriction_list_type = "ALLOW_LIST"

  http_destination {
    endpoint = "https://siem.example.com/ingest"
    basic_auth {
      username = "observecraft"
      password = "example-password"
    }
  }
}
