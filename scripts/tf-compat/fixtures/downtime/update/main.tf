# The fixture's legacy downtimes after an update (PUT /api/v1/downtime/{id}).
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
  scope      = ["env:prod", "host:i-0a1b2c3d4e5f60002"]
  monitor_id = datadog_monitor.load.id
  start      = 1893456000
  end        = 1893470400
  message    = "Billing maintenance, longer @slack-billing"
}

resource "datadog_downtime" "weekdays" {
  scope        = ["env:staging"]
  monitor_tags = ["service:billing"]
  start        = 1893974400
  end          = 1893981600
  timezone     = "UTC"
  recurrence {
    type       = "weeks"
    period     = 1
    week_days  = ["Mon", "Wed", "Fri"]
    until_date = 1924905600 # 2030-12-31
  }
}
