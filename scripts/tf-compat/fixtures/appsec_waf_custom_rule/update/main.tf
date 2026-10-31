# Blocking, a wider match, and turned off.
resource "datadog_appsec_waf_custom_rule" "bad_actor" {
  name      = "tf-compat bad actor query"
  enabled   = false
  blocking  = true
  path_glob = "/api/search"

  scope {
    env     = "prod"
    service = "storefront-api"
  }

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
      regex = "bad(actor|guy)"
    }
  }

  tags = {
    category = "attack_attempt"
    type     = "tf_compat"
  }
}
