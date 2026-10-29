# The update applied over main.tf: the correction changes in place (PATCH /api/v1/slo/correction/{id}).

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
  category    = "Deployment"
  description = "Database upgrade and rollout"
  start       = 1798761600
  end         = 1798772400
  timezone    = "Europe/Paris"
}
