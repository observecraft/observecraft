// The cost dashboards of Summarize → GPU, Networking, Storage and SaaS Costs: each tab lists its boards
// in a sidebar. A board is a scope over the costs, template variables and widgets over that scope.

export type Widget =
  | { kind: 'kpi'; title: string; value: 'range' | 'recent' | 'prior' | 'change'; scope?: string; spark?: boolean; w?: number }
  | { kind: 'bars'; title: string; groupBy?: string; scope?: string; interval?: 'day' | 'week' | 'month'; fixedRange?: string; w?: number }
  | { kind: 'donut'; title: string; groupBy: string; scope?: string; w?: number }
  | { kind: 'table'; title: string; groupBy: string; scope?: string; changes?: boolean; w?: number }
  | { kind: 'note'; lines: string[]; title?: string; w?: number };

export interface BoardSection {
  title: string;
  tone?: 'green' | 'purple';
  widgets: Widget[];
}

export interface Board {
  id: string;
  label: string;
  /** Provider (or platform) whose logo the sidebar shows. */
  provider?: string;
  title: string;
  scope: string;
  vars: string[];
  header: Widget[];
  sections: BoardSection[];
}

const kpis = (what: string, scope?: string): Widget[] => [
  { kind: 'kpi', title: `Total ${what}`, value: 'range', scope, w: 3 },
  { kind: 'kpi', title: `Total ${what} (past 30d)`, value: 'recent', scope, w: 3 },
  { kind: 'kpi', title: `Total ${what} (30d prior)`, value: 'prior', scope, w: 3 },
  { kind: 'kpi', title: 'Month-over-month Change', value: 'change', scope, w: 3 },
];

// ---------------------------------------------------------------- GPU

const gpu = (id: string, label: string, provider: string | undefined, groupBy: string): Board => ({
  id,
  label,
  provider,
  title: provider ? `${label} GPU Cost Overview` : 'GPU Cost Overview',
  scope: `gpu_type:*${provider ? ` providername:${provider}` : ''}`,
  vars: ['team'],
  header: kpis('GPU Spend'),
  sections: [
    {
      title: 'GPU Spend',
      widgets: [
        { kind: 'bars', title: provider ? 'Total GPU Spend by GPU Type' : 'Total GPU Spend by Cloud Provider', groupBy: provider ? 'gpu_type' : 'providername', w: 12 },
        { kind: 'bars', title: 'GPU Spend by Team (Monthly)', groupBy: 'team', interval: 'month', fixedRange: '1y', w: 12 },
      ],
    },
    {
      title: 'Cost & Efficiency',
      widgets: [
        { kind: 'donut', title: 'GPU Spend by GPU Type', groupBy: 'gpu_type', w: 5 },
        { kind: 'table', title: 'GPU Spend Changes', groupBy, changes: true, w: 7 },
      ],
    },
  ],
});

export const GPU_BOARDS: Board[] = [
  gpu('overview', 'Overview', undefined, 'servicename'),
  gpu('aws', 'AWS', 'aws', 'aws_instance_family'),
  gpu('azure', 'Azure', 'azure', 'subaccountname'),
  gpu('gcp', 'GCP', 'gcp', 'servicename'),
];

// ---------------------------------------------------------------- networking

const TRANSFER_TYPES = [
  'Data transfer types:',
  '• cross-region: between two regions of the provider.',
  '• cross-zone: between availability zones of one region.',
  '• within-zone: inside one availability zone.',
  '• internet: out of the provider to the public internet.',
];

export const NETWORKING_BOARDS: Board[] = [
  {
    id: 'aws',
    label: 'AWS',
    provider: 'aws',
    title: 'AWS Networking Costs Overview',
    scope: 'providername:aws aws_product:(datatransfer OR natgateway OR elb OR cloudfront)',
    vars: ['aws_product', 'regionname', 'service', 'team'],
    header: [
      { kind: 'kpi', title: 'Total Networking Cost', value: 'range', spark: true, w: 3 },
      { kind: 'kpi', title: 'Total Data Transfer Cost', value: 'range', scope: 'aws_product:datatransfer', w: 3 },
      { kind: 'kpi', title: 'Total Elastic Load Balancer Cost', value: 'range', scope: 'aws_product:elb', w: 3 },
      { kind: 'kpi', title: 'Total NAT Gateway Cost', value: 'range', scope: 'aws_product:natgateway', w: 3 },
      { kind: 'bars', title: 'Total Networking Cost', groupBy: 'aws_product', w: 12 },
    ],
    sections: [
      {
        title: 'Data Transfer Overview',
        widgets: [
          { kind: 'note', lines: TRANSFER_TYPES, w: 3 },
          { kind: 'donut', title: 'Costs by Data Transfer Type', groupBy: 'data_transfer_type', scope: 'aws_product:datatransfer', w: 4 },
          { kind: 'bars', title: 'Costs by Data Transfer Type', groupBy: 'data_transfer_type', scope: 'aws_product:datatransfer', w: 5 },
        ],
      },
      {
        title: 'Networking Cost Changes',
        widgets: [{ kind: 'table', title: 'Networking cost by service', groupBy: 'service', changes: true, w: 12 }],
      },
    ],
  },
  {
    id: 'azure',
    label: 'Azure',
    provider: 'azure',
    title: 'Azure Networking Costs Overview',
    scope: 'providername:azure metercategory:Bandwidth',
    vars: ['regionname', 'team'],
    header: [
      { kind: 'kpi', title: 'Total Networking Cost', value: 'range', spark: true, w: 4 },
      { kind: 'bars', title: 'Total Networking Cost', groupBy: 'data_transfer_type', w: 8 },
    ],
    sections: [
      {
        title: 'Data Transfer Overview',
        widgets: [
          { kind: 'note', lines: TRANSFER_TYPES, w: 4 },
          { kind: 'table', title: 'Costs by Data Transfer Type', groupBy: 'data_transfer_type', changes: true, w: 8 },
        ],
      },
    ],
  },
  {
    id: 'gcp',
    label: 'GCP',
    provider: 'gcp',
    title: 'Google Cloud Networking Costs Overview',
    scope: 'providername:gcp gcp_product:networking',
    vars: ['regionname', 'team'],
    header: [
      { kind: 'kpi', title: 'Total Networking Cost', value: 'range', spark: true, w: 4 },
      { kind: 'bars', title: 'Total Networking Cost', groupBy: 'data_transfer_type', w: 8 },
    ],
    sections: [
      {
        title: 'Data Transfer Overview',
        widgets: [
          { kind: 'note', lines: TRANSFER_TYPES, w: 4 },
          { kind: 'table', title: 'Costs by Data Transfer Type', groupBy: 'data_transfer_type', changes: true, w: 8 },
        ],
      },
    ],
  },
];

// ---------------------------------------------------------------- storage

const storage = (id: string, label: string, provider: string, title: string, scope: string, typeTag: string, vars: string[]): Board => ({
  id,
  label,
  provider,
  title,
  scope: `providername:${provider} ${scope}`,
  vars,
  header: [
    { kind: 'kpi', title: 'Total Cost', value: 'range', spark: true, w: 3 },
    { kind: 'kpi', title: 'Cost past month', value: 'recent', w: 2 },
    { kind: 'kpi', title: 'Cost previous month', value: 'prior', w: 2 },
    { kind: 'bars', title: 'Daily cost', w: 5 },
  ],
  sections: [
    {
      title: 'Cost by Storage Types',
      widgets: [
        { kind: 'donut', title: 'Cost by storage type', groupBy: typeTag, w: 5 },
        { kind: 'table', title: 'Storage type cost', groupBy: typeTag, w: 7 },
      ],
    },
    {
      title: 'Cost by Sub Account',
      widgets: [
        { kind: 'bars', title: 'Cost by sub account', groupBy: 'subaccountname', w: 7 },
        { kind: 'table', title: 'Cost by team', groupBy: 'team', changes: true, w: 5 },
      ],
    },
  ],
});

export const STORAGE_BOARDS: Board[] = [
  storage('s3', 'S3', 'aws', 'S3 Cost Overview', 'aws_product:s3', 'storage_class', ['subaccountname', 'regionname', 'team', 'service']),
  storage('azure', 'Azure Storage', 'azure', 'Azure Storage Cost Overview', 'metercategory:Storage', 'metersubcategory', ['subaccountname', 'team', 'env']),
  storage('gcs', 'Cloud Storage', 'gcp', 'Cloud Storage Cost Overview', 'gcp_product:storage', 'storage_class', ['subaccountname', 'team']),
];

// ---------------------------------------------------------------- SaaS

/** A SaaS provider's board (providers other than the clouds, as their bills name them). */
export function saasBoard(provider: string): Board {
  return {
    id: provider,
    label: provider,
    provider,
    title: `${provider} Cost Overview and Changes`,
    scope: `providername:"${provider}"`,
    vars: ['servicename', 'team', 'service'],
    header: kpis(`${provider} Spend`).map((w) =>
      w.kind === 'kpi' && w.value === 'range'
        ? { ...w, title: `Total ${provider} Spend` }
        : w.kind === 'kpi' && w.value !== 'change'
          ? { ...w, title: w.title.replace(`${provider} Spend`, 'Spend') }
          : w,
    ),
    sections: [
      {
        title: 'Cost Overview',
        tone: 'purple',
        widgets: [
          { kind: 'note', lines: ['Tip: click a row of a table to filter the whole dashboard to that product.', 'Filtering the dashboard helps isolate what drives cost changes.'], w: 3 },
          { kind: 'bars', title: 'Cost by product', groupBy: 'servicename', w: 9 },
          { kind: 'donut', title: 'Cost by product', groupBy: 'servicename', w: 5 },
          { kind: 'table', title: `${provider} products with largest cost changes`, groupBy: 'servicename', changes: true, w: 7 },
        ],
      },
      {
        title: 'Cost Allocation by Your Tags',
        tone: 'purple',
        widgets: [
          { kind: 'bars', title: 'Cost by team', groupBy: 'team', w: 7 },
          { kind: 'table', title: 'Cost by service', groupBy: 'service', changes: true, w: 5 },
        ],
      },
    ],
  };
}

// ---------------------------------------------------------------- containers

export const CONTAINER_BOARDS: Board[] = [
  {
    id: 'kubernetes',
    label: 'Kubernetes',
    provider: 'kubernetes',
    title: 'Kubernetes Cost Overview',
    scope: 'kube_cluster_name:*',
    vars: ['kube_cluster_name', 'kube_namespace', 'team'],
    header: kpis('Container Spend'),
    sections: [
      {
        title: 'Clusters',
        widgets: [
          { kind: 'bars', title: 'Cost by cluster', groupBy: 'kube_cluster_name', w: 7 },
          { kind: 'table', title: 'Cluster cost changes', groupBy: 'kube_cluster_name', changes: true, w: 5 },
        ],
      },
      {
        title: 'Workloads',
        widgets: [
          { kind: 'donut', title: 'Cost by namespace', groupBy: 'kube_namespace', w: 5 },
          { kind: 'table', title: 'Cost by deployment', groupBy: 'kube_deployment', changes: true, w: 7 },
        ],
      },
    ],
  },
];
