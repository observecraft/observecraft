# datadog_service_level_objective (POST/GET/PUT/DELETE /api/v1/slo) and the
# datadog_service_level_objective(s) data sources (GET /api/v1/slo, GET /api/v1/slo/search).

# Created by setup.sh through our own API.
variable "monitor_id" { type = number }

# A metric SLO with a 7d and a 30d threshold.
resource "datadog_service_level_objective" "metric" {
  name        = "tf-compat checkout availability"
  type        = "metric"
  description = "Checkout requests answered without a server error"
  query {
    numerator   = "sum:trace.http.request.hits{service:order-service,!http.status_code:5xx}.as_count()"
    denominator = "sum:trace.http.request.hits{service:order-service}.as_count()"
  }
  thresholds {
    timeframe = "7d"
    target    = 99.5
    warning   = 99.8
  }
  thresholds {
    timeframe = "30d"
    target    = 99.9
  }
  timeframe         = "7d"
  target_threshold  = 99.5
  warning_threshold = 99.8
  tags              = ["env:prod", "service:order-service", "tf-compat"]
}

# A metric SLO from named queries (sli_specification.count).
resource "datadog_service_level_objective" "count" {
  name = "tf-compat billing success"
  type = "metric"
  sli_specification {
    count {
      good_events_formula = "query1 - query2"
      bad_events_formula  = "query2"
      queries {
        metric_query {
          name  = "query1"
          query = "sum:trace.http.request.hits{service:billing}.as_count()"
        }
      }
      queries {
        metric_query {
          name  = "query2"
          query = "sum:trace.http.request.errors{service:billing}.as_count()"
        }
      }
    }
  }
  thresholds {
    timeframe = "30d"
    target    = 99
  }
  tags = ["service:billing", "tf-compat"]
}

# A monitor SLO. validate = false: the provider would check the monitor with GET /api/v1/monitor/{id},
# which comes with datadog_monitor (Stage 2).
resource "datadog_service_level_objective" "monitor" {
  name        = "tf-compat checkout error monitor"
  type        = "monitor"
  monitor_ids = [var.monitor_id]
  thresholds {
    timeframe = "30d"
    target    = 99.9
    warning   = 99.95
  }
  validate = false
  tags     = ["tf-compat"]
}

# A time-slice SLO: p95 latency under 500 ms per 5-minute slice.
resource "datadog_service_level_objective" "time_slice" {
  name = "tf-compat catalog latency"
  type = "time_slice"
  sli_specification {
    time_slice {
      comparator = "<"
      threshold  = 0.5
      query {
        formula {
          formula_expression = "query1"
        }
        query {
          metric_query {
            name  = "query1"
            query = "avg:trace.http.request.duration{service:catalog-service}"
          }
        }
      }
    }
  }
  thresholds {
    timeframe = "7d"
    target    = 99
  }
  timeframe        = "7d"
  target_threshold = 99
}

data "datadog_service_level_objective" "by_id" {
  id = datadog_service_level_objective.metric.id
  lifecycle {
    postcondition {
      condition     = self.id == datadog_service_level_objective.metric.id && self.type == "metric" && self.target_threshold == 99.5
      error_message = "GET /api/v1/slo?ids= did not find the metric SLO"
    }
  }
}

data "datadog_service_level_objectives" "tagged" {
  tags_query = "service:order-service"
  depends_on = [datadog_service_level_objective.metric]
  lifecycle {
    postcondition {
      condition     = contains([for s in self.slos : s.id], datadog_service_level_objective.metric.id)
      error_message = "GET /api/v1/slo?tags_query= did not find the metric SLO"
    }
  }
}

data "datadog_service_level_objectives" "searched" {
  query      = "tf-compat"
  depends_on = [datadog_service_level_objective.metric, datadog_service_level_objective.count, datadog_service_level_objective.monitor, datadog_service_level_objective.time_slice]
  lifecycle {
    postcondition {
      condition     = length(self.slos) >= 4
      error_message = "GET /api/v1/slo/search did not find the fixture's SLOs"
    }
  }
}
