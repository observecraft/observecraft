# Renamed: the key keeps its value.
resource "datadog_application_key" "ci" {
  name = "tf-compat ci (nightly)"

  lifecycle {
    postcondition {
      condition     = startswith(self.key, "ocak_")
      error_message = "datadog_application_key should still hold the application key returned on create"
    }
  }
}
