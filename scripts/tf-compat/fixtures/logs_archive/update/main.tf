# Every service, another path, tags dropped, no scan limit.
resource "datadog_logs_archive" "s3" {
  name  = "tf-compat-archive"
  query = "*"

  s3_archive {
    bucket       = "example-logs-archive"
    path         = "/observecraft/all"
    account_id   = "123456789012"
    role_name    = "ObserveCraftArchive"
  }

  include_tags = false
}
