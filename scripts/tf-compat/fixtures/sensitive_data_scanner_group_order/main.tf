# The order of every scanning group of the org (PATCH /api/v2/sensitive-data-scanner/config): ours first, then
# whatever the org already has.
resource "datadog_sensitive_data_scanner_group" "credentials" {
  name         = "tf-compat credentials"
  is_enabled   = true
  product_list = ["logs"]
  filter {
    query = "*"
  }
}

resource "datadog_sensitive_data_scanner_group" "personal" {
  name         = "tf-compat personal data"
  is_enabled   = true
  product_list = ["logs"]
  filter {
    query = "service:storefront-api"
  }
  depends_on = [datadog_sensitive_data_scanner_group.credentials]
}

data "datadog_sensitive_data_scanner_group_order" "all" {
  depends_on = [datadog_sensitive_data_scanner_group.personal]
}

locals {
  ours = [datadog_sensitive_data_scanner_group.credentials.id, datadog_sensitive_data_scanner_group.personal.id]
}

resource "datadog_sensitive_data_scanner_group_order" "order" {
  group_ids = concat(local.ours, [for id in data.datadog_sensitive_data_scanner_group_order.all.group_ids : id if !contains(local.ours, id)])
}
