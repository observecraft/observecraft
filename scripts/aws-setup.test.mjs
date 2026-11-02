import assert from 'node:assert/strict';
import test from 'node:test';
import { cloudFormationTemplate, forwarderStackUrl, permissionsPolicy, terraformConfig, trustPolicy, validAccountId, validRoleName } from '../web/src/pages/integrations/aws/awsSetup.ts';

const PRINCIPAL = 'arn:aws:iam::210987654321:role/observecraft-server';
const EXTERNAL_ID = '2803c423184c499dbd123d346e5bd16f';
const PERMISSIONS = ['cloudwatch:GetMetricData', 'cloudwatch:ListMetrics', 'tag:GetResources'];

test('the role trusts the server principal only with the external id', () => {
  assert.deepEqual(trustPolicy(PRINCIPAL, EXTERNAL_ID), {
    Version: '2012-10-17',
    Statement: [{ Effect: 'Allow', Principal: { AWS: PRINCIPAL }, Action: 'sts:AssumeRole', Condition: { StringEquals: { 'sts:ExternalId': EXTERNAL_ID } } }],
  });
  assert.deepEqual(permissionsPolicy(PERMISSIONS), { Version: '2012-10-17', Statement: [{ Effect: 'Allow', Action: PERMISSIONS, Resource: '*' }] });
});

test('the CloudFormation template creates the role with both policies and outputs its ARN', () => {
  const template = JSON.parse(cloudFormationTemplate({ principalArn: PRINCIPAL, externalId: EXTERNAL_ID, roleName: 'ObserveCraftIntegrationRole', permissions: PERMISSIONS }));
  assert.equal(template.AWSTemplateFormatVersion, '2010-09-09');
  const role = template.Resources.ObserveCraftIntegrationRole;
  assert.equal(role.Type, 'AWS::IAM::Role');
  assert.equal(role.Properties.RoleName, 'ObserveCraftIntegrationRole');
  assert.deepEqual(role.Properties.AssumeRolePolicyDocument, trustPolicy(PRINCIPAL, EXTERNAL_ID));
  assert.deepEqual(role.Properties.Policies[0].PolicyDocument, permissionsPolicy(PERMISSIONS));
  assert.deepEqual(template.Outputs.RoleArn.Value, { 'Fn::GetAtt': ['ObserveCraftIntegrationRole', 'Arn'] });
});

test('the Terraform configuration creates the role and the account against this install', () => {
  const hcl = terraformConfig({
    principalArn: PRINCIPAL,
    roleName: 'ObserveCraftIntegrationRole',
    accountId: '123456789012',
    permissions: PERMISSIONS,
    apiUrl: 'https://observecraft.example.com',
    regions: ['us-east-1', 'eu-west-1'],
    logSources: ['s3'],
  });
  assert.match(hcl, /source\s+= "DataDog\/datadog"/);
  assert.match(hcl, /api_url = "https:\/\/observecraft\.example\.com"/);
  assert.match(hcl, /resource "datadog_integration_aws_external_id" "this" \{\}/);
  assert.match(hcl, /resource "aws_iam_role" "observecraft"/);
  assert.match(hcl, /variable = "sts:ExternalId"\n\s+values\s+= \[datadog_integration_aws_external_id\.this\.id\]/);
  assert.match(hcl, /identifiers = \["arn:aws:iam::210987654321:role\/observecraft-server"\]/);
  assert.match(hcl, /"cloudwatch:GetMetricData",/);
  assert.match(hcl, /aws_account_id = "123456789012"/);
  assert.match(hcl, /include_only = \["us-east-1", "eu-west-1"\]/);
  assert.match(hcl, /sources = \["s3"\]/);
  assert.match(hcl, /depends_on = \[aws_iam_role_policy\.observecraft\]/);
  // No key in the file: they come from the environment.
  assert.doesNotMatch(hcl, /api_key|app_key/);
});

test('all regions are written as include_all', () => {
  const hcl = terraformConfig({ principalArn: PRINCIPAL, roleName: 'R', accountId: '123456789012', permissions: PERMISSIONS, apiUrl: 'https://o.example.com', regions: [], logSources: [] });
  assert.match(hcl, /include_all = true/);
});

test('the forwarder stack points the Datadog Forwarder at this install, without the API key in the link', () => {
  const url = new URL(forwarderStackUrl({ region: 'eu-west-1', intakeHost: 'intake.example.com', intakePort: '443', apiUrl: 'https://observecraft.example.com' }));
  assert.equal(url.host, 'eu-west-1.console.aws.amazon.com');
  const params = new URLSearchParams(url.hash.split('?')[1]);
  assert.equal(params.get('templateURL'), 'https://datadog-cloudformation-template.s3.amazonaws.com/aws/forwarder/latest.yaml');
  assert.equal(params.get('param_DdUrl'), 'intake.example.com');
  assert.equal(params.get('param_DdPort'), '443');
  assert.equal(params.get('param_DdApiUrl'), 'https://observecraft.example.com');
  assert.equal(params.get('param_DdApiKey'), null);
});

test('account ids are 12 digits and role names follow IAM rules', () => {
  assert.equal(validAccountId('123456789012'), true);
  assert.equal(validAccountId('12345678901'), false);
  assert.equal(validAccountId('12345678901a'), false);
  assert.equal(validRoleName('ObserveCraftIntegrationRole'), true);
  assert.equal(validRoleName('role with space'), false);
  assert.equal(validRoleName('x'.repeat(65)), false);
});

test('the forwarder link only takes an AWS region, so nothing else can reach the console URL', () => {
  const input = { intakeHost: 'intake.example.com', intakePort: '443', apiUrl: 'https://observecraft.example.com' };
  assert.equal(new URL(forwarderStackUrl({ ...input, region: 'us-gov-west-1' })).host, 'us-gov-west-1.console.aws.amazon.com');
  for (const region of ['evil.example.com/', 'eu-west-1#x', 'javascript:alert(1)', '']) assert.throws(() => forwarderStackUrl({ ...input, region }), /region/, region);
});
