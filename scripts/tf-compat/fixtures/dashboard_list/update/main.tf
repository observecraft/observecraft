# The update: the list is renamed and keeps only the screenboard.
resource "datadog_dashboard" "time" {
  title       = "tf-compat list: timeboard"
  layout_type = "ordered"
  widget {
    note_definition {
      content = "Ordered layout"
    }
  }
}

resource "datadog_dashboard" "screen" {
  title       = "tf-compat list: screenboard"
  layout_type = "free"
  widget {
    timeseries_definition {
      title = "CPU"
      request {
        q = "avg:system.cpu.user{*} by {host}"
      }
    }
    widget_layout {
      x      = 0
      y      = 0
      width  = 47
      height = 15
    }
  }
}

resource "datadog_dashboard_list" "checkout" {
  name = "tf-compat checkout (updated)"
  dash_item {
    type    = "custom_screenboard"
    dash_id = datadog_dashboard.screen.id
  }
}

data "datadog_dashboard_list" "checkout" {
  name       = datadog_dashboard_list.checkout.name
  depends_on = [datadog_dashboard_list.checkout]
  lifecycle {
    postcondition {
      condition     = self.id == datadog_dashboard_list.checkout.id
      error_message = "GET /api/v1/dashboard/lists/manual did not list the list by its name"
    }
  }
}
