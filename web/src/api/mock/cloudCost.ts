// Mock Cloud Cost: the fictional bills the server's demo seed writes (routes/cloud_cost.rs, DEMO_LINES),
// a year of AWS, Azure, Google Cloud and SaaS costs per day, queried as the server queries them.
import type {
  BudgetDoc,
  CommitmentsRequest,
  CommitmentsResult,
  CostBudgetSummary,
  CostQueryRequest,
  CostQueryResult,
  CostTag,
  CostTagsRequest,
  UnitCostRow,
  UnitCostsRequest,
} from '../cloudCost/client';
import { parseQuery } from '../../lib/searchQuery';
import { budgetScope } from '../cloudCost/budgetQuery';

const DAY = 86_400_000;

type Line = [provider: string, service: string, account: string, region: string, base: number, weekend: number, desc: string, tags: Record<string, string>];

const LINES: Line[] = [
  ['aws', 'AmazonEC2', 'ecommerce-prod', 'us-east-1', 1400, 1, 'EC2 instance usage', { aws_product: 'ec2', aws_instance_family: 'm6i', team: 'platform', service: 'storefront-api', env: 'prod' }],
  [
    'aws',
    'AmazonEC2',
    'ecommerce-prod',
    'us-east-1',
    420,
    1,
    'EC2 GPU instance usage',
    { aws_product: 'ec2', aws_instance_family: 'g5', gpu_type: 'a10g', team: 'ml', service: 'catalog-service', env: 'prod' },
  ],
  [
    'aws',
    'AmazonEC2',
    'ecommerce-staging',
    'us-east-1',
    380,
    0.55,
    'EC2 instance usage',
    { aws_product: 'ec2', aws_instance_family: 'm6i', team: 'platform', service: 'storefront-api', env: 'staging' },
  ],
  ['aws', 'AmazonRDS', 'ecommerce-prod', 'us-east-1', 620, 1, 'RDS database instances', { aws_product: 'rds', team: 'checkout', service: 'order-service', env: 'prod' }],
  ['aws', 'AmazonS3', 'ecommerce-prod', 'us-east-1', 210, 1, 'S3 storage and requests', { aws_product: 's3', storage_class: 'standard', team: 'platform', service: 'catalog-service', env: 'prod' }],
  ['aws', 'AmazonS3', 'ecommerce-prod', 'us-east-1', 45, 1, 'S3 Glacier storage', { aws_product: 's3', storage_class: 'glacier', team: 'data', service: 'billing', env: 'prod' }],
  [
    'aws',
    'AWSDataTransfer',
    'ecommerce-prod',
    'us-east-1',
    70,
    0.9,
    'Data transfer between availability zones',
    { aws_product: 'datatransfer', data_transfer_type: 'cross-zone', team: 'platform', env: 'prod' },
  ],
  [
    'aws',
    'AWSDataTransfer',
    'ecommerce-prod',
    'us-east-1',
    40,
    0.9,
    'Data transfer between regions',
    { aws_product: 'datatransfer', data_transfer_type: 'cross-region', team: 'platform', env: 'prod' },
  ],
  [
    'aws',
    'AWSDataTransfer',
    'ecommerce-prod',
    'us-east-1',
    48,
    0.8,
    'Data transfer out to the internet',
    { aws_product: 'datatransfer', data_transfer_type: 'internet', team: 'storefront', service: 'api-gateway', env: 'prod' },
  ],
  ['aws', 'ElasticLoadBalancing', 'ecommerce-prod', 'us-east-1', 85, 1, 'Load balancer hours and capacity units', { aws_product: 'elb', team: 'platform', service: 'api-gateway', env: 'prod' }],
  ['aws', 'AmazonVPC', 'ecommerce-prod', 'us-east-1', 95, 1, 'NAT gateway hours and data processed', { aws_product: 'natgateway', team: 'platform', env: 'prod' }],
  ['aws', 'AmazonEKS', 'ecommerce-prod', 'us-east-1', 70, 1, 'EKS cluster hours', { aws_product: 'eks', team: 'platform', env: 'prod' }],
  ['aws', 'AmazonDynamoDB', 'ecommerce-prod', 'us-east-1', 140, 0.9, 'DynamoDB read and write capacity', { aws_product: 'dynamodb', team: 'fulfillment', service: 'inventory-service', env: 'prod' }],
  ['aws', 'AWSLambda', 'ecommerce-prod', 'us-east-1', 55, 0.8, 'Lambda requests and duration', { aws_product: 'lambda', team: 'payments', service: 'billing', env: 'prod' }],
  [
    'aws',
    'AmazonCloudFront',
    'ecommerce-prod',
    'us-east-1',
    120,
    1.1,
    'CloudFront data transfer and requests',
    { aws_product: 'cloudfront', team: 'storefront', service: 'storefront-api', env: 'prod' },
  ],
  [
    'aws',
    'AmazonBedrock',
    'ecommerce-prod',
    'us-east-1',
    180,
    0.7,
    'Model inference tokens',
    { aws_product: 'bedrock', ai_vendor: 'Amazon Bedrock', ai_model: 'chat-large', team: 'ml', service: 'catalog-service', env: 'prod' },
  ],
  ['azure', 'VirtualMachines', 'ecommerce-eu', 'westeurope', 520, 1, 'Virtual machine compute hours', { metercategory: 'Virtual Machines', team: 'platform', service: 'storefront-api', env: 'prod' }],
  [
    'azure',
    'VirtualMachines',
    'ecommerce-eu',
    'westeurope',
    260,
    1,
    'NC-series GPU virtual machines',
    { metercategory: 'Virtual Machines', gpu_type: 't4', team: 'ml', service: 'catalog-service', env: 'prod' },
  ],
  ['azure', 'Storage', 'ecommerce-eu', 'westeurope', 78, 1, 'Blob storage capacity and operations', { metercategory: 'Storage', metersubcategory: 'Blob Storage', team: 'platform', env: 'prod' }],
  ['azure', 'Storage', 'ecommerce-eu', 'westeurope', 6, 1, 'File shares', { metercategory: 'Storage', metersubcategory: 'Files', team: 'platform', env: 'prod' }],
  ['azure', 'Storage', 'ecommerce-eu', 'westeurope', 2.5, 1, 'Tiered block blob storage', { metercategory: 'Storage', metersubcategory: 'Tiered Block Blob', team: 'data', env: 'prod' }],
  ['azure', 'Storage', 'ecommerce-eu', 'westeurope', 0.6, 1, 'Queue operations', { metercategory: 'Storage', metersubcategory: 'Queues v2', team: 'fulfillment', env: 'prod' }],
  ['azure', 'Storage', 'ecommerce-eu', 'westeurope', 0.25, 1, 'Table storage', { metercategory: 'Storage', metersubcategory: 'Tables', team: 'fulfillment', env: 'prod' }],
  ['azure', 'SQLDatabase', 'ecommerce-eu', 'westeurope', 210, 1, 'SQL Database vCore hours', { metercategory: 'SQL Database', team: 'checkout', service: 'order-service', env: 'prod' }],
  ['azure', 'LogAnalytics', 'ecommerce-eu', 'westeurope', 140, 0.9, 'Log data ingestion', { metercategory: 'Log Analytics', team: 'platform', env: 'prod' }],
  [
    'azure',
    'FoundryModels',
    'ecommerce-eu',
    'westeurope',
    900,
    0.6,
    'Model inference tokens',
    { metercategory: 'Foundry Models', ai_vendor: 'Azure AI Foundry', ai_model: 'chat-large', team: 'ml', service: 'storefront-api', env: 'prod' },
  ],
  [
    'azure',
    'FoundryModels',
    'ecommerce-eu',
    'westeurope',
    120,
    0.6,
    'Embedding tokens',
    { metercategory: 'Foundry Models', ai_vendor: 'Azure AI Foundry', ai_model: 'embed-small', team: 'ml', service: 'catalog-service', env: 'prod' },
  ],
  ['azure', 'Bandwidth', 'ecommerce-eu', 'westeurope', 35, 0.9, 'Data transfer out', { metercategory: 'Bandwidth', data_transfer_type: 'internet', team: 'storefront', env: 'prod' }],
  ['azure', 'AzureDevOps', 'ecommerce-eu', 'westeurope', 60, 0.3, 'Pipelines parallel jobs', { metercategory: 'Azure DevOps', team: 'platform', env: 'ci' }],
  ['gcp', 'BigQuery', 'ecommerce-analytics', 'us-central1', 380, 0.5, 'Analysis bytes scanned', { gcp_product: 'bigquery', team: 'data', service: 'billing', env: 'prod' }],
  ['gcp', 'Compute Engine', 'ecommerce-analytics', 'us-central1', 240, 1, 'N2 instance core and RAM hours', { gcp_product: 'compute', team: 'data', env: 'prod' }],
  ['gcp', 'Compute Engine', 'ecommerce-analytics', 'us-central1', 150, 1, 'A2 GPU instance hours', { gcp_product: 'compute', gpu_type: 'a100', team: 'ml', service: 'catalog-service', env: 'prod' }],
  ['gcp', 'Cloud Storage', 'ecommerce-analytics', 'us-central1', 75, 1, 'Standard storage', { gcp_product: 'storage', storage_class: 'standard', team: 'data', env: 'prod' }],
  ['gcp', 'Kubernetes Engine', 'ecommerce-analytics', 'europe-west1', 110, 1, 'GKE cluster management', { gcp_product: 'gke', team: 'platform', env: 'prod' }],
  [
    'gcp',
    'Vertex AI',
    'ecommerce-analytics',
    'us-central1',
    60,
    0.7,
    'Generative model tokens',
    { gcp_product: 'vertexai', ai_vendor: 'Vertex AI', ai_model: 'chat-medium', team: 'ml', service: 'catalog-service', env: 'prod' },
  ],
  ['gcp', 'Networking', 'ecommerce-analytics', 'us-central1', 40, 0.9, 'Inter-region egress', { gcp_product: 'networking', data_transfer_type: 'cross-region', team: 'data', env: 'prod' }],
  ['Snowflake', 'Compute', 'ecommerce-warehouse', 'us-east-1', 260, 0.5, 'Warehouse credits', { team: 'data', env: 'prod' }],
  ['Snowflake', 'Storage', 'ecommerce-warehouse', 'us-east-1', 30, 1, 'Storage', { team: 'data', env: 'prod' }],
  ['Confluent Cloud', 'Kafka Cluster', 'ecommerce-streaming', 'us-east-1', 140, 1, 'Dedicated cluster CKUs', { team: 'fulfillment', service: 'order-service', env: 'prod' }],
  ['GitHub', 'Actions', 'example-org', 'global', 45, 0.25, 'Actions minutes', { team: 'platform', env: 'ci' }],
  ['Fastly', 'CDN', 'ecommerce-edge', 'global', 55, 1.1, 'Edge bandwidth and requests', { team: 'storefront', service: 'storefront-api', env: 'prod' }],
  ['MongoDB', 'Atlas Cluster', 'ecommerce-catalog', 'us-east-1', 90, 1, 'Dedicated cluster hours', { team: 'catalog', service: 'catalog-service', env: 'prod' }],
  ['OpenAI', 'API', 'ecommerce-assistant', 'global', 110, 0.6, 'Model inference tokens', { ai_vendor: 'OpenAI', ai_model: 'chat-small', team: 'ml', service: 'storefront-api', env: 'prod' }],
];
const SAVINGS_PLAN_DAILY = 300;

/** How a line's usage is priced: [service, description, pricing unit, on-demand price per unit, tags]. */
const PRICES: [string, string, string, number, Record<string, string>][] = [
  ['AmazonEC2', 'EC2 instance usage', 'Hrs', 0.192, { aws_instance_type: 'm6i.xlarge', aws_usage_type: 'BoxUsage:m6i.xlarge', aws_operation: 'RunInstances' }],
  ['AmazonEC2', 'EC2 GPU instance usage', 'Hrs', 1.006, { aws_instance_type: 'g5.xlarge', aws_usage_type: 'BoxUsage:g5.xlarge', aws_operation: 'RunInstances' }],
  ['AmazonRDS', 'RDS database instances', 'Hrs', 0.52, { aws_instance_type: 'db.r6g.xlarge', aws_usage_type: 'InstanceUsage:db.r6g.xl', aws_operation: 'CreateDBInstance' }],
  ['AmazonS3', 'S3 storage and requests', 'GB-Mo', 0.023, { aws_usage_type: 'TimedStorage-ByteHrs' }],
  ['AmazonS3', 'S3 Glacier storage', 'GB-Mo', 0.004, { aws_usage_type: 'TimedStorage-GlacierByteHrs' }],
  ['AWSDataTransfer', 'Data transfer between availability zones', 'GB', 0.01, { aws_usage_type: 'DataTransfer-Regional-Bytes' }],
  ['AWSDataTransfer', 'Data transfer between regions', 'GB', 0.02, { aws_usage_type: 'USE1-USW2-AWS-Out-Bytes' }],
  ['AWSDataTransfer', 'Data transfer out to the internet', 'GB', 0.09, { aws_usage_type: 'DataTransfer-Out-Bytes' }],
  ['ElasticLoadBalancing', 'Load balancer hours and capacity units', 'LCU-Hrs', 0.008, { aws_usage_type: 'LoadBalancerUsage' }],
  ['AmazonVPC', 'NAT gateway hours and data processed', 'GB', 0.045, { aws_usage_type: 'NatGateway-Bytes' }],
  ['AmazonEKS', 'EKS cluster hours', 'Hrs', 0.1, { aws_usage_type: 'AmazonEKS-Hours:perCluster' }],
  ['AmazonDynamoDB', 'DynamoDB read and write capacity', 'WriteCapacityUnit-Hrs', 0.00065, { aws_usage_type: 'WriteCapacityUnit-Hrs' }],
  ['AWSLambda', 'Lambda requests and duration', 'Lambda-GB-Second', 0.0000166667, { aws_usage_type: 'Lambda-GB-Second' }],
  ['AmazonCloudFront', 'CloudFront data transfer and requests', 'GB', 0.085, { aws_usage_type: 'DataTransfer-Out-Bytes' }],
  ['AmazonBedrock', 'Model inference tokens', '1K tokens', 0.003, { aws_usage_type: 'USE1-InputTokenCount' }],
  [
    'VirtualMachines',
    'Virtual machine compute hours',
    '1 Hour',
    0.192,
    { productname: 'Virtual Machines Dsv5 Series', metername: 'D4s v5', additionalinfo_vcpus: '4', additionalinfo_servicetype: 'Standard_D4s_v5', consumedservice: 'Microsoft.Compute' },
  ],
  [
    'VirtualMachines',
    'NC-series GPU virtual machines',
    '1 Hour',
    0.526,
    { productname: 'Virtual Machines NCasT4 v3 Series', metername: 'NC4as T4 v3', additionalinfo_vcpus: '4', additionalinfo_servicetype: 'Standard_NC4as_T4_v3', consumedservice: 'Microsoft.Compute' },
  ],
  ['Storage', 'Blob storage capacity and operations', '1 GB/Month', 0.0184, { productname: 'Blob Storage', metername: 'Hot LRS Data Stored', consumedservice: 'Microsoft.Storage' }],
  ['Storage', 'File shares', '1 GB/Month', 0.06, { productname: 'Files v2', metername: 'Hot LRS Data Stored', consumedservice: 'Microsoft.Storage' }],
  ['Storage', 'Tiered block blob storage', '1 GB/Month', 0.01, { productname: 'Tiered Block Blob', metername: 'Cool LRS Data Stored', consumedservice: 'Microsoft.Storage' }],
  ['Storage', 'Queue operations', '10K', 0.004, { productname: 'Queues v2', metername: 'LRS Class 1 Operations', consumedservice: 'Microsoft.Storage' }],
  ['Storage', 'Table storage', '1 GB/Month', 0.045, { productname: 'Tables', metername: 'LRS Data Stored', consumedservice: 'Microsoft.Storage' }],
  ['SQLDatabase', 'SQL Database vCore hours', '1 Hour', 0.505, { productname: 'SQL Database Single General Purpose - Compute Gen5', metername: 'vCore', consumedservice: 'Microsoft.Sql' }],
  ['LogAnalytics', 'Log data ingestion', '1 GB', 2.76, { productname: 'Log Analytics', metername: 'Pay-as-you-go Data Ingestion', consumedservice: 'Microsoft.OperationalInsights' }],
  ['FoundryModels', 'Model inference tokens', '1M', 2.5, { productname: 'Foundry Models', metername: 'chat-large Input Tokens', consumedservice: 'Microsoft.CognitiveServices' }],
  ['FoundryModels', 'Embedding tokens', '1M', 0.02, { productname: 'Foundry Models', metername: 'embed-small Tokens', consumedservice: 'Microsoft.CognitiveServices' }],
  ['Bandwidth', 'Data transfer out', '1 GB', 0.087, { productname: 'Rtn Preference: MGN', metername: 'Standard Data Transfer Out', consumedservice: 'Microsoft.Network' }],
  ['AzureDevOps', 'Pipelines parallel jobs', '1/Day', 1.33, { productname: 'Azure Pipelines', metername: 'Microsoft-hosted CI/CD', consumedservice: 'Microsoft.VisualStudio' }],
  ['BigQuery', 'Analysis bytes scanned', 'tebibyte', 6.25, { gcp_sku_description: 'Analysis (us-central1)' }],
  ['Compute Engine', 'N2 instance core and RAM hours', 'hour', 0.0316, { gcp_sku_description: 'N2 Instance Core running in Americas', gcp_machine_type: 'n2-standard-4' }],
  ['Compute Engine', 'A2 GPU instance hours', 'hour', 2.93, { gcp_sku_description: 'Nvidia Tesla A100 GPU running in Americas', gcp_machine_type: 'a2-highgpu-1g' }],
  ['Cloud Storage', 'Standard storage', 'gibibyte month', 0.02, { gcp_sku_description: 'Standard Storage US Regional' }],
  ['Kubernetes Engine', 'GKE cluster management', 'hour', 0.1, { gcp_sku_description: 'Regional Kubernetes Clusters' }],
  ['Vertex AI', 'Generative model tokens', '1M tokens', 1.25, { gcp_sku_description: 'Generative model input tokens' }],
  ['Networking', 'Inter-region egress', 'gibibyte', 0.01, { gcp_sku_description: 'Network Inter Region Data Transfer Out from Americas to Europe' }],
  ['Compute', 'Warehouse credits', 'Credits', 3, {}],
  ['Storage', 'Storage', 'TB-Mo', 23, {}],
  ['Kafka Cluster', 'Dedicated cluster CKUs', 'CKU-Hours', 2.25, {}],
  ['Actions', 'Actions minutes', 'Minutes', 0.008, {}],
  ['CDN', 'Edge bandwidth and requests', 'GB', 0.12, {}],
  ['Atlas Cluster', 'Dedicated cluster hours', 'Hours', 2.08, {}],
  ['API', 'Model inference tokens', '1M tokens', 0.6, {}],
];
const priceOf = (service: string, desc: string) => PRICES.find((p) => p[0] === service && p[1] === desc);

interface Commitment {
  id: string;
  kind: string;
  service: string;
  description: string;
  account: string;
  daily: number;
  discount: number;
  term: number;
  upfront: boolean;
}

const COMMITMENTS: Commitment[] = [
  {
    id: 'arn:aws:savingsplans::123456789012:savingsplan/0a1b2c3d-4e5f-4000-8000-000000000001',
    kind: 'Savings Plan',
    service: 'AmazonEC2',
    description: 'EC2 instance usage',
    account: 'ecommerce-prod',
    daily: SAVINGS_PLAN_DAILY,
    discount: 0.28,
    term: 12,
    upfront: true,
  },
  {
    id: 'ri-0a1b2c3d4e5f60001',
    kind: 'Reserved Instance',
    service: 'AmazonRDS',
    description: 'RDS database instances',
    account: 'ecommerce-prod',
    daily: 300,
    discount: 0.4,
    term: 12,
    upfront: false,
  },
  {
    id: 'reservation/00000000-0000-4000-8000-000000000101',
    kind: 'Reserved Instance',
    service: 'VirtualMachines',
    description: 'Virtual machine compute hours',
    account: 'ecommerce-eu',
    daily: 230,
    discount: 0.36,
    term: 12,
    upfront: false,
  },
  {
    id: 'savingsplan/00000000-0000-4000-8000-000000000102',
    kind: 'Savings Plan',
    service: 'VirtualMachines',
    description: 'NC-series GPU virtual machines',
    account: 'ecommerce-eu',
    daily: 90,
    discount: 0.15,
    term: 36,
    upfront: false,
  },
  {
    id: 'projects/ecommerce-analytics/regions/us-central1/commitments/n2-commitment-1',
    kind: 'Committed Use Discount',
    service: 'Compute Engine',
    description: 'N2 instance core and RAM hours',
    account: 'ecommerce-analytics',
    daily: 110,
    discount: 0.37,
    term: 12,
    upfront: false,
  },
];

interface Row {
  day: number;
  billed: number;
  effective: number;
  list: number;
  quantity: number;
  category: string;
  fields: Record<string, string>;
}

function noise(day: number, line: number, salt: number): number {
  let x = Math.imul(day ^ 0x9e3779b9, 0x85ebca6b) ^ Math.imul(line + 1, 0xc2b2ae35) ^ salt;
  x ^= x >>> 16;
  x = Math.imul(x, 0x7feb352d);
  x ^= x >>> 15;
  return (x >>> 0) / 2 ** 31 - 1;
}

const today = () => Math.floor(Date.now() / DAY) * DAY;
let cache: { at: number; rows: Row[] } | null = null;

const isoDay = (ms: number) => new Date(ms).toISOString().slice(0, 10);

/** 400 days of bills up to yesterday, the same for a day (so pages agree). */
function bills(): Row[] {
  const now = today();
  if (cache?.at === now) return cache.rows;
  const rows: Row[] = [];
  const nd = new Date(now);
  // Commitments were bought 10 months ago: before that, the usage they cover ran on demand.
  const bought = Date.UTC(nd.getUTCFullYear(), nd.getUTCMonth() - 10, 1);
  for (let day = now - 400 * DAY; day < now; day += DAY) {
    const n = day / DAY;
    const age = (now - day) / DAY;
    const date = new Date(day);
    const weekend = date.getUTCDay() === 0 || date.getUTCDay() === 6;
    const trend = 1 - 0.18 * Math.min(age / 365, 1.5);
    const swing = 1 + 0.06 * Math.sin((n * Math.PI * 2) / 29);
    const push = (
      [provider, service, account, region, , , desc, tags]: Line,
      c: { billed: number; effective: number; list: number; quantity?: number; pricing?: string; commitment?: [Commitment, string] },
      category: string,
    ) => {
      const price = category === 'usage' ? priceOf(service, desc) : undefined;
      const cm = c.commitment?.[0];
      const bd = new Date(bought);
      rows.push({
        day,
        billed: c.billed,
        effective: c.effective,
        list: c.list,
        quantity: c.quantity ?? 0,
        category,
        fields: {
          ...tags,
          ...(price?.[4] ?? {}),
          ...(provider === 'aws' ? { aws_account: '123456789012' } : {}),
          ...(cm
            ? {
                commitment_term: cm.term % 12 === 0 ? `${cm.term / 12} year${cm.term === 12 ? '' : 's'}` : `${cm.term} months`,
                commitment_purchase_option: cm.upfront ? 'All Upfront' : 'No Upfront',
                commitment_start: isoDay(bought),
                commitment_end: isoDay(Date.UTC(bd.getUTCFullYear(), bd.getUTCMonth() + cm.term, 1)),
                commitmentdiscountid: cm.id,
                commitmentdiscounttype: cm.kind,
                commitmentdiscountstatus: c.commitment![1],
              }
            : {}),
          providername: provider,
          servicename: service,
          subaccountname: account,
          regionname: region,
          chargedescription: desc,
          chargecategory: category,
          ...(price ? { pricingunit: price[2] } : {}),
          ...(c.pricing ? { pricingcategory: c.pricing } : {}),
        },
      });
    };
    LINES.forEach((line, i) => {
      const [, service, account, , base, weekendFactor, desc] = line;
      let cost = base * trend * swing * (1 + 0.09 * noise(n, i, 1)) * (weekend ? weekendFactor : 1);
      if (service === 'Vertex AI' && age < 7) cost *= 9;
      if (desc === 'EC2 instance usage' && account === 'ecommerce-staging' && age < 7) cost *= 0.45;
      if (service === 'BigQuery' && noise(n, i, 2) > 0.85) cost *= 2.2;
      const unit = priceOf(service, desc)?.[3] ?? 0;
      const qty = (list: number) => (unit > 0 ? list / unit : 0);
      const cm = COMMITMENTS.find((c) => c.service === service && c.description === desc && c.account === account && day >= bought);
      if (cm) {
        // The commitment covers most of the usage, up to what it buys a day; the rest is on demand.
        const used = Math.min(cm.daily * (0.94 + 0.05 * noise(n, i, 3)), cost * 0.95);
        const unused = cm.daily - used;
        const covered = used / (1 - cm.discount);
        push(line, { billed: cm.upfront ? 0 : used, effective: used, list: covered, quantity: qty(covered), pricing: 'Committed', commitment: [cm, 'Used'] }, 'usage');
        push(line, { billed: cm.upfront ? 0 : unused, effective: unused, list: 0, pricing: 'Committed', commitment: [cm, 'Unused'] }, 'usage');
        cost -= used;
      }
      push(line, { billed: cost, effective: cost, list: cost, quantity: qty(cost), pricing: 'Standard' }, 'usage');
    });
    if (date.getUTCDate() === 1) {
      const days = new Date(Date.UTC(date.getUTCFullYear(), date.getUTCMonth() + 1, 1)).getTime() / DAY - n;
      push(
        ['aws', 'Savings Plans for AWS Compute usage', 'ecommerce-prod', 'global', 0, 1, 'Savings Plan upfront fee', { aws_product: 'savingsplans', team: 'platform', env: 'prod' }],
        { billed: SAVINGS_PLAN_DAILY * days, effective: 0, list: 0 },
        'purchase',
      );
      push(['azure', 'Tax', 'ecommerce-eu', 'westeurope', 0, 1, 'VAT', {}], { billed: 410 * trend, effective: 410 * trend, list: 410 * trend }, 'tax');
    }
    push(
      ['gcp', 'Compute Engine', 'ecommerce-analytics', 'us-central1', 0, 1, 'Sustained use discount', { gcp_product: 'compute', team: 'data', env: 'prod' }],
      { billed: -18 * trend, effective: -18 * trend, list: -18 * trend },
      'credit',
    );
  }
  cache = { at: now, rows };
  return rows;
}

const FOCUS = [
  'providername',
  'servicename',
  'subaccountname',
  'regionname',
  'availabilityzone',
  'chargecategory',
  'chargedescription',
  'resourceid',
  'pricingunit',
  'billingcurrency',
  'pricingcategory',
  'commitmentdiscountid',
  'commitmentdiscounttype',
  'commitmentdiscountstatus',
];
const field = (r: Row, key: string) => r.fields[FOCUS.includes(key.toLowerCase().replace(/_/g, '')) ? key.toLowerCase().replace(/_/g, '') : key] ?? '';
const glob = (p: string, v: string) =>
  new RegExp(
    `^${p
      .split('*')
      .map((x) => x.replace(/[.+?^${}()|[\]\\]/g, '\\$&'))
      .join('.*')}$`,
    'i',
  ).test(v);

function matcher(query: string) {
  const terms = parseQuery(query);
  return (r: Row) =>
    terms.every((t) =>
      t.kind === 'text'
        ? `${r.fields.servicename} ${r.fields.chargedescription} ${r.fields.providername} ${r.fields.subaccountname}`.toLowerCase().includes(t.text.toLowerCase()) !== t.negate
        : t.values.some((p) => (/^\*+$/.test(p) ? field(r, t.key) !== '' : glob(p, field(r, t.key)))) !== t.negate,
    );
}

const dayOf = (ms: number) => Math.floor(ms / DAY) * DAY;
const round = (v: number) => Math.round(v * 100) / 100;

function bucketOf(day: number, interval: CostQueryRequest['interval']): number {
  const d = new Date(day);
  if (interval === 'month') return Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), 1);
  if (interval === 'week') return day - ((d.getUTCDay() + 6) % 7) * DAY;
  return day;
}

function nextBucket(b: number, interval: CostQueryRequest['interval']): number {
  const d = new Date(b);
  if (interval === 'month') return Date.UTC(d.getUTCFullYear(), d.getUTCMonth() + 1, 1);
  return b + (interval === 'week' ? 7 : 1) * DAY;
}

export async function queryCosts(req: CostQueryRequest): Promise<CostQueryResult> {
  const from = dayOf(req.range.from);
  const to = dayOf(Math.max(req.range.to, req.range.from));
  const len = (to - from) / DAY + 1;
  const prevTo = from - DAY;
  const prevFrom = prevTo - (len - 1) * DAY;
  const match = matcher(req.query ?? '');
  const cost = (r: Row) => (req.costType === 'billed' ? r.billed : r.effective);
  const groupBy = req.groupBy ?? [];
  const buckets: number[] = [];
  for (let b = bucketOf(from, req.interval); b <= to; b = nextBucket(b, req.interval)) buckets.push(b);
  const index = new Map(buckets.map((b, i) => [b, i]));
  const groups = new Map<string, { tags: Record<string, string>; values: number[]; prev: number }>();
  const groupOf = (r: Row) => {
    const tags = Object.fromEntries(groupBy.map((k) => [k, field(r, k) || 'N/A']));
    const key = JSON.stringify(tags);
    let g = groups.get(key);
    if (!g) groups.set(key, (g = { tags, values: buckets.map(() => 0), prev: 0 }));
    return g;
  };
  let through = 0;
  for (const r of bills()) {
    if (r.day < prevFrom || r.day > to) continue;
    if (req.usageOnly && r.category !== 'usage') continue;
    if (!match(r)) continue;
    if (r.day >= from) {
      groupOf(r).values[index.get(bucketOf(r.day, req.interval)) ?? 0] += cost(r);
      through = Math.max(through, r.day);
    } else if (r.day <= prevTo) groupOf(r).prev += cost(r);
  }
  let series = [...groups.values()].map((g) => ({ tags: g.tags, values: g.values.map(round), total: round(g.values.reduce((a, b) => a + b, 0)), previousTotal: round(g.prev) }));
  series.sort((a, b) => Math.abs(b.total) - Math.abs(a.total) || Math.abs(b.previousTotal) - Math.abs(a.previousTotal));
  const total = round(series.reduce((a, s) => a + s.total, 0));
  const previousTotal = round(series.reduce((a, s) => a + s.previousTotal, 0));
  const limit = Math.min(req.limit ?? 200, 200);
  if (series.length > limit) {
    const rest = series.slice(limit);
    series = series.slice(0, limit);
    series.push({
      tags: { other: 'Other' },
      values: buckets.map((_, i) => round(rest.reduce((a, s) => a + s.values[i], 0))),
      total: round(rest.reduce((a, s) => a + s.total, 0)),
      previousTotal: round(rest.reduce((a, s) => a + s.previousTotal, 0)),
    });
  }
  return { buckets, series, total, previousTotal, previousRange: { from: prevFrom, to: prevTo + DAY - 1 }, dataThrough: through || today() - DAY };
}

function tagSource(key: string): string {
  if (FOCUS.includes(key)) return 'focus';
  if (key.startsWith('aws_')) return 'aws';
  if (key.startsWith('azure_') || key === 'metercategory' || key === 'metersubcategory') return 'azure';
  if (key.startsWith('gcp_')) return 'gcp';
  if (key.startsWith('kube_')) return 'kubernetes';
  return 'resource';
}

export async function costTags(req: CostTagsRequest): Promise<CostTag[]> {
  const from = dayOf(req.range.from);
  const to = dayOf(req.range.to);
  const match = matcher(req.query ?? '');
  const tags = new Map<string, { values: Map<string, number>; accounts: Set<string>; covered: number }>();
  let total = 0;
  for (const r of bills()) {
    if (r.day < from || r.day > to || !match(r)) continue;
    const c = req.costType === 'billed' ? r.billed : r.effective;
    total += c;
    for (const [k, v] of Object.entries(r.fields)) {
      if (!v) continue;
      let t = tags.get(k);
      if (!t) tags.set(k, (t = { values: new Map(), accounts: new Set(), covered: 0 }));
      t.values.set(v, (t.values.get(v) ?? 0) + c);
      t.accounts.add(r.fields.subaccountname);
      t.covered += c;
    }
  }
  return [...tags]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([key, t]) => ({
      key,
      source: tagSource(key),
      values: [...t.values]
        .sort((a, b) => b[1] - a[1])
        .slice(0, 50)
        .map(([value, cost]) => ({ value, cost: round(cost) })),
      distinctValues: t.values.size,
      coverage: total ? Math.min(1, Math.max(0, t.covered / total)) : 0,
      accounts: t.accounts.size,
    }));
}

// ---------------------------------------------------------------- budgets

const thisMonth = () => {
  const d = new Date();
  return d.getUTCFullYear() * 100 + d.getUTCMonth() + 1;
};
const monthMs = (m: number) => Date.UTC(Math.floor(m / 100), (m % 100) - 1, 1);
const monthsBetween = (a: number, b: number) => {
  const out: number[] = [];
  for (let m = a; m <= b && out.length < 120; m = m % 100 === 12 ? m + 89 : m + 1) out.push(m);
  return out;
};

function seedBudgets(): BudgetDoc[] {
  const year = Math.floor(thisMonth() / 100);
  const teams: [string, number][] = [
    ['platform', 95_000],
    ['ml', 60_000],
    ['data', 32_000],
  ];
  return [
    {
      type: 'budget',
      id: '3f0d7a52-5f7e-4b1e-9a43-1c2d3e4f5a6b',
      attributes: {
        name: `Jan - Dec ${year} All Providers Cost - Team Budget`,
        metrics_query: 'all.cost{*} by {team}',
        start_month: year * 100 + 1,
        end_month: year * 100 + 12,
        entries: teams.flatMap(([team, amount]) => monthsBetween(year * 100 + 1, year * 100 + 12).map((month) => ({ month, amount, tag_filters: [{ tag_key: 'team', tag_value: team }] }))),
        created_at: Date.UTC(year, 0, 3),
        updated_at: Date.UTC(year, 0, 3),
      },
    },
  ];
}

let budgets: BudgetDoc[] | null = null;
const store = () => (budgets ??= seedBudgets());

/** Each entry's cost so far and forecast for the month (this month: at the pace so far). */
async function withCosts(b: BudgetDoc): Promise<BudgetDoc> {
  const scope = budgetScope(b.attributes.metrics_query);
  const entries = b.attributes.entries ?? [];
  if (!scope) return b;
  const from = monthMs(b.attributes.start_month);
  const to = Math.min(monthMs(b.attributes.end_month) + 31 * DAY, Date.now());
  const res = from < to ? await queryCosts({ query: scope.query, costType: scope.costType, range: { from, to }, interval: 'month', groupBy: scope.by }) : null;
  const now = thisMonth();
  const start = monthMs(now);
  const elapsed = Math.max(1, (Date.now() - start) / DAY);
  const days = (Date.UTC(Math.floor(now / 100), now % 100, 1) - start) / DAY;
  const costed = entries.map((e) => {
    if (e.month > now) return { ...e, costs: { amount: e.amount, actual: null, forecast: null } };
    const series = res?.series.find((s) => scope.by.every((k) => s.tags[k] === (e.tag_filters.find((f) => f.tag_key === k)?.tag_value ?? '')));
    const i = res?.buckets.indexOf(monthMs(e.month)) ?? -1;
    const actual = round(series && i >= 0 ? series.values[i] : 0);
    return { ...e, costs: { amount: e.amount, actual, forecast: e.month === now ? round((actual / elapsed) * days) : actual } };
  });
  const sum = (f: (c: NonNullable<(typeof costed)[number]['costs']>) => number | null) => {
    const vals = costed.map((e) => f(e.costs!)).filter((v): v is number => v != null);
    return vals.length ? round(vals.reduce((a, v) => a + v, 0)) : null;
  };
  const total = entries.reduce((a, e) => a + e.amount, 0);
  return { ...b, attributes: { ...b.attributes, entries: costed, total_amount: total, costs: { amount: total, actual: sum((c) => c.actual), forecast: sum((c) => c.forecast) } } };
}

export async function listBudgets(): Promise<CostBudgetSummary[]> {
  const now = thisMonth();
  const out: CostBudgetSummary[] = [];
  for (const b of store()) {
    const full = await withCosts(b);
    const month = (full.attributes.entries ?? []).filter((e) => e.month === now);
    const covers = now >= b.attributes.start_month && now <= b.attributes.end_month;
    out.push({
      id: b.id!,
      name: b.attributes.name,
      metricsQuery: b.attributes.metrics_query,
      startMonth: b.attributes.start_month,
      endMonth: b.attributes.end_month,
      totalAmount: full.attributes.total_amount ?? 0,
      month: covers ? now : null,
      monthAmount: month.reduce((a, e) => a + e.amount, 0),
      monthActual: round(month.reduce((a, e) => a + (e.costs?.actual ?? 0), 0)),
      monthForecast: round(month.reduce((a, e) => a + (e.costs?.forecast ?? 0), 0)),
    });
  }
  return out.sort((a, b) => a.name.toLowerCase().localeCompare(b.name.toLowerCase()));
}

export async function getBudget(id: string): Promise<BudgetDoc> {
  const b = store().find((x) => x.id === id);
  if (!b) throw new Error('budget not found');
  return withCosts(b);
}

export async function saveBudget(doc: BudgetDoc): Promise<BudgetDoc> {
  const a = doc.attributes;
  const errors: string[] = [];
  if (!a.name?.trim()) errors.push('`name` is required');
  const scope = budgetScope(a.metrics_query ?? '');
  if (!scope) errors.push('invalid `metrics_query`');
  if (a.start_month > a.end_month) errors.push('`start_month` must not be after `end_month`');
  const months = monthsBetween(a.start_month, a.end_month);
  for (const [i, e] of (a.entries ?? []).entries()) {
    if (!months.includes(e.month)) errors.push(`entry ${i}: month ${e.month} is outside the budget's months`);
    const keys = e.tag_filters.map((f) => f.tag_key).sort();
    if (scope && JSON.stringify(keys) !== JSON.stringify([...scope.by].sort())) errors.push(`entry ${i}: the tag filters must name each tag of the query's \`by {…}\` once (${scope.by.join(', ')})`);
  }
  if (errors.length) throw new Error(errors.join('; '));
  const now = Date.now();
  const saved: BudgetDoc = {
    type: 'budget',
    id: doc.id ?? crypto.randomUUID(),
    attributes: { ...a, entries: (a.entries ?? []).map(({ month, amount, tag_filters }) => ({ month, amount, tag_filters })), created_at: doc.id ? a.created_at : now, updated_at: now },
  };
  const list = store();
  const i = list.findIndex((b) => b.id === saved.id);
  if (doc.id && i < 0) throw new Error('failed to upsert budget: budget not found');
  if (i >= 0) list[i] = saved;
  else list.push(saved);
  return saved;
}

export async function deleteBudget(id: string): Promise<void> {
  budgets = store().filter((b) => b.id !== id);
}

// ---------------------------------------------------------------- unit costs and commitments

export async function unitCosts(req: UnitCostsRequest): Promise<UnitCostRow[]> {
  const from = dayOf(req.range.from);
  const to = dayOf(req.range.to);
  const match = matcher(req.query ?? '');
  const groups = new Map<string, UnitCostRow>();
  for (const r of bills()) {
    if (r.day < from || r.day > to || r.category !== 'usage' || r.quantity <= 0 || r.fields.commitmentdiscountstatus === 'Unused' || !match(r)) continue;
    const tags = Object.fromEntries(req.groupBy.map((k) => [k, field(r, k) || 'N/A']));
    const key = JSON.stringify(tags);
    const g = groups.get(key) ?? { tags, usageQuantity: 0, amortizedCost: 0, onDemandCost: 0 };
    g.usageQuantity += r.quantity;
    g.amortizedCost += r.effective;
    g.onDemandCost += r.list;
    groups.set(key, g);
  }
  return [...groups.values()]
    .map((g) => ({ ...g, amortizedCost: round(g.amortizedCost), onDemandCost: round(g.onDemandCost) }))
    .sort((a, b) => b.amortizedCost - a.amortizedCost)
    .slice(0, 500);
}

export async function commitments(req: CommitmentsRequest): Promise<CommitmentsResult> {
  const from = dayOf(req.range.from);
  const to = dayOf(req.range.to);
  const match = matcher(req.query ?? '');
  const rows = bills().filter((r) => r.day >= from && r.day <= to && r.category === 'usage' && r.fields.providername.toLowerCase() === req.provider.toLowerCase() && match(r));
  const buckets: number[] = [];
  for (let d = from; d <= to; d += DAY) buckets.push(d);
  const n = buckets.length;
  const at = (r: Row) => (r.day - from) / DAY;
  const committed = (r: Row) => r.fields.pricingcategory === 'Committed' || !!r.fields.commitmentdiscountid;
  const eligible = new Set(rows.filter(committed).map((r) => r.fields.servicename));
  const zeros = () => new Array<number>(n).fill(0);
  const [cost, equivalent, eligibleCost, eligibleList, onDemand] = [zeros(), zeros(), zeros(), zeros(), zeros()];
  const byType = new Map<string, number[]>();
  const byService = new Map<string, { service: string; region: string; covered: number; onDemand: number }>();
  const util = new Map<string, { used: number; unused: number }>();
  const per = new Map<string, { used: number; unused: number; list: number; r: Row }>();
  for (const r of rows) {
    if (!eligible.has(r.fields.servicename)) continue;
    const i = at(r);
    eligibleCost[i] += r.effective;
    const svcKey = `${r.fields.servicename}|${r.fields.regionname}`;
    const svc = byService.get(svcKey) ?? { service: r.fields.servicename, region: r.fields.regionname, covered: 0, onDemand: 0 };
    byService.set(svcKey, svc);
    if (committed(r)) {
      cost[i] += r.effective;
      const kind = r.fields.commitmentdiscounttype || 'Commitment';
      const u = util.get(kind) ?? { used: 0, unused: 0 };
      util.set(kind, u);
      const c = per.get(r.fields.commitmentdiscountid) ?? { used: 0, unused: 0, list: 0, r };
      per.set(r.fields.commitmentdiscountid, c);
      if (r.fields.commitmentdiscountstatus === 'Unused') {
        u.unused += r.effective;
        c.unused += r.effective;
      } else {
        u.used += r.effective;
        c.used += r.effective;
        c.list += r.list;
        equivalent[i] += r.list;
        eligibleList[i] += r.list;
        if (!byType.has(kind)) byType.set(kind, zeros());
        byType.get(kind)![i] += r.list;
        svc.covered += r.list;
      }
    } else {
      onDemand[i] += r.effective;
      eligibleList[i] += r.list;
      svc.onDemand += r.effective;
    }
  }
  const rate = (c: number, l: number) => (l > 0 ? Math.round((1 - c / l) * 10_000) / 10_000 : null);
  const sum = (a: number[]) => a.reduce((x, y) => x + y, 0);
  const covered = sum(equivalent);
  const odSpend = sum(onDemand);
  const list = [...per]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([id, c]) => ({
      id,
      kind: c.r.fields.commitmentdiscounttype,
      account: c.r.fields.subaccountname,
      region: c.r.fields.regionname,
      service: c.r.fields.servicename,
      term: c.r.fields.commitment_term ?? '',
      purchaseOption: c.r.fields.commitment_purchase_option ?? '',
      start: c.r.fields.commitment_start ?? '',
      end: c.r.fields.commitment_end ?? '',
      amortizedCost: round(c.used + c.unused),
      unusedCost: round(c.unused),
      utilization: c.used + c.unused > 0 ? c.used / (c.used + c.unused) : 0,
      savings: round(c.list - c.used - c.unused),
    }));
  return {
    buckets,
    commitmentCost: cost.map(round),
    onDemandEquivalent: equivalent.map(round),
    savingsRate: buckets.map((_, i) => rate(eligibleCost[i], eligibleList[i])),
    effectiveSavingsRate: rate(sum(eligibleCost), sum(eligibleList)),
    realizedSavings: round(list.reduce((a, c) => a + c.savings, 0)),
    coverage: covered + odSpend > 0 ? covered / (covered + odSpend) : 0,
    onDemandSpend: round(odSpend),
    coverageByType: [...[...byType].sort(([a], [b]) => a.localeCompare(b)).map(([kind, values]) => ({ kind, values: values.map(round) })), { kind: 'On-Demand', values: onDemand.map(round) }],
    coverageByService: [...byService.values()].map((x) => ({ ...x, covered: round(x.covered), onDemand: round(x.onDemand) })).sort((a, b) => b.covered + b.onDemand - (a.covered + a.onDemand)),
    utilization: [...util].sort(([a], [b]) => a.localeCompare(b)).map(([kind, u]) => ({ kind, used: round(u.used), unused: round(u.unused) })),
    commitments: list,
  };
}
