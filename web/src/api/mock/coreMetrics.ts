// Mock metrics of the integrations-core dashboards the mock shows installed (coreDashboards.generated.json) and of
// the demo accounts' AWS, Azure and Google Cloud services (cloudServiceDashboards.generated.json):
// every metric they chart, on a few fictional instances per integration tagged by the dashboards' template
// variables, sized by the metric's unit from metadata.csv.
import core from '../integrations/coreDashboards.generated.json' with { type: 'json' };
import cloudServices from '../integrations/cloudServiceDashboards.generated.json' with { type: 'json' };

type Tags = Record<string, string>;

export interface CoreMetricDef {
  set: string;
  description: string;
  unit?: string;
  base: (e: Tags, r: () => number) => number;
  noise: number;
  integer?: boolean;
  max?: number;
}

const HOSTS = ['i-0a1b2c3d4e5f60001', 'i-0a1b2c3d4e5f60002', 'i-0a1b2c3d4e5f60003', 'i-0a1b2c3d4e5f60006'];
/** Values of common tags, from the e-commerce demo; other tags get `<tag>-<n>`. */
const VALUES: Record<string, string[]> = {
  db: ['orders', 'catalog', 'billing', 'inventory'],
  table: ['orders', 'order_items', 'products', 'invoices'],
  schema: ['public', 'public', 'billing', 'catalog'],
  service: ['order-service', 'catalog-service', 'billing', 'inventory-service'],
  topic: ['orders', 'payments', 'inventory-updates', 'catalog-changes'],
  queue: ['orders', 'emails', 'payments', 'webhooks'],
  env: ['prod', 'prod', 'prod', 'qa'],
  // The AWS services' dimensions (cloudServiceDashboards.generated.json): account 123456789012's demo resources.
  distributionid: ['E1A2B3C4D5E6F7', 'E2B3C4D5E6F7A8', 'E3C4D5E6F7A8B9', 'E4D5E6F7A8B9C0'],
  webacl: ['storefront-cdn', 'api-gateway-alb', 'checkout-alb', 'storefront-cdn'],
  rule: ['ALL', 'AWS-AWSManagedRulesCommonRuleSet', 'ALL', 'rate-limit-checkout'],
  rulegroup: ['AWSManagedRulesCommonRuleSet', 'AWSManagedRulesKnownBadInputsRuleSet', 'AWSManagedRulesSQLiRuleSet', 'AWSManagedRulesCommonRuleSet'],
  clustername: ['storefront-prod', 'storefront-prod', 'storefront-prod', 'storefront-staging'],
  servicename: ['order-service', 'catalog-service', 'billing', 'inventory-service'],
  taskdefinitionfamily: ['order-service', 'catalog-service', 'billing', 'inventory-service'],
  dbinstanceidentifier: ['orders-db', 'catalog-db', 'billing-db', 'inventory-db'],
  dbclusteridentifier: ['orders-aurora', 'orders-aurora', 'catalog-aurora', 'catalog-aurora'],
  proxyname: ['orders-proxy', 'catalog-proxy', 'billing-proxy', 'orders-proxy'],
  functionname: ['order-confirmation-email', 'image-resize', 'inventory-sync', 'payment-webhook'],
  queuename: ['order-events', 'email-notifications', 'inventory-updates', 'order-events-dlq'],
  instanceid: ['i-0a1b2c3d4e5f60000', 'i-0a1b2c3d4e5f60001', 'i-0a1b2c3d4e5f60002', 'i-0a1b2c3d4e5f60003'],
  bucketname: ['example-storefront-assets', 'example-order-exports', 'example-logs-archive', 'example-catalog-images'],
  filterid: ['EntireBucket', 'EntireBucket', 'EntireBucket', 'EntireBucket'],
  storagetype: ['StandardStorage', 'StandardIAStorage', 'StandardStorage', 'GlacierStorage'],
  loadbalancer: ['app/storefront-alb/50dc6c495c0c9188', 'app/api-gateway-alb/8a2b1c9d7e6f5a40', 'app/billing-alb/1f2e3d4c5b6a7980', 'app/storefront-alb/50dc6c495c0c9188'],
  targetgroup: ['targetgroup/storefront-web/6d0ecf831eec9f09', 'targetgroup/api-gateway/943f017f100becff', 'targetgroup/billing/0a1b2c3d4e5f6a7b', 'targetgroup/storefront-api/1b2c3d4e5f6a7b8c'],
};
/** Values of a tag that differ for one integration: CloudFront's functions aren't Lambda's, Azure resources have names. */
const LOGO_VALUES: Record<string, Record<string, string[]>> = {
  'tile:aws-cloudfront': { functionname: ['storefront-headers', 'cart-redirects', 'storefront-headers', 'image-rewrite'] },
  'tile:azure-compute-virtualmachines': { name: ['storefront-api-vm', 'order-service-vm', 'catalog-service-vm', 'billing-vm'] },
  'tile:azure-web-sites': { name: ['storefront-web', 'api-gateway', 'order-functions', 'storefront-web'] },
  'tile:azure-sql-servers-databases': { name: ['orders', 'catalog', 'billing', 'orders'] },
  'tile:azure-storage-storageaccounts': { name: ['storefrontassets', 'orderexports', 'storefrontassets', 'orderexports'] },
  'tile:azure-keyvault-vaults': {
    name: ['storefront-secrets', 'billing-keys', 'order-service-kv', 'storefront-secrets'],
    statuscode: ['200', '200', '429', '401'],
  },
};
/** The Azure services' tags (cloudServiceDashboards.generated.json): Resource Health's status and the split dimensions. */
const AZURE_VALUES: Record<string, string[]> = {
  status: ['Running', 'Running', 'Degraded', 'Running'],
  region: ['westeurope', 'westeurope', 'northeurope', 'westeurope'],
  resource_group: ['storefront', 'storefront', 'billing', 'storefront'],
  subscription_id: ['9f1c2a7e-4b3d-4e8a-9c21-0d5e6f7a8b90', '9f1c2a7e-4b3d-4e8a-9c21-0d5e6f7a8b90', '9f1c2a7e-4b3d-4e8a-9c21-0d5e6f7a8b90', '9f1c2a7e-4b3d-4e8a-9c21-0d5e6f7a8b90'],
  instance: ['lw0sdlwk0000A1', 'lw0sdlwk0000B2', 'lw0sdlwk0000C3', 'lw0sdlwk0000D4'],
  apiname: ['GetBlob', 'PutBlob', 'ListBlobs', 'GetBlobProperties'],
  responsetype: ['Success', 'Success', 'ClientOtherError', 'Success'],
  activitytype: ['secret', 'key', 'certificate', 'authentication'],
  activityname: ['secretget', 'keysign', 'certificateget', 'authentication'],
};
/** The Google Cloud services' tags (cloudServiceDashboards.generated.json): project example-storefront's resources and the metric labels they split by. */
const GCP_VALUES: Record<string, string[]> = {
  project_id: ['example-storefront', 'example-storefront', 'example-storefront', 'example-storefront'],
  region: ['europe-west1', 'europe-west1', 'us-central1', 'europe-west1'],
  zone: ['europe-west1-b', 'europe-west1-c', 'us-central1-a', 'europe-west1-b'],
  location: ['europe-west1', 'europe-west1', 'us-central1', 'europe-west1'],
  instance_id: ['7123456789012300001', '7123456789012300002', '7123456789012300003', '7123456789012300004'],
  instance_name: ['storefront-web-1', 'storefront-web-2', 'order-worker-1', 'catalog-batch-1'],
  instance_group_name: ['storefront-web-mig', 'storefront-web-mig', 'order-worker-mig', 'catalog-batch-mig'],
  device_name: ['persistent-disk-0', 'persistent-disk-0', 'persistent-disk-1', 'persistent-disk-0'],
  database_id: ['example-storefront:orders', 'example-storefront:catalog', 'example-storefront:billing', 'example-storefront:orders-replica'],
  database: ['orders', 'catalog', 'billing', 'orders'],
  data_type: ['data', 'binlog', 'tmp_data', 'data'],
  service_name: ['storefront-web', 'checkout-api', 'recommendations', 'storefront-web'],
  revision_name: ['storefront-web-00042-kax', 'checkout-api-00017-pod', 'recommendations-00008-zuv', 'storefront-web-00041-qir'],
  response_code_class: ['2xx', '2xx', '4xx', '5xx'],
  state: ['active', 'active', 'idle', 'active'],
  bucket_name: ['example-storefront-assets', 'example-order-exports', 'example-logs-archive', 'example-catalog-images'],
  storage_class: ['STANDARD', 'NEARLINE', 'STANDARD', 'COLDLINE'],
  method: ['ReadObject', 'WriteObject', 'ListObjects', 'GetObjectMetadata'],
  topic_id: ['order-events', 'payment-events', 'inventory-updates', 'order-events'],
  subscription_id: ['order-events-billing', 'payment-events-ledger', 'inventory-updates-catalog', 'order-events-email'],
  delivery_type: ['pull', 'push', 'pull', 'streaming pull'],
  response_class: ['success', 'success', 'internal', 'success'],
};
const GCP_LOGO_VALUES: Record<string, Record<string, string[]>> = {
  'tile:gcp-gcs': { response_code: ['OK', 'OK', 'NOT_FOUND', 'PERMISSION_DENIED'] },
  'tile:gcp-pubsub': { response_code: ['success', 'success', 'unavailable', 'success'] },
};
const valuesOf = (logo: string, tag: string) =>
  LOGO_VALUES[logo]?.[tag] ?? GCP_LOGO_VALUES[logo]?.[tag] ?? (logo.startsWith('tile:azure-') ? AZURE_VALUES[tag] : logo.startsWith('tile:gcp-') ? GCP_VALUES[tag] : undefined) ?? VALUES[tag];

/** Sized by unit: fractions and percentages in range, bytes in megabytes to gigabytes, times in milliseconds. */
function base(unit: string | undefined): Pick<CoreMetricDef, 'base' | 'max' | 'integer'> {
  const u = unit?.split('/')[0];
  if (u === 'fraction') return { base: (_e, r) => 0.05 + r() * 0.6, max: 1 };
  if (u === 'percent') return { base: (_e, r) => 5 + r() * 60, max: 100 };
  if (u && /byte$/.test(u)) return { base: (_e, r) => 2e6 + Math.pow(r(), 2) * 4e9 };
  if (u && /(second|millisecond|microsecond|nanosecond)$/.test(u)) return { base: (_e, r) => (u === 'second' ? 0.01 : 10) + r() * (u === 'second' ? 2 : 400) };
  if (u && /^(task|instance|service|container|node)$/.test(u)) return { base: (_e, r) => 1 + r() * 12, integer: true };
  return { base: (_e, r) => 1 + Math.pow(r(), 2) * 900, integer: true };
}

type Dashboard = { logo: string; templateVariables: { prefix?: string }[]; widgets: unknown };

export const CORE_SETS: Record<string, Tags[]> = {};
export const CORE_METRICS: Record<string, CoreMetricDef> = {};

const dashboards = [...(core as { dashboards: Dashboard[] }).dashboards, ...(cloudServices as { dashboards: Dashboard[] }).dashboards];
// One set of instances per integration, tagged by every template variable of its dashboards.
const tagsOf = new Map<string, Set<string>>();
for (const d of dashboards) {
  const tags = tagsOf.get(d.logo) ?? new Set(['host']);
  for (const v of d.templateVariables) if (v.prefix) tags.add(v.prefix);
  // The tags its queries split by or keep (`by {rule}`, `rule:ALL`), so those charts have data too.
  const text = JSON.stringify(d.widgets);
  for (const [, keys] of text.matchAll(/ by \{([a-z0-9_.,]+)\}/g)) for (const k of keys.split(',')) tags.add(k);
  for (const [, filters] of text.matchAll(/:[a-z][a-z0-9_.]*\{([^}]*)\}/g)) for (const f of filters.split(',')) if (/^!?[a-z][a-z0-9_]*:/.test(f)) tags.add(f.replace(/^!/, '').split(':')[0]);
  tagsOf.set(d.logo, tags);
}
for (const [logo, tags] of tagsOf) CORE_SETS[`core_${logo}`] = HOSTS.map((host, i) => Object.fromEntries([...tags].map((t) => [t, t === 'host' ? host : (valuesOf(logo, t)?.[i] ?? `${t}-${i + 1}`)])));

for (const d of dashboards)
  for (const [, metric] of JSON.stringify(d.widgets).matchAll(/[a-z]+:([a-z][a-z0-9_]*\.[a-z0-9_.]+)\{/g)) {
    if (CORE_METRICS[metric]) continue;
    const unit = (core.units as Record<string, string>)[metric] ?? (cloudServices.units as Record<string, string>)[metric];
    // An Azure resource count (azure.<type>.count) is one series per resource, valued 1; availability metrics are 0 or 1.
    // Google Cloud's up and health gauges are 0 or 1, 1 when well.
    const one =
      /^azure\.[a-z0-9_]+\.count$/.test(metric) ||
      metric.endsWith('.vm_availability_metric') ||
      /^gcp\..*\.(up|healthy|instance_available|tunnel_established|operational|available_for_failover)$/.test(metric);
    // Ratios Cloud Monitoring reports without a unit stay between 0 and 1.
    const ratio = /^gcp\..*(hit_ratio|usage_ratio)$/.test(metric);
    // Availability percentages stay close to 100.
    const available = unit === 'percent' && /availability$/.test(metric);
    CORE_METRICS[metric] = one
      ? { set: `core_${d.logo}`, description: metric, unit, noise: 0, base: () => 1, integer: true }
      : ratio
        ? { set: `core_${d.logo}`, description: metric, unit, noise: 0.02, base: (_e, r) => 0.85 + r() * 0.14, max: 1 }
        : available
          ? { set: `core_${d.logo}`, description: metric, unit, noise: 0.001, base: (_e, r) => 99.5 + r() * 0.5, max: 100 }
          : { set: `core_${d.logo}`, description: metric, unit, noise: 0.2, ...base(unit) };
  }
