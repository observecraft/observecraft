# A suppression (POST /api/v2/security_monitoring/configuration/suppressions): the deployment
# pipeline's console sign-ins in staging.
resource "datadog_security_monitoring_suppression" "ci_deployer" {
  name              = "tf-compat ci deployer"
  description       = "The deployment pipeline signs in from known runners."
  enabled           = true
  rule_query        = "source:cloudtrail"
  suppression_query = "@userIdentity.arn:*ci-deployer env:staging"
}
