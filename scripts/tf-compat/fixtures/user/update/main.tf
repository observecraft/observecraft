# Jordan renamed, moved to Read Only and disabled.
data "datadog_role" "standard" {
  filter = "Datadog Standard Role"
}

data "datadog_role" "read_only" {
  filter = "Datadog Read Only Role"
}

resource "datadog_user" "jordan" {
  email    = "tf-compat-jordan@example.com"
  name     = "Jordan Q. Lee"
  roles    = [data.datadog_role.read_only.id]
  disabled = true
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
}
