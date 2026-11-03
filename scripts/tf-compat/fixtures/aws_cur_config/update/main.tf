resource "datadog_aws_cur_config" "billing" {
  account_id    = "123456789012"
  bucket_name   = "example-org-cur"
  bucket_region = "us-east-1"
  report_name   = "observecraft-costs"
  report_prefix = "cur"

  account_filters {
    include_new_accounts = true
    excluded_accounts    = ["210987654321"]
  }
}
