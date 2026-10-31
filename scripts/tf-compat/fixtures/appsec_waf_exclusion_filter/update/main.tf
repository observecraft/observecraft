# A second address, and turned off.
resource "datadog_appsec_waf_exclusion_filter" "scanner" {
  description = "tf-compat internal scanners"
  enabled     = false
  ip_list     = ["192.0.2.250", "192.0.2.251"]
}
