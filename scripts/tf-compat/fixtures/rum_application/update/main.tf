# Renamed and moved to error-focused event processing.
resource "datadog_rum_application" "storefront" {
  name                       = "tf-compat-storefront-web-v2"
  type                       = "browser"
  rum_event_processing_state = "ERROR_FOCUSED_MODE"
}

data "datadog_rum_application" "storefront" {
  id = datadog_rum_application.storefront.id
}
