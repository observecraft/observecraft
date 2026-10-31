# A log detection rule (POST /api/v2/security_monitoring/rules): failed console sign-ins per user.
resource "datadog_security_monitoring_rule" "console_brute_force" {
  name    = "tf-compat console brute force"
  message = "Many failed console sign-ins for {{@userIdentity.arn}}."
  enabled = true

  query {
    name            = "failures"
    query           = "source:cloudtrail @evt.name:ConsoleLogin @evt.outcome:failure"
    aggregation     = "count"
    group_by_fields = ["@userIdentity.arn"]
  }

  case {
    name      = "many"
    status    = "high"
    condition = "failures > 25"
  }

  case {
    name      = "some"
    status    = "medium"
    condition = "failures > 5"
  }

  options {
    evaluation_window   = 900
    keep_alive          = 3600
    max_signal_duration = 86400
  }

  tags = ["team:security", "tactic:TA0006-credential-access"]
}
