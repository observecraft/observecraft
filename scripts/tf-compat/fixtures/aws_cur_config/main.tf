# Where the management account's Cost and Usage Report is delivered (POST /api/v2/cost/aws_cur_config),
# with the member accounts it covers; the update changes only the filters (PATCH, answered as a list).
resource "datadog_aws_cur_config" "billing" {
  account_id    = "123456789012"
  bucket_name   = "example-org-cur"
  bucket_region = "us-east-1"
  report_name   = "observecraft-costs"
  report_prefix = "cur"

  account_filters {
    include_new_accounts = false
    included_accounts    = ["123456789012", "210987654321"]
  }
}
