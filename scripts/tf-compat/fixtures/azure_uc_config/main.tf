# A subscription's actual and amortized cost exports (POST /api/v2/cost/azure_uc_config): one pair, read
# back by dataset.
resource "datadog_azure_uc_config" "storefront" {
  account_id = "00000000-0000-0000-0000-000000000001"
  client_id  = "00000000-0000-0000-0000-000000000002"
  scope      = "/subscriptions/00000000-0000-0000-0000-000000000003"

  actual_bill_config {
    export_name       = "observecraft-actual"
    export_path       = "cost-exports"
    storage_account   = "examplecostexports"
    storage_container = "costs"
  }

  amortized_bill_config {
    export_name       = "observecraft-amortized"
    export_path       = "cost-exports"
    storage_account   = "examplecostexports"
    storage_container = "costs"
  }
}
