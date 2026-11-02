// What the add-account flows hand to AWS: the integration role's trust and permissions policies, a
// CloudFormation template creating it, a Terraform configuration creating the role and the account
// (Datadog's Terraform provider against this install), and the link deploying the Datadog Forwarder
// (Apache-2.0, DataDog/datadog-serverless-functions) with its logs sent to this install's intake.

export const AWS_REGIONS = [
  'us-east-1',
  'us-east-2',
  'us-west-1',
  'us-west-2',
  'af-south-1',
  'ap-east-1',
  'ap-south-1',
  'ap-south-2',
  'ap-southeast-1',
  'ap-southeast-2',
  'ap-southeast-3',
  'ap-southeast-4',
  'ap-northeast-1',
  'ap-northeast-2',
  'ap-northeast-3',
  'ca-central-1',
  'ca-west-1',
  'eu-central-1',
  'eu-central-2',
  'eu-west-1',
  'eu-west-2',
  'eu-west-3',
  'eu-south-1',
  'eu-south-2',
  'eu-north-1',
  'il-central-1',
  'me-south-1',
  'me-central-1',
  'sa-east-1',
];

export const DEFAULT_ROLE_NAME = 'ObserveCraftIntegrationRole';

export const validAccountId = (id: string) => /^\d{12}$/.test(id);
export const validRoleName = (name: string) => /^[\w+=,.@-]{1,64}$/.test(name);

export function trustPolicy(principalArn: string, externalId: string) {
  return {
    Version: '2012-10-17',
    Statement: [{ Effect: 'Allow', Principal: { AWS: principalArn }, Action: 'sts:AssumeRole', Condition: { StringEquals: { 'sts:ExternalId': externalId } } }],
  };
}

export function permissionsPolicy(permissions: string[]) {
  return { Version: '2012-10-17', Statement: [{ Effect: 'Allow', Action: permissions, Resource: '*' }] };
}

interface RoleInput {
  principalArn: string;
  externalId: string;
  roleName: string;
  permissions: string[];
}

/** A CloudFormation template (JSON) creating the integration role. */
export function cloudFormationTemplate({ principalArn, externalId, roleName, permissions }: RoleInput): string {
  const template = {
    AWSTemplateFormatVersion: '2010-09-09',
    Description: 'ObserveCraft AWS integration role: lets ObserveCraft read CloudWatch metrics and resource tags.',
    Resources: {
      ObserveCraftIntegrationRole: {
        Type: 'AWS::IAM::Role',
        Properties: {
          RoleName: roleName,
          AssumeRolePolicyDocument: trustPolicy(principalArn, externalId),
          Policies: [{ PolicyName: 'ObserveCraftIntegrationPolicy', PolicyDocument: permissionsPolicy(permissions) }],
        },
      },
    },
    Outputs: { RoleArn: { Description: 'The role to enter in ObserveCraft', Value: { 'Fn::GetAtt': ['ObserveCraftIntegrationRole', 'Arn'] } } },
  };
  return JSON.stringify(template, null, 2) + '\n';
}

interface TerraformInput {
  principalArn: string;
  roleName: string;
  accountId: string;
  permissions: string[];
  apiUrl: string;
  /** Empty: all regions. */
  regions: string[];
  logSources: string[];
}

/** HCL string literal (JSON's escaping is valid HCL for these values). */
const str = (s: string) => JSON.stringify(s);
const list = (items: string[]) => `[${items.map(str).join(', ')}]`;

/** Terraform: the role (AWS provider) and the account (Datadog's provider against this install). Keys come from the environment. */
export function terraformConfig({ principalArn, roleName, accountId, permissions, apiUrl, regions, logSources }: TerraformInput): string {
  return `terraform {
  required_providers {
    aws = {
      source = "hashicorp/aws"
    }
    datadog = {
      source = "DataDog/datadog"
    }
  }
}

provider "aws" {}

# Keys from DD_API_KEY (an ObserveCraft ingest key) and DD_APP_KEY (an application key).
provider "datadog" {
  api_url = ${str(apiUrl)}
}

resource "datadog_integration_aws_external_id" "this" {}

data "aws_iam_policy_document" "observecraft_assume_role" {
  statement {
    actions = ["sts:AssumeRole"]
    principals {
      type        = "AWS"
      identifiers = ${list([principalArn])}
    }
    condition {
      test     = "StringEquals"
      variable = "sts:ExternalId"
      values   = [datadog_integration_aws_external_id.this.id]
    }
  }
}

resource "aws_iam_role" "observecraft" {
  name               = ${str(roleName)}
  description        = "ObserveCraft AWS integration"
  assume_role_policy = data.aws_iam_policy_document.observecraft_assume_role.json
}

resource "aws_iam_role_policy" "observecraft" {
  name = "ObserveCraftIntegrationPolicy"
  role = aws_iam_role.observecraft.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect = "Allow"
      Action = [
${permissions.map((p) => `        ${str(p)},`).join('\n')}
      ]
      Resource = "*"
    }]
  })
}

resource "datadog_integration_aws_account" "this" {
  aws_account_id = ${str(accountId)}
  aws_partition  = "aws"

  auth_config {
    aws_auth_config_role {
      role_name   = aws_iam_role.observecraft.name
      external_id = datadog_integration_aws_external_id.this.id
    }
  }

  aws_regions {
    ${regions.length ? `include_only = ${list(regions)}` : 'include_all = true'}
  }

  logs_config {
    lambda_forwarder {
      sources = ${list(logSources)}
    }
  }

  metrics_config {
    namespace_filters {}
  }

  resources_config {}

  traces_config {
    xray_services {}
  }

  depends_on = [aws_iam_role_policy.observecraft]
}
`;
}

interface ForwarderInput {
  region: string;
  intakeHost: string;
  intakePort: string;
  apiUrl: string;
}

export const FORWARDER_TEMPLATE_URL = 'https://datadog-cloudformation-template.s3.amazonaws.com/aws/forwarder/latest.yaml';

/** AWS console quick-create link for the Datadog Forwarder stack, its logs sent to this install. The API key is entered in the console. */
export function forwarderStackUrl({ region, intakeHost, intakePort, apiUrl }: ForwarderInput): string {
  const params = new URLSearchParams({
    stackName: 'observecraft-forwarder',
    templateURL: FORWARDER_TEMPLATE_URL,
    param_DdUrl: intakeHost,
    param_DdPort: intakePort,
    param_DdApiUrl: apiUrl,
  });
  return `https://${region}.console.aws.amazon.com/cloudformation/home?region=${region}#/stacks/quickcreate?${params}`;
}
