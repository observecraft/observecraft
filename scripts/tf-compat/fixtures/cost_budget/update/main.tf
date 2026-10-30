# Renamed, a raised amount for checkout in February, and the custom forecast moved to February.
resource "datadog_cost_budget" "teams" {
  name          = "tf-compat team budgets (FY26 Q1)"
  metrics_query = "aws.cost.amortized{env:prod} by {team}"
  start_month   = 202601
  end_month     = 202603

  budget_line {
    amounts = { "202601" = 30000, "202602" = 34000, "202603" = 32000 }
    tag_filters {
      tag_key   = "team"
      tag_value = "checkout"
    }
  }

  budget_line {
    amounts = { "202601" = 12000, "202602" = 12000, "202603" = 12000 }
    tag_filters {
      tag_key   = "team"
      tag_value = "platform"
    }
  }
}

resource "datadog_cost_custom_forecast" "teams" {
  budget_uid = datadog_cost_budget.teams.id

  entries {
    month  = 202602
    amount = 36000
    tag_filters {
      tag_key   = "team"
      tag_value = "checkout"
    }
  }
}

data "datadog_cost_budget" "teams" {
  id = datadog_cost_budget.teams.id
}

output "budget" {
  value = {
    name         = data.datadog_cost_budget.teams.name
    total_amount = datadog_cost_budget.teams.total_amount
  }
}
