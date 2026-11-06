# datadog_integration_slack_channel (/api/v1/integration/slack/configuration/accounts/{account_name}/channels).
# The workspace (account "tf-compat") is connected by setup.sh, as the Slack integration page would.
resource "datadog_integration_slack_channel" "checkout" {
  account_name = "tf-compat"
  channel_name = "#checkout-alerts"
  display {
    message      = true
    notified     = false
    snapshot     = true
    tags         = true
    mute_buttons = false
  }
}
