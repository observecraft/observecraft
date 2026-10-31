# The archive order (/api/v2/logs/config/archive-order), read back as the organization has it.
resource "datadog_logs_archive" "gcs" {
  name  = "tf-compat-archive-gcs"
  query = "env:prod"
  gcs_archive {
    bucket       = "example-logs-archive"
    path         = "/gcs"
    client_email = "archiver@example-project.iam.gserviceaccount.com"
    project_id   = "example-project"
  }
}

resource "datadog_logs_archive_order" "order" {
  depends_on = [datadog_logs_archive.gcs]
}
