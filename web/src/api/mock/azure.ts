// In-browser Azure integration for mock mode (Integrations → Microsoft Azure). App registrations live in
// memory for the session; defaults follow the server (datadog_api/azure_integration.rs).
import type { AzureAccount, AzureAccountInput, AzureAccountStatus, AzureConfig } from '../azure/types';

const delay = <T>(value: T, ms = 120) => new Promise<T>((r) => setTimeout(() => r(structuredClone(value)), ms));
const fail = (message: string): never => {
  throw new Error(message);
};

const DEFAULTS: AzureConfig = {
  automute: false,
  resource_collection_enabled: false,
  cspm_enabled: false,
  custom_metrics_enabled: false,
  host_filters: '',
  container_app_filters: '',
  app_service_plan_filters: '',
  metrics_enabled: true,
  metrics_enabled_default: true,
  usage_metrics_enabled: true,
  secretless_auth_enabled: false,
  resource_provider_configs: [],
};

let accounts: AzureAccount[] | null = null;

function db(): AzureAccount[] {
  accounts ??= [
    {
      id: '3c2b1a09-8f7e-4d6c-9b5a-4f3e2d1c0b9a',
      tenant_name: '7d3e5f1a-2b4c-4d6e-8f0a-1b2c3d4e5f60',
      client_id: '9a8b7c6d-5e4f-4a3b-8c2d-1e0f9a8b7c6d',
      has_secret: true,
      config: { ...DEFAULTS, host_filters: 'env:prod', automute: true },
    },
  ];
  return accounts;
}

const find = (id: string) => db().find((a) => a.id === id) ?? fail('Azure app registration not found');

export const listAzureAccounts = (): Promise<AzureAccount[]> => delay(db());

/** Whether the mock has an account: the catalog tile is installed, as the server counts one. */
export const azureConnected = (): boolean => db().length > 0;

function config(input: AzureAccountInput, base: AzureConfig): AzureConfig {
  const next = { ...base };
  for (const key of Object.keys(DEFAULTS) as (keyof AzureConfig)[]) if (input[key] !== undefined) (next as Record<string, unknown>)[key] = input[key];
  return next;
}

export function createAzureAccount(input: AzureAccountInput): Promise<AzureAccount> {
  if (!input.tenant_name?.trim() || !input.client_id?.trim()) fail('tenant_name and client_id are required');
  const existing = db().find((a) => a.tenant_name === input.tenant_name && a.client_id === input.client_id);
  const account: AzureAccount = {
    id: existing?.id ?? crypto.randomUUID(),
    tenant_name: input.tenant_name!.trim(),
    client_id: input.client_id!.trim(),
    has_secret: !!input.client_secret || !!existing?.has_secret,
    config: config(input, DEFAULTS),
  };
  accounts = [...db().filter((a) => a.id !== account.id), account];
  return delay(account, 300);
}

export function updateAzureAccount(id: string, input: AzureAccountInput): Promise<AzureAccount> {
  const a = find(id);
  Object.assign(a, { tenant_name: input.tenant_name ?? a.tenant_name, client_id: input.client_id ?? a.client_id, has_secret: a.has_secret || !!input.client_secret, config: config(input, a.config) });
  return delay(a, 200);
}

export function deleteAzureAccount(id: string): Promise<void> {
  accounts = db().filter((a) => a.id !== id);
  return delay(undefined, 200);
}

/** Two subscriptions collected; the Monitoring Reader role is missing on a third. */
export function getAzureAccountStatus(id: string): Promise<AzureAccountStatus> {
  find(id);
  const now = Date.now();
  const ok = (scope: string, metrics: number) => ({ scope, last_success_at: now - 3 * 60_000, collected_until: now - 8 * 60_000, metrics, points: metrics * 30, error: null });
  return delay({
    subscriptions: [
      { scope: '', last_success_at: now - 20 * 60_000, collected_until: null, metrics: 3, points: 0, error: null },
      ok('1f2e3d4c-5b6a-4978-8a9b-0c1d2e3f4a5b', 64),
      ok('2a3b4c5d-6e7f-4809-9a1b-2c3d4e5f6a7b', 18),
      {
        scope: '3b4c5d6e-7f80-4912-8b2c-3d4e5f6a7b8c',
        last_success_at: null,
        collected_until: null,
        metrics: 0,
        points: 0,
        error: {
          kind: 'access_denied',
          message: "The client does not have authorization to perform action 'Microsoft.Insights/metrics/read' over scope '/subscriptions/3b4c5d6e-7f80-4912-8b2c-3d4e5f6a7b8c'",
          missing_action: 'Microsoft.Insights/metrics/read',
          at: now - 5 * 60_000,
        },
      },
    ],
  });
}
