# datadog_downtime_schedule against /api/v2/downtime: a one-time downtime of one monitor and a
# weekday recurrence for the monitors carrying a tag.
resource "datadog_monitor" "cpu" {
  name    = "Storefront API CPU (tf-compat downtimes)"
  type    = "metric alert"
  query   = "avg(last_5m):avg:system.cpu.user{service:storefront-api} by {host} > 90"
  message = "CPU is high @slack-ops"
  tags    = ["service:storefront-api"]

  monitor_thresholds {
    critical = 90
  }
}

resource "datadog_downtime_schedule" "deploy" {
  scope   = "env:prod AND host:i-0a1b2c3d4e5f60001"
  message = "Deploying storefront-api @slack-ops"
  monitor_identifier {
    monitor_id = datadog_monitor.cpu.id
  }
  one_time_schedule {
    start = "2030-01-01T00:00:00Z"
    end   = "2030-01-01T02:00:00Z"
  }
}

resource "datadog_downtime_schedule" "weekday_backups" {
  scope = "env:staging"
  monitor_identifier {
    monitor_tags = ["service:inventory-service", "team:platform"]
  }
  recurring_schedule {
    timezone = "Europe/Paris"
    recurrence {
      duration = "2h"
      rrule    = "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR"
      start    = "2030-01-07T01:00:00"
    }
  }
  display_timezone                 = "Europe/Paris"
  mute_first_recovery_notification = true
  notify_end_states                = ["alert", "warn"]
  notify_end_types                 = ["expired"]
}

resource "datadog_downtime_schedule" "everything" {
  scope = "*"
  monitor_identifier {
    monitor_tags = ["*"]
  }
  one_time_schedule {
    start = "2031-06-01T00:00:00Z"
  }
}
