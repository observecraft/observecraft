# Renamed: the key keeps its value.
resource "datadog_api_key" "agents" {
  name = "tf-compat storefront agents (eu)"

  lifecycle {
    postcondition {
      condition     = startswith(self.key, "ocik_")
      error_message = "datadog_api_key should still hold the ingest key returned on create"
    }
  }
}
