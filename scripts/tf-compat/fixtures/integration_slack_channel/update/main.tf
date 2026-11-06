# Renamed, with mute buttons and without the message body.
resource "datadog_integration_slack_channel" "checkout" {
  account_name = "tf-compat"
  channel_name = "#checkout-incidents"
  display {
    message      = false
    notified     = true
    snapshot     = false
    tags         = true
    mute_buttons = true
  }
}
