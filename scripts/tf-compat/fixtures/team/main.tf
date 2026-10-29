# A team (POST /api/v2/team) with an admin member, and the team data source.
resource "datadog_team" "checkout" {
  handle      = "tf-compat-checkout"
  name        = "Checkout"
  description = "Owns the storefront checkout and the order-service."
}

resource "datadog_user" "member" {
  email                = "tf-compat-team-member@example.com"
  send_user_invitation = false
}

resource "datadog_team_membership" "member" {
  team_id = datadog_team.checkout.id
  user_id = datadog_user.member.id
  role    = "admin"
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
