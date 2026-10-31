# A passlist entry (POST /api/v2/remote_config/products/asm/waf/exclusion_filters): an internal scanner.
resource "datadog_appsec_waf_exclusion_filter" "scanner" {
  description = "tf-compat internal scanner"
  enabled     = true
  ip_list     = ["192.0.2.250"]
}
