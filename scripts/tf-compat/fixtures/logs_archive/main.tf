# An S3 log archive (POST /api/v2/logs/config/archives); its keys are set in the UI.
resource "datadog_logs_archive" "s3" {
  name  = "tf-compat-archive"
  query = "service:order-service"

  s3_archive {
    bucket       = "example-logs-archive"
    path         = "/observecraft"
    account_id   = "123456789012"
    role_name    = "ObserveCraftArchive"
  }

  include_tags                    = true
  rehydration_tags                = ["team:checkout"]
  rehydration_max_scan_size_in_gb = 100
}
