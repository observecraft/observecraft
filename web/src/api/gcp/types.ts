import type { components } from '../generated/schema';

// The GCP integration's wire contract: service accounts with Datadog's attributes
// (`/api/v2/integration/gcp/accounts`, the same rows), this install's service account, and the crawl per project.

export interface GcpConfig {
  automute: boolean;
  account_tags: string[];
  host_filters: string[];
  cloud_run_revision_filters: string[];
  region_filter_configs: string[];
  monitored_resource_configs: { type: string; filters: string[] }[];
  metric_namespace_configs: { id: string; disabled?: boolean; filters: string[] }[];
  is_cspm_enabled: boolean;
  is_security_command_center_enabled: boolean;
  is_resource_change_collection_enabled: boolean;
  is_per_project_quota_enabled: boolean;
  resource_collection_enabled: boolean;
  is_org_folder_resource_collection_enabled: boolean;
  is_global_location_enabled: boolean;
}

export interface GcpAccount {
  id: string;
  client_email: string;
  config: GcpConfig;
}

export type GcpAccountInput = Partial<GcpConfig> & { client_email?: string };

export interface GcpService {
  prefix: string;
  short: string;
  name: string;
  id: string;
  categories: string[];
}

export type GcpSetup = components['schemas']['GcpSetup'];
export type GcpAccountStatus = components['schemas']['GcpAccountStatus'];
