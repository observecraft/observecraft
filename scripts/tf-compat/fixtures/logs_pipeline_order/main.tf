# The processing order of every pipeline (PUT /api/v1/logs/config/pipeline-order): the new one first.
resource "datadog_logs_custom_pipeline" "first" {
  name       = "tf-compat first pipeline"
  is_enabled = true
  filter {
    query = "source:nginx"
  }
}

data "datadog_logs_pipelines" "all" {
  depends_on = [datadog_logs_custom_pipeline.first]
}

resource "datadog_logs_pipeline_order" "order" {
  name = "tf-compat-pipeline-order"
  pipelines = concat(
    [datadog_logs_custom_pipeline.first.id],
    [for p in data.datadog_logs_pipelines.all.logs_pipelines : p.id if p.id != datadog_logs_custom_pipeline.first.id],
  )
}
