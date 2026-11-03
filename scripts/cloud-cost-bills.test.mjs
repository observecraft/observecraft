import assert from 'node:assert/strict';
import test from 'node:test';
import { azureScope, billErrors, billSource, billState, curBucketPolicy, monthsRead, validPrefix } from '../web/src/pages/cloudCost/billSetup.ts';

const aws = (attrs = {}) => ({
  id: '1',
  provider: 'aws',
  last_import_at: null,
  imported_months: ['2026-09', '2026-10'],
  doc: {
    type: 'aws_cur_config',
    id: '1',
    attributes: {
      account_id: '123456789012',
      bucket_name: 'example-org-cur',
      bucket_region: 'us-east-1',
      report_name: 'observecraft-costs',
      report_prefix: 'cur',
      status: 'active',
      created_at: '',
      error_messages: [],
      ...attrs,
    },
  },
});

const azureExport = (dataset_type, extra = {}) => ({
  dataset_type,
  account_id: 'tenant',
  client_id: 'client',
  scope: '/subscriptions/sub',
  export_name: `costs-${dataset_type}`,
  export_path: 'exports',
  storage_account: 'exampleorgcosts',
  storage_container: 'billing',
  status: 'active',
  error_messages: [],
  created_at: '',
  ...extra,
});
const azure = (actual = {}, amortized = {}) => ({
  id: '2',
  provider: 'azure',
  imported_months: ['actual:2026-10', 'amortized:2026-10', 'actual:2026-09'],
  doc: { type: 'azure_uc_configs', id: '2', attributes: { configs: [azureExport('amortized', amortized), azureExport('actual', actual)] } },
});

test('the export prefix has no leading or trailing slash', () => {
  assert.ok(validPrefix('cur'));
  assert.ok(validPrefix('billing/cur'));
  assert.ok(validPrefix(''));
  assert.ok(!validPrefix('/cur'));
  assert.ok(!validPrefix('cur/'));
});

test('the bucket policy lets the role list the bucket and read its objects', () => {
  assert.deepEqual(
    curBucketPolicy(' example-org-cur ').Statement.map((s) => [s.Action, s.Resource]),
    [
      [['s3:ListBucket'], 'arn:aws:s3:::example-org-cur'],
      [['s3:GetObject'], 'arn:aws:s3:::example-org-cur/*'],
    ],
  );
  assert.equal(curBucketPolicy('').Statement[0].Resource, 'arn:aws:s3:::<BUCKET_NAME>');
});

test('the Azure scope is a subscription or a billing account', () => {
  assert.equal(azureScope('subscription', ' 0000-1111 '), '/subscriptions/0000-1111');
  assert.equal(azureScope('billing-account', '12345678'), '/providers/Microsoft.Billing/billingAccounts/12345678');
});

test('state, source and months of an AWS config', () => {
  assert.equal(billState(aws()), 'active');
  assert.equal(billState(aws({ status: 'disabled', error_messages: ['x'] })), 'disabled');
  assert.equal(billState(aws({ error_messages: ['Access Denied'] })), 'error');
  assert.deepEqual(billSource(aws()), { account: '123456789012', location: 's3://example-org-cur/cur/observecraft-costs' });
  assert.equal(billSource(aws({ report_prefix: '' })).location, 's3://example-org-cur/observecraft-costs');
  assert.equal(monthsRead(aws()), 2);
});

test('state, source and months of an Azure config', () => {
  assert.equal(billState(azure()), 'active');
  assert.equal(billState(azure({ status: 'disabled' }, { status: 'disabled' })), 'disabled');
  assert.equal(billState(azure({ error_messages: ['No export found'] })), 'error');
  assert.deepEqual(billErrors(azure({ error_messages: ['No export found'] })), ['actual: No export found']);
  assert.deepEqual(billSource(azure()), { account: 'tenant · /subscriptions/sub', location: 'exampleorgcosts/billing' });
  assert.equal(monthsRead(azure()), 2);
});
