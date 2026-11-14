# Enabled, wider filter, half of the logs sampled.
resource "datadog_sensitive_data_scanner_group" "pci" {
  name         = "tf-compat PCI compliance (billing and orders)"
  description  = "Payment data of the billing and order services"
  is_enabled   = true
  product_list = ["logs"]
  filter {
    query = "service:(billing OR order-service)"
  }
  samplings {
    product = "logs"
    rate    = 50
  }
}
