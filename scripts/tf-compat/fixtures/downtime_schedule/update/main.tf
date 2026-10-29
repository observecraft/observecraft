# The fixture's downtimes after an update (PATCH /api/v2/downtime/{id}).
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
  scope   = "env:prod"
  message = "Deploying storefront-api, all hosts @slack-ops"
  monitor_identifier {
    monitor_id = datadog_monitor.cpu.id
  }
  one_time_schedule {
    start = "2030-01-01T00:00:00Z"
    end   = "2030-01-01T04:00:00Z"
  }
}

resource "datadog_downtime_schedule" "weekday_backups" {
  scope = "env:staging AND service:inventory-service"
  monitor_identifier {
    monitor_tags = ["service:inventory-service"]
  }
  recurring_schedule {
    timezone = "Europe/Paris"
    recurrence {
      duration = "3h"
      rrule    = "FREQ=WEEKLY;BYDAY=MO,WE,FR;UNTIL=20301231T000000Z"
      start    = "2030-01-07T00:30:00"
    }
  }
  display_timezone                 = "Europe/Paris"
  mute_first_recovery_notification = false
  notify_end_states                = ["alert", "no data", "warn"]
  notify_end_types                 = ["canceled", "expired"]
}

resource "datadog_downtime_schedule" "everything" {
  scope   = "*"
  message = "Freeze"
  monitor_identifier {
    monitor_tags = ["*"]
  }
  one_time_schedule {
    start = "2031-06-01T00:00:00Z"
    end   = "2031-06-02T00:00:00Z"
  }
}
