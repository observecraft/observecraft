# Also leaves the runners' own address out of the evaluation, and turned off.
resource "datadog_security_monitoring_suppression" "ci_deployer" {
  name                 = "tf-compat ci deployer"
  description          = "The deployment pipeline signs in from known runners."
  enabled              = false
  rule_query           = "source:cloudtrail"
  suppression_query    = "@userIdentity.arn:*ci-deployer env:staging"
  data_exclusion_query = "@network.client.ip:192.0.2.10"
}
