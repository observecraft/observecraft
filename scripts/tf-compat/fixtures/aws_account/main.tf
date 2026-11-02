# An AWS account (POST /api/v2/integration/aws/accounts): role delegation, two regions, EC2 tag filter.
resource "datadog_integration_aws_external_id" "storefront" {}

resource "datadog_integration_aws_account" "storefront" {
  aws_account_id = "123456789012"
  aws_partition  = "aws"
  account_tags   = ["team:storefront", "env:prod"]

  auth_config {
    aws_auth_config_role {
      role_name   = "ObserveCraftIntegrationRole"
      external_id = datadog_integration_aws_external_id.storefront.id
    }
  }

  aws_regions {
    include_only = ["us-east-1", "eu-west-1"]
  }

  logs_config {
    lambda_forwarder {
      lambdas = ["arn:aws:lambda:us-east-1:123456789012:function:DatadogForwarder"]
      sources = ["s3", "elbv2"]
    }
  }

  metrics_config {
    namespace_filters {}
    tag_filters {
      namespace = "AWS/EC2"
      tags      = ["env:prod"]
    }
  }

  resources_config {}

  traces_config {
    xray_services {}
  }
}

# What the add-account flows read: the namespaces and the role's IAM permissions.
data "datadog_integration_aws_available_namespaces" "all" {}
data "datadog_integration_aws_iam_permissions" "role" {}

output "namespaces_include_ec2" {
  value = contains(data.datadog_integration_aws_available_namespaces.all.aws_namespaces, "AWS/EC2")
}

output "role_can_read_metrics" {
  value = contains(data.datadog_integration_aws_iam_permissions.role.iam_permissions, "cloudwatch:GetMetricData")
}
