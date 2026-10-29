# Users created as invitations (POST /api/v2/users + /api/v2/user_invitations) with roles, a role
# given through datadog_user_role, and the user data sources. Re-applying after a destroy (the second
# authentication run) hits the disabled user's email: the provider re-enables it (409 → PATCH).
data "datadog_role" "standard" {
  filter = "Datadog Standard Role"
}

resource "datadog_user" "jordan" {
  email = "tf-compat-jordan@example.com"
  name  = "Jordan Lee"
  roles = [data.datadog_role.standard.id]
}

resource "datadog_user" "sam" {
  email                = "tf-compat-sam@example.com"
  send_user_invitation = false
}

resource "datadog_role" "on_call" {
  name = "tf-compat on-call"
}

resource "datadog_user_role" "sam_on_call" {
  role_id = datadog_role.on_call.id
  user_id = datadog_user.sam.id
}

data "datadog_user" "jordan" {
  filter      = datadog_user.jordan.email
  exact_match = true
}

data "datadog_users" "pending" {
  filter_status = "Pending"
  depends_on    = [datadog_user.jordan, datadog_user.sam]
}

output "users" {
  value = {
    jordan  = data.datadog_user.jordan.status
    pending = length(data.datadog_users.pending.users)
  }

  precondition {
    condition     = datadog_user.jordan.user_invitation_id != ""
    error_message = "datadog_user should have sent an invitation"
  }
}
