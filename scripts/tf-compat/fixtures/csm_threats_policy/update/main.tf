# Another description, a second host group, and turned off.
resource "datadog_csm_threats_policy" "storefront" {
  name            = "tf-compat storefront hosts"
  description     = "Agent rules for the storefront and checkout hosts"
  enabled         = false
  host_tags_lists = [["env:prod", "service:storefront-api"], ["env:prod", "service:checkout"]]
}
