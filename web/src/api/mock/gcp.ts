// In-browser GCP integration for mock mode (Integrations → Google Cloud). Service accounts live in memory
// for the session; defaults follow the server (datadog_api/gcp_integration.rs).
import type { GcpAccount, GcpAccountInput, GcpAccountStatus, GcpConfig, GcpSetup } from '../gcp/types';

const delay = <T>(value: T, ms = 120) => new Promise<T>((r) => setTimeout(() => r(structuredClone(value)), ms));
const fail = (message: string): never => {
  throw new Error(message);
};

const DEFAULTS: GcpConfig = {
  automute: false,
  account_tags: [],
  host_filters: [],
  cloud_run_revision_filters: [],
  region_filter_configs: [],
  monitored_resource_configs: [],
  metric_namespace_configs: [],
  is_cspm_enabled: false,
  is_security_command_center_enabled: false,
  is_resource_change_collection_enabled: false,
  is_per_project_quota_enabled: false,
  resource_collection_enabled: false,
  is_org_folder_resource_collection_enabled: false,
  is_global_location_enabled: true,
};

let accounts: GcpAccount[] | null = null;

function db(): GcpAccount[] {
  accounts ??= [
    {
      id: '5e4d3c2b-1a09-4f8e-9d7c-6b5a4f3e2d1c',
      client_email: 'observecraft-integration@example-storefront.iam.gserviceaccount.com',
      config: { ...DEFAULTS, account_tags: ['team:storefront'], monitored_resource_configs: [{ type: 'gce_instance', filters: ['env:prod'] }] },
    },
  ];
  return accounts;
}

const find = (id: string) => db().find((a) => a.id === id) ?? fail('GCP service account not found');

function config(input: GcpAccountInput, base: GcpConfig): GcpConfig {
  const next = { ...base };
  for (const key of Object.keys(DEFAULTS) as (keyof GcpConfig)[]) if (input[key] !== undefined) (next as Record<string, unknown>)[key] = input[key];
  if (next.is_cspm_enabled && !next.resource_collection_enabled) fail('Resource Collection must be enabled for CSM to be enabled');
  return next;
}

export const getGcpSetup = (): Promise<GcpSetup> => delay({ principal_email: 'observecraft@example-host.iam.gserviceaccount.com' });
export const listGcpAccounts = (): Promise<GcpAccount[]> => delay(db());

export function createGcpAccount(input: GcpAccountInput): Promise<GcpAccount> {
  const email = input.client_email?.trim() || fail('client_email is required');
  if (db().some((a) => a.client_email === email)) fail('a GCP service account with this client email already exists');
  const account = { id: crypto.randomUUID(), client_email: email, config: config(input, DEFAULTS) };
  db().push(account);
  return delay(account, 300);
}

export function updateGcpAccount(id: string, input: GcpAccountInput): Promise<GcpAccount> {
  const a = find(id);
  a.config = config(input, a.config);
  return delay(a, 200);
}

export function deleteGcpAccount(id: string): Promise<void> {
  accounts = db().filter((a) => a.id !== id);
  return delay(undefined, 200);
}

/** Two projects collected; Cloud Monitoring is disabled in a third. */
export function getGcpAccountStatus(id: string): Promise<GcpAccountStatus> {
  find(id);
  const now = Date.now();
  const ok = (scope: string, metrics: number) => ({ scope, last_success_at: now - 4 * 60_000, collected_until: now - 9 * 60_000, metrics, points: metrics * 20, error: null });
  return delay({
    projects: [
      { scope: '', last_success_at: now - 30 * 60_000, collected_until: null, metrics: 3, points: 0, error: null },
      ok('example-storefront', 212),
      ok('example-catalog', 57),
      {
        scope: 'example-sandbox',
        last_success_at: null,
        collected_until: null,
        metrics: 0,
        points: 0,
        error: { kind: 'disabled', message: 'Cloud Monitoring API has not been used in project example-sandbox before or it is disabled.', missing_action: null, at: now - 40 * 60_000 },
      },
    ],
  });
}
