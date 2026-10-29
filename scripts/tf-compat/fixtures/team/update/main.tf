# Renamed, new description, the member is no longer a team admin.
resource "datadog_team" "checkout" {
  handle      = "tf-compat-checkout"
  name        = "Checkout & Payments"
  description = "Owns the storefront checkout, the order-service and billing."
}

resource "datadog_user" "member" {
  email                = "tf-compat-team-member@example.com"
  send_user_invitation = false
}

resource "datadog_team_membership" "member" {
  team_id = datadog_team.checkout.id
  user_id = datadog_user.member.id
}

data "datadog_team" "checkout" {
  team_id = datadog_team.checkout.id
}

output "team" {
  value = {
    handle  = data.datadog_team.checkout.handle
    summary = datadog_team.checkout.summary
  }
}
