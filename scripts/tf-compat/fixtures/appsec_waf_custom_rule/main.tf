# An In-App WAF custom rule (POST /api/v2/remote_config/products/asm/waf/custom_rules).
resource "datadog_appsec_waf_custom_rule" "bad_actor" {
  name      = "tf-compat bad actor query"
  enabled   = true
  blocking  = false
  path_glob = "/api/search"

  scope {
    env     = "prod"
    service = "storefront-api"
  }

  # Like Datadog, the server answers with `options` on every condition, which the provider reads back
  # as a block: configurations declare it.
  condition {
    operator = "match_regex"
    parameters {
      input {
        address  = "server.request.query"
        key_path = ["q"]
      }
      options {
        case_sensitive = false
      }
      regex = "badactor"
    }
  }

  tags = {
    category = "attack_attempt"
    type     = "tf_compat"
  }
}
