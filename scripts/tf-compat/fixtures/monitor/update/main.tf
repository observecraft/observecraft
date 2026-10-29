# The fixture's monitors after an update (PUT /api/v1/monitor/{id}).
resource "datadog_monitor" "cpu" {
  name    = "Storefront API CPU, all hosts (tf-compat)"
  type    = "metric alert"
  query   = "max(last_10m):avg:system.cpu.user{service:storefront-api} by {host} > 95"
  message = "CPU is very high on {{host.name}} @slack-ops"
  tags    = ["env:prod", "service:storefront-api", "team:storefront", "tf-compat:monitor"]

  priority = 1
  monitor_thresholds {
    critical = 95
    warning  = 85
  }
  notify_no_data      = false
  renotify_interval   = 30
  evaluation_delay    = 120
  require_full_window = false
  include_tags        = false
}

resource "datadog_monitor" "latency" {
  name    = "Billing latency change (tf-compat)"
  type    = "query alert"
  query   = "pct_change(avg(last_10m),last_1d):avg:trace.http.request.duration{service:billing} > 75"
  message = "Billing got slower than yesterday @slack-billing"
  tags    = ["service:billing", "tf-compat:monitor"]

  monitor_thresholds {
    critical = 75
    warning  = 50
  }
  draft_status = "draft"
}

resource "datadog_monitor" "checkout_errors" {
  name    = "Checkout errors (tf-compat)"
  type    = "log alert"
  query   = "logs(\"service:order-service status:error\").index(\"*\").rollup(\"count\").by(\"service,host\").last(\"15m\") > 100"
  message = "{{#is_alert}}Many errors in {{service.name}}{{/is_alert}} @slack-checkout"
  tags    = ["service:order-service", "tf-compat:monitor"]

  monitor_thresholds {
    critical = 100
  }
  enable_logs_sample = false
}

resource "datadog_monitor_json" "load" {
  monitor = jsonencode({
    name     = "Inventory load (tf-compat)"
    type     = "metric alert"
    query    = "avg(last_15m):avg:system.load.1{service:inventory-service} > 6"
    message  = "Load is high @slack-ops"
    tags     = ["service:inventory-service", "tf-compat:monitor"]
    priority = 2
    options = {
      thresholds        = { critical = 6, warning = 4 }
      notify_no_data    = true
      notify_audit      = false
      new_host_delay    = 300
      include_tags      = true
      renotify_interval = 60
    }
  })
}

data "datadog_monitor" "cpu" {
  name_filter = "Storefront API CPU, all hosts (tf-compat)"
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
