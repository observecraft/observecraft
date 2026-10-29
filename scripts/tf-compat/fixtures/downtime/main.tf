# datadog_downtime (deprecated by the provider, still supported) against /api/v1/downtime.
resource "datadog_monitor" "load" {
  name    = "Billing load (tf-compat legacy downtimes)"
  type    = "metric alert"
  query   = "avg(last_5m):avg:system.load.1{service:billing} > 4"
  message = "Load is high @slack-billing"
  tags    = ["service:billing"]

  monitor_thresholds {
    critical = 4
  }
}

resource "datadog_downtime" "maintenance" {
  scope      = ["env:prod"]
  monitor_id = datadog_monitor.load.id
  start      = 1893456000 # 2030-01-01T00:00:00Z
  end        = 1893463200
  message    = "Billing maintenance @slack-billing"
}

resource "datadog_downtime" "weekdays" {
  scope        = ["env:staging"]
  monitor_tags = ["service:billing"]
  start        = 1893974400 # 2030-01-07T00:00:00Z, a Monday
  end          = 1893978000
  timezone     = "UTC"
  recurrence {
    type      = "weeks"
    period    = 1
    week_days = ["Mon", "Tue", "Wed", "Thu", "Fri"]
  }
}
