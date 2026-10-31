# A tighter threshold, a shorter window, and turned off.
resource "datadog_security_monitoring_rule" "console_brute_force" {
  name    = "tf-compat console brute force v2"
  message = "Many failed console sign-ins for {{@userIdentity.arn}}."
  enabled = false

  query {
    name            = "failures"
    query           = "source:cloudtrail @evt.name:ConsoleLogin @evt.outcome:failure"
    aggregation     = "count"
    group_by_fields = ["@userIdentity.arn"]
  }

  case {
    name      = "many"
    status    = "critical"
    condition = "failures > 10"
  }

  options {
    evaluation_window   = 600
    keep_alive          = 1800
    max_signal_duration = 86400
  }

  tags = ["team:security"]
}
