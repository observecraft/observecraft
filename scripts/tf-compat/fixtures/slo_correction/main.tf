# datadog_slo_correction (POST/GET/PATCH/DELETE /api/v1/slo/correction): a one-off correction of an SLO.

resource "datadog_service_level_objective" "checkout" {
  name = "tf-compat checkout availability (corrected)"
  type = "metric"
  query {
    numerator   = "sum:trace.http.request.hits{service:order-service,!http.status_code:5xx}.as_count()"
    denominator = "sum:trace.http.request.hits{service:order-service}.as_count()"
  }
  thresholds {
    timeframe = "7d"
    target    = 99.5
  }
}

resource "datadog_slo_correction" "maintenance" {
  slo_id      = datadog_service_level_objective.checkout.id
  category    = "Scheduled Maintenance"
  description = "Database upgrade"
  start       = 1798761600
  end         = 1798765200
  timezone    = "UTC"
}
