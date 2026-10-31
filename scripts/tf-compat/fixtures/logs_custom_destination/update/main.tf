# Splunk HEC instead, with a source type.
resource "datadog_logs_custom_destination" "siem" {
  name    = "tf-compat siem"
  query   = "service:checkout-service"
  enabled = false

  forward_tags = false

  splunk_destination {
    endpoint     = "https://splunk.example.com:8088"
    access_token = "example-token"
    sourcetype {
      value = "observecraft:logs"
    }
  }
}
