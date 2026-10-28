import {
  Activity,
  Bot,
  Boxes,
  Bug,
  Cloud,
  Code2,
  Database,
  Gauge,
  GitBranch,
  LayoutDashboard,
  Megaphone,
  Monitor,
  Network,
  Puzzle,
  ScrollText,
  ShieldCheck,
  Siren,
  Sparkles,
  Zap,
  type LucideIcon,
} from 'lucide-react';

export interface MenuLink {
  label: string;
  to: string;
  isNew?: boolean;
  isPreview?: boolean;
  external?: boolean;
}

/** A titled block of the hover menu; each column is a vertical list of links. */
export interface MenuGroup {
  title: string;
  /** Makes the title itself a link (groups without sub-items). */
  to?: string;
  badge?: 'NEW' | 'PREVIEW';
  columns: MenuLink[][];
}

export interface NavItem {
  label: string;
  to: string;
  icon: LucideIcon;
  menu?: MenuGroup[];
}

/** Primary navigation. Links without a built page land on the "not designed yet" placeholder. */
export const NAV_SECTIONS: NavItem[][] = [
  [
    {
      label: 'Assist AI',
      to: '/assist',
      icon: Sparkles,
      menu: [
        {
          title: 'Assist AI',
          columns: [
            [
              { label: 'Chat', to: '/assist' },
              { label: 'Investigations', to: '/assist/investigations' },
            ],
          ],
        },
      ],
    },
    {
      label: 'Dashboards',
      to: '/dashboards',
      icon: LayoutDashboard,
      menu: [
        {
          title: 'Dashboards',
          columns: [
            [
              { label: 'Dashboard List', to: '/dashboards' },
              { label: 'Shared Dashboards', to: '/dashboards/shared' },
            ],
            [{ label: 'New Dashboard', to: '/dashboards?new=1' }],
          ],
        },
        {
          title: 'Notebooks',
          columns: [
            [
              { label: 'Notebook List', to: '/notebooks' },
              { label: 'Templates', to: '/notebooks/templates' },
            ],
            [{ label: 'New Notebook', to: '/notebooks?new=1' }],
          ],
        },
      ],
    },
    {
      label: 'Monitoring',
      to: '/monitors',
      icon: Activity,
      menu: [
        {
          title: 'Monitors',
          columns: [
            [
              { label: 'Monitor List', to: '/monitors' },
              { label: 'Triggered', to: '/monitors/triggered' },
              { label: 'Downtimes', to: '/monitors/downtimes' },
              { label: 'Quality', to: '/monitors/quality' },
            ],
            [
              { label: 'New Monitor', to: '/monitors/create' },
              { label: 'Monitor Templates', to: '/monitors/templates' },
              { label: 'Service Level Objectives', to: '/slo' },
              { label: 'Synthetic Tests', to: '/synthetics' },
            ],
          ],
        },
        {
          title: 'Event Management',
          columns: [
            [
              { label: 'All Events', to: '/event/explorer' },
              { label: 'Triage Inbox', to: '/event/inbox' },
            ],
          ],
        },
      ],
    },
    {
      label: 'Developer Portal',
      to: '/portal',
      icon: Code2,
      menu: [
        {
          title: 'Developer Portal',
          columns: [
            [
              { label: 'Software Catalog', to: '/apm' },
              { label: 'Scorecards', to: '/portal/scorecards' },
            ],
            [{ label: 'Self-Service Actions', to: '/portal/actions' }],
          ],
        },
      ],
    },
    {
      label: 'Incident Response',
      to: '/incidents',
      icon: Siren,
      menu: [
        {
          title: 'Incidents',
          columns: [
            [
              { label: 'Incident List', to: '/incidents' },
              { label: 'Declare Incident', to: '/incidents?declare=1' },
            ],
            [
              { label: 'On-Call', to: '/on-call' },
              { label: 'Status Pages', to: '/status-pages', isNew: true },
            ],
          ],
        },
      ],
    },
    {
      label: 'Automation',
      to: '/automation',
      icon: Zap,
      menu: [
        {
          title: 'Automation',
          columns: [
            [
              { label: 'Workflows', to: '/automation' },
              { label: 'App Builder', to: '/automation/apps' },
            ],
          ],
        },
      ],
    },
  ],
  [
    {
      label: 'Infrastructure',
      to: '/infrastructure',
      icon: Boxes,
      // Datadog's Infrastructure menu.
      menu: [
        {
          title: 'Resources',
          columns: [
            [
              { label: 'Catalog', to: '/infrastructure/catalog' },
              { label: 'Cloudcraft', to: '/cloudcraft' },
              { label: 'Changes', to: '/infrastructure/changes', isPreview: true },
            ],
            [
              { label: 'Policies', to: '/infrastructure/policies', isPreview: true },
              { label: 'Add Resources', to: '/infrastructure/add' },
            ],
          ],
        },
        { title: 'Universal Service Monitoring', to: '/usm', badge: 'NEW', columns: [] },
        {
          title: 'Hosts',
          to: '/infrastructure',
          columns: [
            [
              { label: 'Host Map', to: '/infrastructure/map' },
              { label: 'GPU Monitoring', to: '/gpu-monitoring' },
            ],
          ],
        },
        {
          title: 'Containers',
          columns: [
            [
              { label: 'Kubernetes Overview', to: '/kubernetes' },
              { label: 'Kubernetes Explorer', to: '/orchestration/explorer' },
              { label: 'Kubernetes Autoscaling', to: '/kubernetes/autoscaling', isNew: true },
              { label: 'Container Images', to: '/orchestration/explorer?resource=container_images' },
              { label: 'ECS Explorer', to: '/orchestration/explorer?resource=ecs_tasks' },
            ],
          ],
        },
        { title: 'Processes', to: '/process', columns: [] },
        { title: 'Serverless', to: '/functions', columns: [] },
        {
          title: 'Cloud Network',
          columns: [
            [
              { label: 'Analytics', to: '/network' },
              { label: 'Network Map', to: '/network/map' },
            ],
          ],
        },
        {
          title: 'Network Devices',
          columns: [
            [
              { label: 'Device Geomap', to: '/devices/geomap', isNew: true },
              { label: 'Device Topology Map', to: '/devices/topology' },
              { label: 'NetFlow', to: '/devices/netflow' },
            ],
            [{ label: 'Settings', to: '/devices/settings' }],
          ],
        },
        { title: 'Network Path', to: '/network-path', columns: [] },
        { title: 'Storage Management', to: '/storage-management', badge: 'NEW', columns: [] },
      ],
    },
    {
      label: 'Cloud Cost',
      to: '/cloud-cost',
      icon: Cloud,
      menu: [
        {
          title: 'Cloud Cost',
          columns: [
            [
              { label: 'Analytics', to: '/cloud-cost' },
              { label: 'Recommendations', to: '/cloud-cost/recommendations' },
            ],
          ],
        },
      ],
    },
    {
      label: 'APM',
      to: '/apm',
      icon: Network,
      menu: [
        {
          title: 'Application Performance Monitoring',
          columns: [
            [
              { label: 'Services', to: '/apm' },
              { label: 'Endpoints', to: '/apm/endpoints' },
              { label: 'Traces', to: '/apm/traces' },
              { label: 'Recommendations', to: '/apm/recommendations' },
            ],
            [
              { label: 'Instrument a Service', to: '/apm?instrument=1' },
              { label: 'Settings', to: '/apm/settings' },
            ],
          ],
        },
        {
          title: 'Profiles',
          columns: [
            [
              { label: 'Explorer', to: '/apm/profiles' },
              { label: 'Comparison', to: '/apm/profiles/comparison' },
              { label: 'Insights', to: '/apm/profiles/insights' },
            ],
          ],
        },
        {
          title: 'Live Debugger',
          badge: 'PREVIEW',
          columns: [[{ label: 'Sessions List', to: '/apm/debugger' }], [{ label: 'Configuration', to: '/apm/debugger/configuration' }]],
        },
        { title: 'Data Streams Monitoring', columns: [[{ label: 'Kafka Console', to: '/data-streams/kafka' }]] },
        { title: 'Dynamic Instrumentation', to: '/apm/dynamic-instrumentation', columns: [] },
        { title: 'Database Monitoring', to: '/data', columns: [] },
        { title: 'Data Jobs Monitoring', to: '/data/jobs', columns: [] },
        { title: 'Sensitive Data Scanner', to: '/organization-settings/sensitive-data-scanner', columns: [] },
      ],
    },
    {
      label: 'Digital Experience',
      to: '/rum',
      icon: Monitor,
      menu: [
        {
          title: 'Digital Experience',
          columns: [
            [
              { label: 'Real User Monitoring', to: '/rum' },
              { label: 'Session Replay', to: '/rum/replay' },
            ],
          ],
        },
        {
          title: 'Synthetic Tests',
          columns: [
            [
              { label: 'Tests', to: '/synthetics/tests', isNew: true },
              { label: 'New API Test', to: '/synthetics/create' },
            ],
          ],
        },
      ],
    },
    {
      label: 'Software Delivery',
      to: '/ci',
      icon: GitBranch,
      menu: [
        {
          title: 'CI/CD Optimization',
          badge: 'NEW',
          columns: [
            [
              { label: 'Overview', to: '/ci' },
              { label: 'Explorer', to: '/ci/explorer' },
              { label: 'Flaky Management', to: '/ci/flaky' },
            ],
            [
              { label: 'Add Repository', to: '/ci/settings#add-repository' },
              { label: 'Settings', to: '/ci/settings' },
            ],
          ],
        },
        {
          title: 'Feature Flags',
          columns: [[{ label: 'Feature Flags', to: '/feature-flags' }], [{ label: 'Environments', to: '/feature-flags/environments' }]],
        },
        { title: 'Deployment Gates', to: '/deployment-gates', badge: 'NEW', columns: [] },
        { title: 'Intro to Code Coverage', to: '/code-coverage', columns: [] },
        {
          title: 'Code Security',
          columns: [
            [
              { label: 'Vulnerabilities', to: '/code-security/vulnerabilities' },
              { label: 'Repositories', to: '/code-security/repositories' },
            ],
            [
              { label: 'Repository Settings', to: '/code-security/settings' },
              { label: 'Setup', to: '/code-security/setup' },
            ],
          ],
        },
        { title: 'PR Gates', to: '/pr-gates', badge: 'NEW', columns: [] },
        {
          title: 'Delivery Performance',
          columns: [
            [
              { label: 'DORA Metrics', to: '/delivery/dora' },
              { label: 'AI Impact', to: '/delivery/ai-impact', isPreview: true },
            ],
            [{ label: 'Settings', to: '/delivery/settings' }],
          ],
        },
      ],
    },
    {
      label: 'Security',
      to: '/security',
      icon: ShieldCheck,
      menu: [
        {
          title: 'Security',
          columns: [
            [
              { label: 'Signals', to: '/security' },
              { label: 'Vulnerabilities', to: '/security/vulnerabilities' },
            ],
          ],
        },
      ],
    },
    {
      label: 'Data Observability',
      to: '/data',
      icon: Database,
      menu: [
        {
          title: 'Data Observability',
          columns: [
            [
              { label: 'Database Monitoring', to: '/data' },
              { label: 'Data Jobs', to: '/data/jobs' },
            ],
          ],
        },
      ],
    },
    {
      label: 'AI Observability',
      to: '/llm',
      icon: Bot,
      menu: [
        {
          title: 'AI Observability',
          columns: [
            [
              { label: 'Applications', to: '/llm' },
              { label: 'LLM Traces', to: '/llm/traces' },
              { label: 'Cost', to: '/llm/cost' },
              { label: 'Evaluations', to: '/llm/evaluations' },
            ],
          ],
        },
      ],
    },
  ],
  [
    {
      label: 'Errors',
      to: '/errors',
      icon: Bug,
      menu: [{ title: 'Error Tracking', columns: [[{ label: 'Issues', to: '/errors' }]] }],
    },
    {
      label: 'Metrics',
      to: '/metrics',
      icon: Gauge,
      menu: [
        {
          title: 'Metrics',
          columns: [
            [
              { label: 'Explorer', to: '/metrics' },
              { label: 'Summary', to: '/metrics/summary' },
            ],
          ],
        },
      ],
    },
    {
      label: 'Logs',
      to: '/logs',
      icon: ScrollText,
      menu: [
        {
          title: 'Search & Analytics',
          columns: [
            [
              { label: 'Explorer', to: '/logs' },
              { label: 'Live Tail', to: '/logs/livetail' },
              { label: 'Notebooks', to: '/notebooks' },
            ],
            [{ label: 'Archive Search', to: '/logs/archive-search', isNew: true }],
          ],
        },
        {
          title: 'Log Configuration',
          columns: [
            [
              { label: 'Pipelines', to: '/logs/pipelines' },
              { label: 'Flex Logs Controls', to: '/logs/flex-controls' },
              { label: 'Archiving & Forwarding', to: '/logs/archives' },
            ],
            [
              { label: 'Add a Log Source', to: '/logs/onboarding' },
              { label: 'Generate Metrics', to: '/logs/generate-metrics' },
              { label: 'Data Access', to: '/logs/data-access' },
              { label: 'Migration', to: '/logs/migration/import', isNew: true },
            ],
          ],
        },
        {
          title: 'Observability Pipelines',
          columns: [
            [
              { label: 'Volume Control Packs', to: '/observability-pipelines/packs' },
              { label: 'New Pipeline', to: '/observability-pipelines/new' },
            ],
          ],
        },
        {
          title: 'Sensitive Data Scanner',
          columns: [[{ label: 'Scanning Groups', to: '/organization-settings/sensitive-data-scanner' }]],
        },
      ],
    },
  ],
];

export const NAV_FOOTER: NavItem[] = [
  {
    label: 'Integrations',
    to: '/integrations',
    icon: Puzzle,
    menu: [
      {
        title: 'Integrations',
        columns: [
          [
            { label: 'Integrations', to: '/integrations' },
            { label: 'Marketplace', to: '/integrations/marketplace' },
          ],
          [
            { label: 'Reference Tables', to: '/integrations/reference-tables' },
            { label: 'Link Source Code', to: '/integrations/source-code' },
          ],
        ],
      },
    ],
  },
  { label: 'Release Notes', to: '/release-notes', icon: Megaphone },
];

/** "+" quick-create entries. */
export const QUICK_CREATE: MenuLink[] = [
  { label: 'New Dashboard', to: '/dashboards?new=1' },
  { label: 'New Notebook', to: '/notebooks?new=1' },
  { label: 'Declare Incident', to: '/incidents?declare=1' },
  { label: 'New Monitor', to: '/monitors/create' },
  { label: 'New Log Pipeline', to: '/logs/pipelines' },
  { label: 'Invite Users', to: '/organization-settings/users' },
];
