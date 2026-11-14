# Hashed instead of partly masked, with suppressions; the standard pattern rule disabled.
resource "datadog_sensitive_data_scanner_group" "group" {
  name         = "tf-compat rules"
  is_enabled   = true
  product_list = ["logs"]
  filter {
    query = "service:order-service"
  }
}

data "datadog_sensitive_data_scanner_standard_pattern" "card" {
  filter = "Credit Card Number"
}

resource "datadog_sensitive_data_scanner_rule" "order_ids" {
  group_id    = datadog_sensitive_data_scanner_group.group.id
  name        = "tf-compat customer ids (hashed)"
  description = "Customer ids in order logs"
  is_enabled  = true
  pattern     = "C-[0-9]{6,8}"
  namespaces  = ["customer"]
  tags        = ["sensitive_data:customer_id", "team:orders"]
  priority    = 1
  text_replacement {
    type = "hash"
  }
  suppressions {
    exact_match = ["C-000000"]
    starts_with = ["C-999"]
  }
}

resource "datadog_sensitive_data_scanner_rule" "cards" {
  group_id            = datadog_sensitive_data_scanner_group.group.id
  name                = "tf-compat cards"
  is_enabled          = false
  standard_pattern_id = data.datadog_sensitive_data_scanner_standard_pattern.card.id
  tags                = ["sensitive_data:credit_card"]
  text_replacement {
    type               = "replacement_string"
    replacement_string = "[card]"
  }
  depends_on = [datadog_sensitive_data_scanner_rule.order_ids]
}
