# A scanning group (POST /api/v2/sensitive-data-scanner/config/groups, read back from GET /config).
resource "datadog_sensitive_data_scanner_group" "pci" {
  name         = "tf-compat PCI compliance"
  description  = "Payment data of the billing services"
  is_enabled   = false
  product_list = ["logs"]
  filter {
    query = "service:billing"
  }
}
