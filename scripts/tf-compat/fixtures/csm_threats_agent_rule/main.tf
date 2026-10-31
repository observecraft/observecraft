# A Workload Protection agent rule in its own policy (POST /api/v2/remote_config/products/cws/agent_rules).
resource "datadog_csm_threats_policy" "shop" {
  name    = "tf-compat agent rules"
  enabled = true
}

resource "datadog_csm_threats_agent_rule" "shell" {
  name         = "tf_compat_shell_in_storefront"
  description  = "A shell started in a storefront container"
  enabled      = true
  expression   = "exec.file.name in [\"sh\", \"bash\"] && container.id != \"\""
  policy_id    = datadog_csm_threats_policy.shop.id
  product_tags = ["team:storefront"]
}
