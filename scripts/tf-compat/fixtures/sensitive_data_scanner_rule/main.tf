# Scanning rules (POST /api/v2/sensitive-data-scanner/config/rules): one with its own pattern, keywords and a
# partial replacement, one using a standard pattern found by name (GET /config/standard-patterns).
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
  group_id            = datadog_sensitive_data_scanner_group.group.id
  name                = "tf-compat customer ids"
  description         = "Customer ids in order logs"
  is_enabled          = true
  pattern             = "C-[0-9]{6}"
  namespaces          = ["customer"]
  excluded_namespaces = ["customer.public"]
  tags                = ["sensitive_data:customer_id"]
  priority            = 2
  text_replacement {
    type            = "partial_replacement_from_end"
    number_of_chars = 4
  }
  included_keyword_configuration {
    keywords        = ["customer", "client"]
    character_count = 20
  }
}

resource "datadog_sensitive_data_scanner_rule" "cards" {
  group_id            = datadog_sensitive_data_scanner_group.group.id
  name                = "tf-compat cards"
  is_enabled          = true
  standard_pattern_id = data.datadog_sensitive_data_scanner_standard_pattern.card.id
  tags                = ["sensitive_data:credit_card"]
  text_replacement {
    type               = "replacement_string"
    replacement_string = "[card]"
  }
  depends_on = [datadog_sensitive_data_scanner_rule.order_ids]
}
