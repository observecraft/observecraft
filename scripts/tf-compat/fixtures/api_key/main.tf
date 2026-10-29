# An ingest key (POST /api/v2/api_keys): the secret comes once, on create, and stays in the state.
resource "datadog_api_key" "agents" {
  name = "tf-compat storefront agents"

  lifecycle {
    postcondition {
      condition     = startswith(self.key, "ocik_")
      error_message = "datadog_api_key should hold the ingest key returned on create"
    }
  }
}
