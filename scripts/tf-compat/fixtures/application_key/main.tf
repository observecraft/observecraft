# An application key of the provider's user (POST /api/v2/current_user/application_keys).
resource "datadog_application_key" "ci" {
  name = "tf-compat ci"

  lifecycle {
    postcondition {
      condition     = startswith(self.key, "ocak_")
      error_message = "datadog_application_key should hold the application key returned on create"
    }
  }
}
