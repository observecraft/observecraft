import type { components } from '../generated/schema';

// The AWS integration's wire contract: accounts as Datadog's API documents them
// (`/api/v2/integration/aws/accounts`, the same rows), and what the add-account flows need.

export interface AwsTagFilter {
  namespace: string;
  tags: string[];
}

export interface AwsAccountAttributes {
  aws_account_id: string;
  aws_partition: 'aws' | 'aws-cn' | 'aws-us-gov';
  account_tags: string[];
  /** Role (role_name + external_id) or keys (access_key_id; the secret is never answered). */
  auth_config: { role_name?: string; external_id?: string; access_key_id?: string };
  aws_regions: { include_all?: boolean; include_only?: string[] };
  logs_config: { lambda_forwarder: { lambdas: string[]; sources: string[]; log_source_config: { tag_filters: { source: string; tags: string[] }[] } } };
  metrics_config: {
    enabled: boolean;
    automute_enabled: boolean;
    collect_custom_metrics: boolean;
    collect_cloudwatch_alarms: boolean;
    tag_filters: AwsTagFilter[];
    namespace_filters: { include_only?: string[]; exclude_only?: string[] };
  };
  resources_config: { cloud_security_posture_management_collection: boolean; extended_collection: boolean };
  traces_config: { xray_services: { include_all?: boolean; include_only?: string[] } };
  created_at: string;
  modified_at: string;
}

export interface AwsAccount {
  id: string;
  attributes: AwsAccountAttributes;
}

type Sections = Omit<AwsAccountAttributes, 'created_at' | 'modified_at' | 'auth_config'>;

/** A create (aws_account_id and auth_config required) or an update: the sections sent replace their fields. */
export type AwsAccountInput = { [K in keyof Sections]?: Sections[K] extends unknown[] ? Sections[K] : Sections[K] extends object ? Partial<Sections[K]> : Sections[K] } & {
  auth_config?: AwsAccountAttributes['auth_config'] & { secret_access_key?: string };
};

export interface AwsNamespace {
  namespace: string;
  name: string;
  /** Catalog tile id, e.g. "aws-ec2". */
  id: string;
  categories: string[];
}

export interface AwsSetup {
  /** The AWS principal this install assumes roles with; null until the server sets OC_AWS_PRINCIPAL_ARN. */
  principal_arn: string | null;
  permissions: string[];
  default_excluded_namespaces: string[];
  namespaces: AwsNamespace[];
  log_services: { id: string; name: string }[];
}

/** The crawler's progress and last issue per region of an account. */
export type AwsAccountStatus = components['schemas']['AwsAccountStatus'];
export type ScopeStatus = components['schemas']['ScopeStatus'];
export type CrawlIssue = components['schemas']['CrawlIssue'];
