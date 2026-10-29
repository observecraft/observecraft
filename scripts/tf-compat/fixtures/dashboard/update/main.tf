# The update: title, a template variable default and a widget change; the note moves into the group.
resource "datadog_dashboard" "checkout" {
  title       = "tf-compat checkout (updated)"
  description = "Managed by make tf-compat"
  layout_type = "ordered"
  reflow_type = "fixed"
  tags        = ["team:checkout", "team:payments"]

  template_variable {
    name     = "env"
    prefix   = "env"
    defaults = ["staging"]
  }
  template_variable {
    name             = "service"
    prefix           = "service"
    available_values = ["checkout-api", "order-service"]
    defaults         = ["checkout-api"]
  }
  template_variable_preset {
    name = "staging"
    template_variable {
      name   = "env"
      values = ["staging"]
    }
  }

  widget {
    timeseries_definition {
      title       = "Requests and errors"
      show_legend = true
      request {
        q            = "sum:trace.http.request.hits{$env,$service}.as_count()"
        display_type = "bars"
      }
      request {
        q            = "sum:trace.http.request.errors{$env,$service}.as_count()"
        display_type = "line"
      }
    }
    widget_layout {
      x      = 0
      y      = 0
      width  = 9
      height = 3
    }
  }

  widget {
    query_value_definition {
      title     = "Latency"
      precision = 3
      request {
        query {
          metric_query {
            name       = "query1"
            query      = "avg:trace.http.request.duration{$env,$service}"
            aggregator = "avg"
          }
        }
        formula {
          formula_expression = "query1"
        }
        conditional_formats {
          comparator = ">"
          value      = 0.5
          palette    = "white_on_red"
        }
      }
    }
    widget_layout {
      x      = 9
      y      = 0
      width  = 3
      height = 3
    }
  }

  widget {
    group_definition {
      title            = "Hosts"
      layout_type      = "ordered"
      background_color = "vivid_blue"

      widget {
        toplist_definition {
          title = "CPU by host"
          request {
            q = "top(avg:system.cpu.user{$env} by {host}, 10, 'mean', 'desc')"
          }
        }
        widget_layout {
          x      = 0
          y      = 0
          width  = 8
          height = 3
        }
      }
      widget {
        note_definition {
          content          = "Runbook: page the checkout on-call."
          background_color = "yellow"
          font_size        = "14"
          text_align       = "left"
          show_tick        = false
        }
        widget_layout {
          x      = 8
          y      = 0
          width  = 4
          height = 3
        }
      }
    }
    widget_layout {
      x      = 0
      y      = 3
      width  = 12
      height = 4
    }
  }
}

data "datadog_dashboard" "checkout" {
  name       = datadog_dashboard.checkout.title
  depends_on = [datadog_dashboard.checkout]
  lifecycle {
    postcondition {
      condition     = self.id == datadog_dashboard.checkout.id
      error_message = "GET /api/v1/dashboard did not list the dashboard by its title"
    }
  }
}

output "dashboard_url" {
  value = datadog_dashboard.checkout.url
}
