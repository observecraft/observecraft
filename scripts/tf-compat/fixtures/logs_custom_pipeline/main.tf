# A custom pipeline (POST /api/v1/logs/config/pipelines) with processors the engine runs.
resource "datadog_logs_custom_pipeline" "checkout" {
  name       = "tf-compat checkout pipeline"
  is_enabled = true
  filter {
    query = "service:checkout-service"
  }

  processor {
    grok_parser {
      name       = "parse access line"
      is_enabled = true
      source     = "message"
      samples    = ["GET /cart 200"]
      grok {
        support_rules = ""
        match_rules   = "access %%{word:http.method} %%{notSpace:http.url} %%{integer:http.status_code}"
      }
    }
  }

  processor {
    status_remapper {
      name       = "status from level"
      is_enabled = true
      sources    = ["level"]
    }
  }

  processor {
    attribute_remapper {
      name                 = "client ip"
      is_enabled           = true
      sources              = ["client"]
      source_type          = "attribute"
      target               = "network.client.ip"
      target_type          = "attribute"
      preserve_source      = false
      override_on_conflict = false
    }
  }

  processor {
    category_processor {
      name       = "response class"
      is_enabled = true
      target     = "http.status_category"
      category {
        name = "error"
        filter {
          query = "@http.status_code:[500 TO 599]"
        }
      }
      category {
        name = "ok"
        filter {
          query = "@http.status_code:[200 TO 299]"
        }
      }
    }
  }
}
