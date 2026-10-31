# A wider expression, another description, and turned off.
resource "datadog_csm_threats_policy" "shop" {
  name    = "tf-compat agent rules"
  enabled = true
}

resource "datadog_csm_threats_agent_rule" "shell" {
  name         = "tf_compat_shell_in_storefront"
  description  = "A shell or interpreter started in a storefront container"
  enabled      = false
  expression   = "exec.file.name in [\"sh\", \"bash\", \"python3\"] && container.id != \"\""
  policy_id    = datadog_csm_threats_policy.shop.id
  product_tags = ["team:storefront", "severity:medium"]
}
