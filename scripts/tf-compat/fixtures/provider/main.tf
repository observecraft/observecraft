# The provider starts, validates its credentials (GET /api/v1/validate with api_key + app_key,
# GET /api/v2/validate_keys with bearer_token) and acts as the application key's user.
# A data source is needed: OpenTofu configures a provider only when something uses it.
data "datadog_current_user" "me" {
  lifecycle {
    postcondition {
      condition     = self.email != "" && self.org_id != ""
      error_message = "GET /api/v2/current_user did not return the application key's user and org"
    }
  }
}

output "user" {
  value = {
    email = data.datadog_current_user.me.email
    org   = data.datadog_current_user.me.org_id
  }
}
