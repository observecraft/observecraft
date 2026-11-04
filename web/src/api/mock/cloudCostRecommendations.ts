// Mock Cloud Cost recommendations: fictional AWS and Azure resources the server's rules
// (routes/cloud_cost_recommendations.rs) would flag, filtered and totalled as the server does.
import type { CostRecommendation, RecommendationFilters, RecommendationGroup, RecommendationsResult } from '../cloudCost/client';
import { globMatch } from '../../lib/metricRules';

const LOOKBACK_DAYS = 14;
const AWS_ACCOUNT = '123456789012';
const SUBSCRIPTION = '00000000-0000-0000-0000-000000000001';

const RESOURCE_TYPES: Record<string, { label: string; provider: string; price: number; basis: string; action: string }> = {
  aws_ec2_instance: {
    label: 'EC2 Instance',
    provider: 'aws',
    price: 70.08,
    basis: "an m5.large on demand in us-east-1 ($0.096 an hour): instance types aren't collected",
    action: "Make sure nothing depends on it (a standby, a scheduled job's host), snapshot the EBS volumes you want to keep, then stop or terminate it (EC2 → Instances → Instance state).",
  },
  aws_rds_instance: {
    label: 'RDS Instance',
    provider: 'aws',
    price: 49.64,
    basis: 'a db.t3.medium Single-AZ on demand in us-east-1 ($0.068 an hour), storage not included',
    action: 'Take a final snapshot, then delete the instance. Stopping it only pauses the instance charge for 7 days, after which RDS starts it again.',
  },
  aws_ebs_volume: {
    label: 'EBS Volume',
    provider: 'aws',
    price: 8,
    basis: "100 GiB of gp3 in us-east-1 ($0.08 per GiB-month): volume sizes aren't collected",
    action: 'Snapshot the volume if its data matters, detach it from its instance, then delete it.',
  },
  aws_nat_gateway: {
    label: 'AWS NAT Gateway',
    provider: 'aws',
    price: 32.85,
    basis: '$0.045 an hour in us-east-1, data processing not included',
    action: 'Check that no route table still sends traffic to it, then delete it and release its Elastic IP address.',
  },
  aws_alb: {
    label: 'Application Load Balancer',
    provider: 'aws',
    price: 16.43,
    basis: '$0.0225 an hour in us-east-1, LCUs not included',
    action: 'Check that no DNS record or client still points at its DNS name, then delete it and the target groups only it used.',
  },
  azure_vm: {
    label: 'Azure Virtual Machine',
    provider: 'azure',
    price: 70.08,
    basis: "a D2s v5 pay-as-you-go Linux VM in East US ($0.096 an hour): VM sizes aren't collected",
    action: 'Make sure nothing depends on it, then stop (deallocate) it or delete it with its disks. A VM stopped from inside the OS is still allocated and billed.',
  },
  azure_disk: {
    label: 'Azure Managed Disk',
    provider: 'azure',
    price: 19.71,
    basis: "a P10 Premium SSD (128 GiB) in East US: disk sizes aren't collected",
    action: 'Snapshot the disk if its data matters, then delete it; a disk you must keep can move to Standard HDD.',
  },
  azure_public_ip: {
    label: 'Azure Public IP Address',
    provider: 'azure',
    price: 3.65,
    basis: 'a Standard static IPv4 address ($0.005 an hour)',
    action: 'Check that no DNS record or allowlist still names the address, dissociate it, then delete it.',
  },
};

const RECOMMENDATION_TYPES: Record<string, string> = {
  idle_compute: 'Terminate idle instances',
  rightsize_compute: 'Downsize over-provisioned instances',
  unused_database: 'Delete unused databases',
  idle_volume: 'Delete idle volumes',
  unused_nat_gateway: 'Delete unused NAT gateways',
  idle_load_balancer: 'Delete idle load balancers',
  unused_public_ip: 'Release unused public IPs',
};

const RIGHTSIZE_ACTIONS: Record<string, string> = {
  aws: 'Change it to the next smaller size of its family (stop it, then Actions → Instance settings → Change instance type, or update its launch template) and watch its CPU for a week.',
  azure: 'Resize it to the next smaller size of its series (the VM’s Size page; a running VM restarts) and watch its CPU for a week.',
};

type Fixture = [resourceType: string, recommendationType: string, id: string, name: string, region: string, reason: string, evidence: [string, string][], billed: number | null, tags: string[]];

const arm = (group: string, type: string, name: string) => `/subscriptions/${SUBSCRIPTION}/resourceGroups/${group}/providers/${type}/${name}`;

const FIXTURES: Fixture[] = [
  [
    'aws_ec2_instance',
    'idle_compute',
    'i-0a1b2c3d4e5f60001',
    'storefront-api-canary',
    'us-east-1',
    'Max CPU 1.8% and 2.1 MB of network traffic a day over 14 days',
    [
      ['Max CPU', '1.8%'],
      ['Average CPU', '0.6%'],
      ['Network traffic a day', '2.1 MB'],
    ],
    280.32,
    ['env:staging', 'team:storefront', 'service:storefront-api'],
  ],
  [
    'aws_ec2_instance',
    'idle_compute',
    'i-0a1b2c3d4e5f60002',
    'catalog-batch-old',
    'us-east-1',
    'Max CPU 0.9% and 640 KB of network traffic a day over 14 days',
    [
      ['Max CPU', '0.9%'],
      ['Average CPU', '0.3%'],
      ['Network traffic a day', '640 KB'],
    ],
    140.16,
    ['env:prod', 'team:catalog', 'service:catalog-service'],
  ],
  [
    'aws_ec2_instance',
    'rightsize_compute',
    'i-0a1b2c3d4e5f60003',
    'order-service-worker-1',
    'us-east-1',
    'Average CPU 4.2% and max CPU 22% over 14 days',
    [
      ['Average CPU', '4.2%'],
      ['Max CPU', '22%'],
      ['Network traffic a day', '3.4 GB'],
    ],
    560.64,
    ['env:prod', 'team:checkout', 'service:order-service'],
  ],
  [
    'aws_ec2_instance',
    'rightsize_compute',
    'i-0a1b2c3d4e5f60004',
    'inventory-service-2',
    'eu-west-1',
    'Average CPU 6.8% and max CPU 31% over 14 days',
    [
      ['Average CPU', '6.8%'],
      ['Max CPU', '31%'],
      ['Network traffic a day', '1.2 GB'],
    ],
    280.32,
    ['env:prod', 'team:inventory', 'service:inventory-service'],
  ],
  [
    'aws_ec2_instance',
    'rightsize_compute',
    'i-0a1b2c3d4e5f60005',
    'api-gateway-3',
    'us-east-1',
    'Average CPU 8.1% and max CPU 36% over 14 days',
    [
      ['Average CPU', '8.1%'],
      ['Max CPU', '36%'],
      ['Network traffic a day', '18.4 GB'],
    ],
    null,
    ['env:prod', 'team:platform', 'service:api-gateway'],
  ],
  [
    'aws_rds_instance',
    'unused_database',
    'billing-reports-replica',
    'billing-reports-replica',
    'us-east-1',
    'No connections over 14 days',
    [
      ['Max connections', '0'],
      ['Max CPU', '3.1%'],
    ],
    412.8,
    ['env:prod', 'team:billing', 'service:billing'],
  ],
  [
    'aws_rds_instance',
    'unused_database',
    'catalog-db-staging',
    'catalog-db-staging',
    'eu-west-1',
    'No connections over 12 days',
    [
      ['Max connections', '0'],
      ['Max CPU', '2.4%'],
    ],
    null,
    ['env:staging', 'team:catalog'],
  ],
  [
    'aws_ebs_volume',
    'idle_volume',
    'vol-0a1b2c3d4e5f60001',
    'vol-0a1b2c3d4e5f60001',
    'us-east-1',
    '0 read and write operations a day over 14 days',
    [['Read and write operations a day', '0']],
    48,
    ['env:prod', 'team:storefront'],
  ],
  [
    'aws_ebs_volume',
    'idle_volume',
    'vol-0a1b2c3d4e5f60002',
    'vol-0a1b2c3d4e5f60002',
    'us-east-1',
    '0.2 read and write operations a day over 14 days',
    [['Read and write operations a day', '0.2']],
    null,
    ['env:staging'],
  ],
  [
    'aws_nat_gateway',
    'unused_nat_gateway',
    'nat-0a1b2c3d4e5f60001',
    'nat-0a1b2c3d4e5f60001',
    'eu-west-1',
    '120 KB of traffic a day over 14 days',
    [['Traffic a day', '120 KB']],
    null,
    ['env:staging', 'team:platform'],
  ],
  [
    'aws_alb',
    'idle_load_balancer',
    'app/storefront-legacy/50dc6c495c0c9188',
    'app/storefront-legacy/50dc6c495c0c9188',
    'us-east-1',
    '3 requests a day over 14 days',
    [['Requests a day', '3']],
    21.3,
    ['env:prod', 'team:storefront'],
  ],
  [
    'azure_vm',
    'idle_compute',
    arm('shop-staging', 'Microsoft.Compute/virtualMachines', 'catalog-vm-2'),
    'catalog-vm-2',
    'eastus',
    'Max CPU 2.6% and 1.4 MB of network traffic a day over 14 days',
    [
      ['Max CPU', '2.6%'],
      ['Average CPU', '1.1%'],
      ['Network traffic a day', '1.4 MB'],
    ],
    null,
    ['env:staging', 'team:catalog'],
  ],
  [
    'azure_vm',
    'rightsize_compute',
    arm('shop-prod', 'Microsoft.Compute/virtualMachines', 'billing-vm-1'),
    'billing-vm-1',
    'eastus',
    'Average CPU 5.3% and max CPU 28% over 14 days',
    [
      ['Average CPU', '5.3%'],
      ['Max CPU', '28%'],
      ['Network traffic a day', '840 MB'],
    ],
    286.4,
    ['env:prod', 'team:billing', 'service:billing'],
  ],
  [
    'azure_disk',
    'idle_volume',
    arm('shop-staging', 'Microsoft.Compute/disks', 'catalog-vm-2-data'),
    'catalog-vm-2-data',
    'eastus',
    'At most 0 read or write operations a second over 14 days',
    [['Max operations a second', '0']],
    null,
    ['env:staging', 'team:catalog'],
  ],
  [
    'azure_public_ip',
    'unused_public_ip',
    arm('shop-prod', 'Microsoft.Network/publicIPAddresses', 'storefront-old-ip'),
    'storefront-old-ip',
    'westeurope',
    '0 B of traffic a day over 14 days',
    [['Traffic a day', '0 B']],
    null,
    ['env:prod', 'team:storefront'],
  ],
];

const round2 = (v: number) => Math.round(v * 100) / 100;

function build([resourceType, recommendationType, id, name, region, reason, evidence, billed, tags]: Fixture): CostRecommendation {
  const t = RESOURCE_TYPES[resourceType];
  const monthlyCost = billed ?? t.price;
  const share = recommendationType === 'rightsize_compute' ? 0.5 : 1;
  return {
    id: `${resourceType}:${id}`,
    provider: t.provider,
    account: t.provider === 'aws' ? AWS_ACCOUNT : SUBSCRIPTION,
    region,
    resourceId: id,
    resourceName: name,
    resourceType,
    resourceTypeLabel: t.label,
    recommendationType,
    recommendationLabel: RECOMMENDATION_TYPES[recommendationType],
    reason,
    evidence: evidence.map(([label, value]) => ({ label, value })),
    action: recommendationType === 'rightsize_compute' ? RIGHTSIZE_ACTIONS[t.provider] : t.action,
    monthlyCost,
    costSource: billed == null ? 'list_price' : 'bill',
    costBasis: billed == null ? t.basis : '',
    monthlySavings: round2(monthlyCost * share),
    observedDays: Number(/over (\d+) days/.exec(reason)?.[1] ?? LOOKBACK_DAYS),
    tags,
  };
}

function groups(recs: CostRecommendation[], key: (r: CostRecommendation) => [string, string]): RecommendationGroup[] {
  const by = new Map<string, RecommendationGroup>();
  for (const r of recs) {
    const [id, label] = key(r);
    const g = by.get(id) ?? { id, label, count: 0, monthlyCost: 0, monthlySavings: 0 };
    g.count += 1;
    g.monthlyCost += r.monthlyCost;
    g.monthlySavings += r.monthlySavings;
    by.set(id, g);
  }
  return [...by.values()]
    .map((g) => ({ ...g, monthlyCost: round2(g.monthlyCost), monthlySavings: round2(g.monthlySavings) }))
    .sort((a, b) => b.monthlySavings - a.monthlySavings || a.id.localeCompare(b.id));
}

const matches = (f: RecommendationFilters, r: CostRecommendation) =>
  (!f.provider || f.provider === r.provider) &&
  (!f.resourceType || f.resourceType === r.resourceType) &&
  (!f.recommendationType || f.recommendationType === r.recommendationType) &&
  (f.tags ?? []).every((t) => [...r.tags, `account:${r.account}`, `region:${r.region}`].some((x) => globMatch(t, x)));

export async function recommendations(f: RecommendationFilters): Promise<RecommendationsResult> {
  const all = FIXTURES.map(build);
  const recs = all.filter((r) => matches(f, r)).sort((a, b) => b.monthlySavings - a.monthlySavings || a.id.localeCompare(b.id));
  const monthlySavings = recs.reduce((s, r) => s + r.monthlySavings, 0);
  return {
    recommendations: recs,
    count: recs.length,
    monthlyCost: round2(recs.reduce((s, r) => s + r.monthlyCost, 0)),
    monthlySavings: round2(monthlySavings),
    annualSavings: round2(monthlySavings * 12),
    byType: groups(recs, (r) => [r.recommendationType, r.recommendationLabel]),
    resourceTypes: groups(all, (r) => [r.resourceType, r.resourceTypeLabel]),
    recommendationTypes: groups(all, (r) => [r.recommendationType, r.recommendationLabel]),
    accounts: { aws: 1, azure: 1 },
    lookbackDays: LOOKBACK_DAYS,
  };
}
