# An APM retention filter (POST /api/v2/apm/config/retention-filters): slow checkout requests.
resource "datadog_apm_retention_filter" "slow_checkout" {
  name        = "tf-compat slow checkout"
  rate        = "1.0"
  filter_type = "spans-sampling-processor"
  enabled     = true
  filter {
    query = "service:storefront-api resource_name:\"POST /checkout\" @duration:>2000000000"
  }
}
