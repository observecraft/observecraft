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
  /** A create action, shown with a "+" before its label. */
  plus?: boolean;
  isNew?: boolean;
  isPreview?: boolean;
  external?: boolean;
}

/** A titled block of the hover menu; each column is a vertical list of links. */
export interface MenuGroup {
  title: string;
  /** Makes the title itself a link. */
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

type Badge = MenuGroup['badge'];

/** A menu link; `+` before the label marks a create action, `!NEW` / `!PREVIEW` after it a badge. */
function link(spec: string, to: string): MenuLink {
  const badge = /!(NEW|PREVIEW)$/.exec(spec)?.[1];
  const label = spec.replace(/!(NEW|PREVIEW)$/, '').replace(/^\+/, '');
  return { label, to, ...(spec.startsWith('+') && { plus: true }), ...(badge === 'NEW' && { isNew: true }), ...(badge === 'PREVIEW' && { isPreview: true }) };
}

/** A group: its title (linking to `to`), left column and optional right column. */
function group(title: string, to: string, left: [string, string][] = [], right: [string, string][] = [], badge?: Badge): MenuGroup {
  // A right column keeps its place even when the left one is empty.
  const cols = right.length ? [left, right] : left.length ? [left] : [];
  const columns = cols.map((c) => c.map(([spec, href]) => link(spec, href)));
  return { title, to, ...(badge && { badge }), columns };
}

/** Primary navigation; its hover menus follow the reference product's (see scripts/nav-menus.test.mjs). */
export const NAV_SECTIONS: NavItem[][] = [
  [
    {
      label: 'Assist AI',
      to: '/assist',
      icon: Sparkles,
      menu: [
        group(
          'Assist Chat',
          '/assist',
          [],
          [
            ['Reports', '/assist/reports'],
            ['Settings', '/assist/settings'],
          ],
          'NEW',
        ),
        group(
          'Assist Investigation',
          '/assist/investigations',
          [
            ['+New Investigation', '/assist/investigations/new'],
            ['Investigations List', '/assist/investigations'],
            ['Monitors', '/assist/investigations/monitors'],
          ],
          [
            ['Reports', '/assist/investigations/reports'],
            ['Settings', '/assist/investigations/settings'],
          ],
          'NEW',
        ),
        group(
          'Assist Code',
          '/assist/code',
          [
            ['Sessions', '/assist/code'],
            ['Automations', '/assist/code/automations'],
          ],
          [
            ['Usage', '/assist/code/usage'],
            ['Settings', '/assist/code/settings'],
          ],
          'NEW',
        ),
        group('Assist Security Analyst', '/assist/security-analyst', [], [], 'NEW'),
        group('Assist Agent Builder', '/automation/agents', [], [], 'NEW'),
        group('AI Credits Management', '/assist/credits'),
        group('Usage', '/assist/usage', [], [], 'NEW'),
      ],
    },
    {
      label: 'Dashboards',
      to: '/dashboards',
      icon: LayoutDashboard,
      menu: [
        group('Dashboards', '/dashboards', [
          ['Dashboard List', '/dashboards'],
          ['+New Dashboard', '/dashboards?new=1'],
          ['Reports', '/dashboards/reports'],
        ]),
        group('Notebooks', '/notebooks', [
          ['Notebooks List', '/notebooks'],
          ['+New Notebook', '/notebooks?new=1'],
        ]),
        group('Sheets', '/sheets', [
          ['Spreadsheets List', '/sheets'],
          ['+New Spreadsheet', '/sheets?new=1'],
        ]),
        group('Quick Graph', '/dashboards/quick-graph'),
        group('SQL Editor', '/ddsql'),
      ],
    },
    {
      label: 'Monitoring',
      to: '/monitors',
      icon: Activity,
      menu: [
        group(
          'Monitors',
          '/monitors',
          [
            ['Monitor List', '/monitors'],
            ['+New Monitor', '/monitors/create'],
            ['Triggered', '/monitors/triggered'],
            ['Downtimes', '/monitors/downtimes'],
            ['Quality', '/monitors/quality'],
          ],
          [['Settings', '/monitors/settings']],
        ),
        group('SLOs', '/slo'),
        group('Check Summary', '/check/summary'),
        group(
          'Event Management',
          '/event/inbox',
          [
            ['Triage Inbox', '/event/inbox'],
            ['All Events', '/event/explorer'],
          ],
          [['Settings', '/event/settings']],
        ),
        group('Watchdog', '/watchdog'),
        group('External Provider Status', '/watchdog/external-provider-status', [], [], 'NEW'),
      ],
    },
    {
      label: 'Developer Portal',
      to: '/portal',
      icon: Code2,
      menu: [
        group(
          'Internal Developer Portal',
          '/portal/home',
          [
            ['Catalog', '/portal/catalog'],
            ['Scorecards', '/portal/scorecards'],
            ['Reports', '/portal/reports'],
            ['Surveys', '/portal/surveys'],
            ['Actions', '/portal/actions'],
          ],
          [['Settings', '/portal/settings']],
        ),
      ],
    },
    {
      label: 'Incident Response',
      to: '/incidents',
      icon: Siren,
      menu: [
        group(
          'On-Call',
          '/on-call',
          [
            ['Teams', '/on-call/teams'],
            ['Pages', '/on-call/pages'],
            ['+Send a Page', '/on-call/pages?new=1'],
          ],
          [
            ['Profile', '/on-call/profile'],
            ['Settings', '/on-call/settings'],
          ],
        ),
        group(
          'Incident Management',
          '/incidents',
          [
            ['Incident List', '/incidents'],
            ['+Declare Incident', '/incidents?declare=1'],
          ],
          [['Settings', '/incidents/settings']],
        ),
        group('Status Pages', '/status-pages', [], [], 'NEW'),
        group('Work Management', '/work'),
      ],
    },
    {
      label: 'Automation',
      to: '/automation',
      icon: Zap,
      menu: [
        group('Agent Builder', '/automation/agents'),
        group('Workflow Automation', '/automation', [['+New Workflow', '/automation/workflows/new']]),
        group('Apps', '/automation/apps', [['+New App', '/automation/apps/new']]),
        group('Forms', '/automation/forms'),
        group('Action Catalog', '/automation/actions'),
        group('Work Management', '/work'),
      ],
    },
  ],
  [
    {
      label: 'Infrastructure',
      to: '/infrastructure',
      icon: Boxes,
      menu: [
        group(
          'Resources',
          '/infrastructure/catalog',
          [
            ['Catalog', '/infrastructure/catalog'],
            ['Cloudcraft', '/cloudcraft'],
            ['Changes!PREVIEW', '/infrastructure/changes'],
          ],
          [
            ['Policies!PREVIEW', '/infrastructure/policies'],
            ['Add Resources', '/infrastructure/add'],
          ],
        ),
        group('Universal Service Monitoring', '/usm', [], [], 'NEW'),
        group('Hosts', '/infrastructure', [
          ['Host Map', '/infrastructure/map'],
          ['GPU Monitoring', '/gpu-monitoring'],
        ]),
        group('Containers', '/kubernetes', [
          ['Kubernetes Overview', '/kubernetes'],
          ['Kubernetes Explorer', '/orchestration/explorer'],
          ['Kubernetes Autoscaling!NEW', '/kubernetes/autoscaling'],
          ['Container Images', '/orchestration/explorer?resource=container_images'],
          ['ECS Explorer', '/orchestration/explorer?resource=ecs_tasks'],
        ]),
        group('Processes', '/process'),
        group('Serverless', '/serverless'),
        group('Cloud Network', '/network', [
          ['Analytics', '/network'],
          ['Network Map', '/network/map'],
        ]),
        group(
          'Network Devices',
          '/devices',
          [
            ['Device Geomap!NEW', '/devices/geomap'],
            ['Device Topology Map', '/devices/topology'],
            ['NetFlow', '/devices/netflow'],
          ],
          [['Settings', '/devices/settings']],
        ),
        group('Network Path', '/network-path'),
        group('Storage Management', '/storage-management', [], [], 'NEW'),
        group('End User Devices', '/end-user-devices', [], [], 'PREVIEW'),
      ],
    },
    {
      label: 'Cloud Cost',
      to: '/cloud-cost',
      icon: Cloud,
      menu: [
        group(
          'Summarize',
          '/cloud-cost',
          [
            ['Overview', '/cloud-cost'],
            ['Containers', '/cloud-cost/containers'],
            ['AI Costs!NEW', '/cloud-cost/ai'],
          ],
          [
            ['Settings', '/cloud-cost/settings'],
            ['Data History', '/cloud-cost/data-history'],
            ['Onboarding Checklist', '/cloud-cost/onboarding'],
          ],
        ),
        group('Analyze', '/cloud-cost/explorer', [
          ['Explorer', '/cloud-cost/explorer'],
          ['Reports', '/cloud-cost/reports'],
        ]),
        group('Monitor', '/monitors?q=cost'),
        group('Optimize', '/cloud-cost/recommendations', [
          ['Recommendations', '/cloud-cost/recommendations'],
          ['Automations!NEW', '/cloud-cost/automations'],
        ]),
        group('Allocate', '/cloud-cost/tag-pipelines', [
          ['Tag Pipelines', '/cloud-cost/tag-pipelines'],
          ['Custom Allocation Rules', '/cloud-cost/allocation-rules'],
        ]),
        group('Plan', '/cloud-cost/budgets', [
          ['Budgets', '/cloud-cost/budgets'],
          ['Commitment Programs', '/cloud-cost/commitments'],
        ]),
      ],
    },
    {
      label: 'APM',
      to: '/apm',
      icon: Network,
      menu: [
        group(
          'Application Performance Monitoring',
          '/apm',
          [
            ['Services', '/apm'],
            ['Endpoints', '/portal/catalog?section=endpoint&view=performance'],
            ['Traces', '/apm/traces'],
            ['Recommendations', '/apm/recommendations'],
          ],
          [
            ['+Instrument a Service', '/apm/instrument'],
            ['Settings', '/apm/settings'],
          ],
        ),
        group('Profiles', '/apm/profiles', [
          ['Explorer', '/apm/profiles'],
          ['Comparison', '/apm/profiles/comparison'],
          ['Insights', '/apm/profiles/insights'],
        ]),
        group('Live Debugger', '/apm/debugger', [['Sessions List', '/apm/debugger']], [['Configuration', '/apm/debugger/configuration']], 'PREVIEW'),
        group('Data Streams Monitoring', '/data-streams', [['Kafka Console', '/data-streams/kafka']]),
        group('Dynamic Instrumentation', '/apm/dynamic-instrumentation'),
        group('Database Monitoring', '/data'),
        group('Data Jobs Monitoring', '/data/jobs'),
        group('Sensitive Data Scanner', '/organization-settings/sensitive-data-scanner'),
      ],
    },
    {
      label: 'Digital Experience',
      to: '/synthetics',
      icon: Monitor,
      menu: [
        group('Journey Monitoring', '/journeys', [], [], 'NEW'),
        group(
          'Synthetic Monitoring & Testing',
          '/synthetics',
          [
            ['Tests', '/synthetics/tests'],
            ['+New Test', '/synthetics/templates'],
            ['Test Suites', '/synthetics/suites'],
            ['+New Test Suite', '/synthetics/suites/new'],
            ['AI Testing!NEW', '/synthetics/ai-testing'],
            ['Results Explorer', '/synthetics/explorer'],
            ['Continuous Testing', '/synthetics/continuous-testing'],
            ['Test Coverage', '/synthetics/test-coverage'],
          ],
          [['Settings', '/synthetics/settings']],
        ),
        group(
          'Real User Monitoring',
          '/rum',
          [
            ['Summary', '/rum'],
            ['Operations!NEW', '/rum/operations'],
            ['Investigate', '/rum/explorer?type=error'],
            ['Explore', '/rum/explorer'],
          ],
          [
            ['Manage Applications', '/rum/applications'],
            ['Generate Metrics', '/rum/generate-metrics'],
            ['Sensitive Data Scanner', '/organization-settings/sensitive-data-scanner'],
          ],
        ),
        group('Session Replay', '/rum/replay/sessions', [
          ['Recordings', '/rum/replay/sessions'],
          ['Playlists', '/rum/replay/playlists'],
          ['Heatmaps', '/rum/replay/heatmaps'],
        ]),
        group('Product Analytics', '/product-analytics'),
        group('Experiments', '/product-analytics/experiments'),
      ],
    },
    {
      label: 'Software Delivery',
      to: '/ci',
      icon: GitBranch,
      menu: [
        group(
          'CI/CD Optimization',
          '/ci',
          [
            ['Overview', '/ci'],
            ['Explorer', '/ci/explorer'],
            ['Flaky Management', '/ci/flaky'],
          ],
          [
            ['Add Repository', '/ci/settings#add-repository'],
            ['Settings', '/ci/settings'],
          ],
          'NEW',
        ),
        group('Feature Flags', '/feature-flags', [['Feature Flags', '/feature-flags']], [['Environments', '/feature-flags/environments']]),
        group('Deployment Gates', '/delivery/deployment-gates', [], [], 'NEW'),
        group('Intro to Code Coverage', '/ci/code-coverage'),
        group(
          'Code Security',
          '/code-security',
          [
            ['Vulnerabilities', '/code-security/vulnerabilities'],
            ['Repositories', '/code-security/repositories'],
          ],
          [
            ['Detection Coverage', '/code-security/detection-coverage'],
            ['Settings', '/code-security/settings'],
          ],
        ),
        group('PR Gates', '/delivery/pr-gates', [], [], 'NEW'),
        group(
          'Delivery Performance',
          '/delivery/dora',
          [
            ['DORA Metrics', '/delivery/dora'],
            ['AI Impact!PREVIEW', '/delivery/ai-impact'],
          ],
          [['Settings', '/delivery/settings']],
        ),
      ],
    },
    {
      label: 'Security',
      to: '/security',
      icon: ShieldCheck,
      menu: [
        group(
          'Security',
          '/security',
          [
            ['Inbox', '/security/inbox'],
            ['Signals', '/security/signals'],
          ],
          [
            ['Research Feed', '/security/feed'],
            ['Settings', '/security/settings'],
          ],
        ),
        group(
          'Cloud SIEM',
          '/security/siem',
          [
            ['Content Packs', '/security/siem/content-packs'],
            ['Signals', '/security/siem/signals'],
          ],
          [
            ['Detection Rules', '/security/siem/rules'],
            ['Settings', '/security/siem/settings'],
          ],
        ),
        group(
          'Code Security',
          '/code-security',
          [
            ['Vulnerabilities', '/code-security/vulnerabilities'],
            ['Repositories', '/code-security/repositories'],
          ],
          [
            ['Detection Coverage', '/code-security/detection-coverage'],
            ['Settings', '/code-security/settings'],
          ],
        ),
        group(
          'Cloud Security',
          '/security/cloud',
          [
            ['Misconfigurations', '/security/cloud/findings'],
            ['Vulnerabilities', '/security/cloud/vulnerabilities'],
          ],
          [
            ['Inventory', '/security/cloud/inventory'],
            ['CVE Explorer', '/security/cloud/inventory/cves'],
          ],
        ),
        group(
          'App & API Protection',
          '/security/appsec',
          [
            ['API Inventory', '/security/appsec/inventory'],
            ['Signals', '/security/appsec/signals'],
          ],
          [
            ['Policies', '/security/appsec/policies'],
            ['Settings', '/security/appsec/inventory/services'],
          ],
        ),
        group('Workload Protection', '/security/workload-protection'),
        group(
          'Sensitive Data Scanner',
          '/organization-settings/sensitive-data-scanner',
          [['Findings', '/sensitive-data-scanner/findings']],
          [['Settings', '/organization-settings/sensitive-data-scanner']],
        ),
        group('AI Guard', '/security/ai-guard', [], [], 'NEW'),
      ],
    },
    {
      label: 'Data Observability',
      to: '/data-obs',
      icon: Database,
      menu: [
        group('Data Observability', '/data-obs', [['Catalog!NEW', '/data-obs/catalog']], [['Settings', '/data-obs/settings']], 'NEW'),
        group('Quality Monitoring', '/data-obs/monitors', [['Monitors', '/data-obs/monitors']], [], 'NEW'),
        group('Jobs Monitoring', '/data/jobs', [
          ['Jobs', '/data/jobs'],
          ['Queries!PREVIEW', '/data-obs/queries'],
        ]),
        group('Lineage', '/data-obs/lineage', [], [], 'NEW'),
        group('CI/CD', '/data-obs/ci-cd', [], [], 'PREVIEW'),
      ],
    },
    {
      label: 'AI Observability',
      to: '/llm',
      icon: Bot,
      menu: [
        group(
          'Agent Observability',
          '/llm',
          [
            ['Overview', '/llm'],
            ['Cost!NEW', '/llm/cost'],
            ['Traces', '/llm/traces'],
            ['Evaluators', '/llm/evaluations'],
            ['Experiments', '/llm/experiments'],
          ],
          [['AI Gateway!PREVIEW', '/llm/gateway']],
        ),
        group('GPU Monitoring', '/gpu-monitoring', [
          ['Summary', '/gpu-monitoring'],
          ['Explorer', '/gpu-monitoring/explorer'],
          ['Capacity Planning!PREVIEW', '/gpu-monitoring/capacity'],
          ['Training!PREVIEW', '/gpu-monitoring/training'],
          ['Inference!PREVIEW', '/gpu-monitoring/inference'],
        ]),
        group('Agent Console', '/llm/agent-console', [], [], 'PREVIEW'),
      ],
    },
  ],
  [
    { label: 'Errors', to: '/errors', icon: Bug },
    {
      label: 'Metrics',
      to: '/metrics',
      icon: Gauge,
      menu: [
        group(
          'Metrics',
          '/metrics',
          [
            ['Overview', '/metrics/overview'],
            ['Explorer', '/metrics'],
            ['Catalog', '/metrics/catalog'],
            ['Derived Metrics', '/metrics/derived'],
          ],
          [['Settings!PREVIEW', '/metrics/settings']],
        ),
      ],
    },
    {
      label: 'Logs',
      to: '/logs',
      icon: ScrollText,
      menu: [
        group(
          'Search & Analytics',
          '/logs',
          [
            ['Explorer', '/logs'],
            ['Live Tail', '/logs/livetail'],
            ['Notebooks', '/notebooks'],
          ],
          [['Archive Search!NEW', '/logs/archive-search']],
        ),
        group(
          'Log Configuration',
          '/logs/pipelines',
          [
            ['Pipelines', '/logs/pipelines'],
            ['Indexes', '/logs/indexes'],
            ['Flex Logs Controls', '/logs/flex-controls'],
            ['Archiving & Forwarding', '/logs/archives'],
          ],
          [
            ['Add a Log Source', '/logs/onboarding'],
            ['Generate Metrics', '/logs/generate-metrics'],
            ['Data Access', '/logs/data-access'],
            ['Migration!NEW', '/logs/migration/import'],
          ],
        ),
        group('Observability Pipelines', '/observability-pipelines', [
          ['Volume Control Packs', '/observability-pipelines/packs'],
          ['+New Pipeline', '/observability-pipelines/new'],
        ]),
        group('Sensitive Data Scanner', '/organization-settings/sensitive-data-scanner'),
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
      group('Integrations', '/integrations'),
      group('Marketplace', '/integrations/marketplace'),
      group('Reference Tables', '/integrations/reference-tables'),
      group('Source Code Integration', '/integrations/source-code'),
      group('Fleet Automation', '/fleet', [
        ['Fleet View', '/fleet'],
        ['+Install Agents', '/infrastructure/add'],
        ['Configuration!NEW', '/fleet/configuration'],
        ['Upgrades', '/fleet/upgrades'],
      ]),
      group('IDE Plugins', '/integrations/ide'),
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
