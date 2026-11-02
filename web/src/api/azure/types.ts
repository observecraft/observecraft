import type { components } from '../generated/schema';

// The Azure integration's wire contract: app registrations with Datadog's fields
// (`/api/v1/integration/azure`, the same rows), and the crawl status per subscription.

export interface AzureConfig {
  automute: boolean;
  resource_collection_enabled: boolean;
  cspm_enabled: boolean;
  custom_metrics_enabled: boolean;
  host_filters: string;
  container_app_filters: string;
  app_service_plan_filters: string;
  metrics_enabled: boolean;
  metrics_enabled_default: boolean;
  usage_metrics_enabled: boolean;
  secretless_auth_enabled: boolean;
  resource_provider_configs: { namespace: string; metrics_enabled: boolean }[];
}

export interface AzureAccount {
  id: string;
  tenant_name: string;
  client_id: string;
  /** A client secret is stored (it is never returned). */
  has_secret: boolean;
  config: AzureConfig;
}

/** A create (tenant, client id and secret) or the fields to change. */
export type AzureAccountInput = Partial<AzureConfig> & { tenant_name?: string; client_id?: string; client_secret?: string };

export interface AzureResourceType {
  type: string;
  name: string;
  id: string;
  prefix: string;
  categories: string[];
}

export type AzureAccountStatus = components['schemas']['AzureAccountStatus'];
