# A free-layout dashboard written as Datadog's JSON document, as exported from Datadog.
resource "datadog_dashboard_json" "storefront" {
  dashboard = jsonencode({
    title       = "tf-compat storefront (JSON)"
    description = "Managed by make tf-compat"
    layout_type = "free"
    notify_list = []
    template_variables = [
      { name = "env", prefix = "env", available_values = [], default = "prod" }
    ]
    widgets = [
      {
        definition = {
          type  = "timeseries"
          title = "Orders"
          requests = [{
            response_format = "timeseries"
            queries         = [{ data_source = "metrics", name = "query1", query = "sum:orders.created{$env,service:storefront-api}.as_count()" }]
            formulas        = [{ formula = "query1" }]
            display_type    = "bars"
          }]
        }
        layout = { x = 0, y = 0, width = 48, height = 15 }
      },
      {
        definition = { type = "free_text", text = "Storefront", color = "#4d4d4d", font_size = "auto", text_align = "left" }
        layout     = { x = 48, y = 0, width = 24, height = 6 }
      },
      {
        definition = {
          type  = "query_table"
          title = "Errors by endpoint"
          requests = [{
            response_format = "scalar"
            queries         = [{ data_source = "metrics", name = "query1", query = "sum:trace.http.request.errors{$env,service:storefront-api} by {resource_name}", aggregator = "sum" }]
            formulas        = [{ formula = "query1", limit = { count = 10, order = "desc" } }]
          }]
        }
        layout = { x = 48, y = 6, width = 48, height = 15 }
      }
    ]
  })
}
