# A Workload Protection agent policy (POST /api/v2/remote_config/products/cws/policy).
resource "datadog_csm_threats_policy" "storefront" {
  name            = "tf-compat storefront hosts"
  description     = "Agent rules for the storefront hosts"
  enabled         = true
  host_tags_lists = [["env:prod", "service:storefront-api"]]
}
