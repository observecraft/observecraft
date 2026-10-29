# datadog_monitor (metric, APM and log monitors), datadog_monitor_json and the datadog_monitor(s)
# data sources against /api/v1/monitor. The provider validates each monitor while planning
# (POST /api/v1/monitor/validate, /api/v1/monitor/{id}/validate).
resource "datadog_monitor" "cpu" {
  name    = "Storefront API CPU (tf-compat)"
  type    = "metric alert"
  query   = "avg(last_5m):avg:system.cpu.user{service:storefront-api} by {host} > 90"
  message = "CPU is high on {{host.name}} @slack-ops @oncall@example.com"
  tags    = ["env:prod", "service:storefront-api", "tf-compat:monitor"]

  priority = 2
  monitor_thresholds {
    critical          = 90
    warning           = 80
    critical_recovery = 85
  }
  notify_no_data      = true
  no_data_timeframe   = 20
  renotify_interval   = 60
  escalation_message  = "Still high @pagerduty-storefront"
  evaluation_delay    = 60
  new_group_delay     = 120
  require_full_window = false
  include_tags        = true
  notify_by           = ["host"]
}

resource "datadog_monitor" "latency" {
  name    = "Billing latency change (tf-compat)"
  type    = "query alert"
  query   = "pct_change(avg(last_10m),last_1h):avg:trace.http.request.duration{service:billing} > 50"
  message = "Billing got slower @slack-billing"
  tags    = ["service:billing", "tf-compat:monitor"]

  monitor_thresholds {
    critical = 50
  }
}

resource "datadog_monitor" "checkout_errors" {
  name    = "Checkout errors (tf-compat)"
  type    = "log alert"
  query   = "logs(\"service:order-service status:error\").index(\"*\").rollup(\"count\").by(\"service\").last(\"10m\") > 50"
  message = "{{#is_alert}}Many errors in {{service.name}}{{/is_alert}} @slack-checkout"
  tags    = ["service:order-service", "tf-compat:monitor"]

  monitor_thresholds {
    critical = 50
    warning  = 20
  }
  enable_logs_sample = true
}

resource "datadog_monitor_json" "load" {
  monitor = jsonencode({
    name     = "Inventory load (tf-compat)"
    type     = "metric alert"
    query    = "avg(last_15m):avg:system.load.1{service:inventory-service} > 4"
    message  = "Load is high @slack-ops"
    tags     = ["service:inventory-service", "tf-compat:monitor"]
    priority = 3
    options = {
      thresholds        = { critical = 4, warning = 3 }
      notify_no_data    = false
      notify_audit      = false
      new_host_delay    = 300
      include_tags      = true
      renotify_interval = 0
    }
  })
}

data "datadog_monitor" "cpu" {
  name_filter = "Storefront API CPU (tf-compat)"
  depends_on  = [datadog_monitor.cpu]
}

data "datadog_monitors" "tagged" {
  monitor_tags_filter = ["tf-compat:monitor"]
  depends_on          = [datadog_monitor.cpu, datadog_monitor.latency, datadog_monitor.checkout_errors, datadog_monitor_json.load]

  lifecycle {
    postcondition {
      condition     = length(self.monitors) == 4
      error_message = "GET /api/v1/monitor?monitor_tags=tf-compat:monitor should list the fixture's 4 monitors"
    }
  }
}

output "cpu_monitor" {
  value = {
    id    = data.datadog_monitor.cpu.id
    query = data.datadog_monitor.cpu.query
  }
}
