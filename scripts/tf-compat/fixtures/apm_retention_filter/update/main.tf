# Half of them, with their whole traces, and turned off.
resource "datadog_apm_retention_filter" "slow_checkout" {
  name        = "tf-compat slow checkout (sampled)"
  rate        = "0.5"
  trace_rate  = "0.5"
  filter_type = "spans-sampling-processor"
  enabled     = false
  filter {
    query = "service:storefront-api resource_name:\"POST /checkout\" @duration:>2000000000"
  }
}
