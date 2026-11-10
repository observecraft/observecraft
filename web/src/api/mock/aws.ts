// In-browser AWS integration for mock mode (Integrations → Amazon Web Services). Accounts live in memory
// for the session; defaults, validation and updates follow the server (datadog_api/aws_integration.rs).
import seed from '../../../../server/seeds/aws_integration.json';
import type { AwsAccount, AwsAccountAttributes, AwsAccountInput, AwsAccountStatus, AwsSetup } from '../aws/types';

const delay = <T>(value: T, ms = 120) => new Promise<T>((r) => setTimeout(() => r(structuredClone(value)), ms));
const fail = (message: string): never => {
  throw new Error(message);
};

const PERMISSIONS = ['cloudwatch:GetMetricData', 'cloudwatch:ListMetrics', 'tag:GetResources'];
const EXCLUDED = ['AWS/SQS', 'AWS/ElasticMapReduce', 'AWS/Usage'];

const hex = () => Array.from(crypto.getRandomValues(new Uint8Array(16)), (b) => b.toString(16).padStart(2, '0')).join('');

function defaults(): Omit<AwsAccountAttributes, 'aws_account_id' | 'auth_config' | 'created_at' | 'modified_at'> {
  return {
    aws_partition: 'aws',
    account_tags: [],
    aws_regions: { include_all: true },
    logs_config: { lambda_forwarder: { lambdas: [], sources: [], log_source_config: { tag_filters: [] } } },
    metrics_config: { enabled: true, automute_enabled: true, collect_custom_metrics: false, collect_cloudwatch_alarms: false, tag_filters: [], namespace_filters: { exclude_only: EXCLUDED } },
    resources_config: { cloud_security_posture_management_collection: false, extended_collection: true },
    traces_config: { xray_services: { include_only: [] } },
  };
}

let accounts: AwsAccount[] | null = null;

function db(): AwsAccount[] {
  if (accounts) return accounts;
  const at = new Date(Date.UTC(2026, 8, 2, 10, 15)).toISOString();
  const base = defaults();
  accounts = [
    {
      id: '6f1c2b0a-3d4e-4f5a-8b9c-0d1e2f3a4b5c',
      attributes: {
        ...base,
        aws_account_id: '123456789012',
        account_tags: ['env:prod', 'team:storefront'],
        auth_config: { role_name: 'ObserveCraftIntegrationRole', external_id: '2803c423184c499dbd123d346e5bd16f' },
        aws_regions: { include_only: ['us-east-1', 'eu-west-1'] },
        logs_config: {
          lambda_forwarder: { lambdas: ['arn:aws:lambda:us-east-1:123456789012:function:observecraft-forwarder'], sources: ['s3', 'elbv2', 'lambda'], log_source_config: { tag_filters: [] } },
        },
        created_at: at,
        modified_at: at,
      },
    },
    {
      id: '0a9b8c7d-6e5f-4a3b-9c2d-1e0f9a8b7c6d',
      attributes: {
        ...base,
        aws_account_id: '123456789013',
        account_tags: ['env:staging'],
        auth_config: { role_name: 'ObserveCraftIntegrationRole', external_id: 'b6ec0b8a9ddf478e98b0134747bab2c5' },
        created_at: at,
        modified_at: at,
      },
    },
  ];
  return accounts;
}

export const getAwsSetup = (): Promise<AwsSetup> =>
  delay({
    principal_arn: 'arn:aws:iam::123456789012:role/observecraft-server',
    permissions: PERMISSIONS,
    default_excluded_namespaces: EXCLUDED,
    namespaces: seed.namespaces,
    log_services: seed.log_services,
  });

export const newExternalId = (): Promise<string> => delay(hex());

export const listAwsAccounts = (): Promise<AwsAccount[]> => delay(db());

/** Whether the mock has an account: the catalog tile is installed, as the server counts one. */
export const awsConnected = (): boolean => db().length > 0;

function auth(input: AwsAccountInput, current?: AwsAccountAttributes['auth_config']): AwsAccountAttributes['auth_config'] {
  const a = input.auth_config;
  if (!a) return current ?? fail('auth_config is required');
  if (!!a.role_name === !!a.access_key_id) fail('auth_config needs exactly one of role_name and access_key_id');
  if (current && !!current.role_name !== !!a.role_name) fail('cannot switch between role and key based auth');
  if (a.role_name) return { role_name: a.role_name, external_id: a.external_id ?? current?.external_id ?? hex() };
  if (!current && !a.secret_access_key) fail('secret_access_key is required with access_key_id');
  return { access_key_id: a.access_key_id };
}

/** Sections sent replace their fields; regions are replaced whole. */
function apply(input: AwsAccountInput, current: AwsAccountAttributes | null): AwsAccountAttributes {
  const base = current ?? { ...defaults(), aws_account_id: '', auth_config: {}, created_at: '', modified_at: '' };
  const id = input.aws_account_id ?? base.aws_account_id;
  if (!/^\d{12}$/.test(id)) fail(`invalid aws_account_id: ${id}`);
  const now = new Date().toISOString();
  return {
    ...base,
    aws_account_id: id,
    aws_partition: input.aws_partition ?? base.aws_partition,
    account_tags: input.account_tags ?? base.account_tags,
    auth_config: auth(input, current?.auth_config),
    aws_regions: input.aws_regions ? (input.aws_regions as AwsAccountAttributes['aws_regions']) : base.aws_regions,
    logs_config: { ...base.logs_config, ...input.logs_config } as AwsAccountAttributes['logs_config'],
    metrics_config: { ...base.metrics_config, ...input.metrics_config },
    resources_config: { ...base.resources_config, ...input.resources_config },
    traces_config: { ...base.traces_config, ...input.traces_config } as AwsAccountAttributes['traces_config'],
    created_at: current?.created_at || now,
    modified_at: now,
  };
}

export function createAwsAccount(input: AwsAccountInput): Promise<AwsAccount> {
  const attributes = apply(input, null);
  if (db().some((a) => a.attributes.aws_account_id === attributes.aws_account_id)) fail('AWS account with provided id already exists');
  const account = { id: crypto.randomUUID(), attributes };
  db().push(account);
  return delay(account, 300);
}

export function updateAwsAccount(id: string, input: AwsAccountInput): Promise<AwsAccount> {
  const account = db().find((a) => a.id === id) ?? fail('AWS account not found');
  account.attributes = apply(input, account.attributes);
  return delay(account, 200);
}

export function deleteAwsAccount(id: string): Promise<void> {
  accounts = db().filter((a) => a.id !== id);
  return delay(undefined, 200);
}

const STANDARD_REGIONS = ['us-east-1', 'us-east-2', 'us-west-2', 'eu-west-1', 'eu-central-1'];

/** The crawl of the demo accounts: the staging account's role lacks GetMetricData in one region. */
export function getAwsAccountStatus(id: string): Promise<AwsAccountStatus> {
  const account = db().find((a) => a.id === id) ?? fail('AWS account not found');
  const now = Date.now();
  const regions = account.attributes.aws_regions.include_only ?? STANDARD_REGIONS;
  return delay({
    regions: regions.map((region, i) => {
      const denied = account.attributes.aws_account_id === '123456789013' && i === 0;
      return {
        scope: region,
        last_success_at: denied ? null : now - (2 + i) * 60_000,
        collected_until: denied ? null : now - (7 + i) * 60_000,
        metrics: denied ? 0 : 140 - i * 23,
        points: denied ? 0 : (140 - i * 23) * 30,
        error: denied
          ? {
              kind: 'access_denied',
              message: `User: arn:aws:sts::123456789013:assumed-role/ObserveCraftIntegrationRole/observecraft is not authorized to perform: cloudwatch:GetMetricData`,
              missing_action: 'cloudwatch:GetMetricData',
              at: now - 4 * 60_000,
            }
          : null,
      };
    }),
  });
}
