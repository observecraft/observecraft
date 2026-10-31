import assert from 'node:assert/strict';
import test from 'node:test';
import { registerHooks } from 'node:module';

registerHooks({
  resolve(specifier, context, next) {
    try {
      return next(specifier, context);
    } catch (err) {
      if (specifier.startsWith('.')) return next(`${specifier}.ts`, context);
      throw err;
    }
  },
});

const { NAV_SECTIONS, NAV_FOOTER } = await import('../web/src/components/shell/navMenus.ts');
const ITEMS = [...NAV_SECTIONS.flat(), ...NAV_FOOTER];

// The sidebar's hover menus, as the reference product shows them (Oct 2026): per item, its groups as
// "Title[!BADGE] = left items | right items". "+" marks create actions; "!NEW"/"!PREVIEW" a badge.
// Products the reference calls "Bits …" are "Assist …" here.
const REFERENCE = {
  'Assist AI': [
    'Assist Chat!NEW = | Reports, Settings',
    'Assist Investigation!NEW = +New Investigation, Investigations List, Monitors | Reports, Settings',
    'Assist Code!NEW = Sessions, Automations | Usage, Settings',
    'Assist Security Analyst!NEW',
    'Assist Agent Builder!NEW',
    'AI Credits Management',
    'Usage!NEW',
  ],
  Dashboards: ['Dashboards = Dashboard List, +New Dashboard, Reports', 'Notebooks = Notebooks List, +New Notebook', 'Sheets = Spreadsheets List, +New Spreadsheet', 'Quick Graph', 'SQL Editor'],
  Monitoring: [
    'Monitors = Monitor List, +New Monitor, Triggered, Downtimes, Quality | Settings',
    'SLOs',
    'Check Summary',
    'Event Management = Triage Inbox, All Events | Settings',
    'Watchdog',
    'External Provider Status!NEW',
  ],
  'Developer Portal': ['Internal Developer Portal = Catalog, Scorecards, Reports, Surveys, Actions | Settings'],
  'Incident Response': ['On-Call = Teams, Pages, +Send a Page | Profile, Settings', 'Incident Management = Incident List, +Declare Incident | Settings', 'Status Pages!NEW', 'Work Management'],
  Automation: ['Agent Builder', 'Workflow Automation = +New Workflow', 'Apps = +New App', 'Forms', 'Action Catalog', 'Work Management'],
  Infrastructure: [
    'Resources = Catalog, Cloudcraft, Changes!PREVIEW | Policies!PREVIEW, Add Resources',
    'Universal Service Monitoring!NEW',
    'Hosts = Host Map, GPU Monitoring',
    'Containers = Kubernetes Overview, Kubernetes Explorer, Kubernetes Autoscaling!NEW, Container Images, ECS Explorer',
    'Processes',
    'Serverless',
    'Cloud Network = Analytics, Network Map',
    'Network Devices = Device Geomap!NEW, Device Topology Map, NetFlow | Settings',
    'Network Path',
    'Storage Management!NEW',
    'End User Devices!PREVIEW',
  ],
  'Cloud Cost': [
    'Summarize = Overview, Containers, AI Costs!NEW | Settings, Data History, Onboarding Checklist',
    'Analyze = Explorer, Reports',
    'Monitor',
    'Optimize = Recommendations, Automations!NEW',
    'Allocate = Tag Pipelines, Custom Allocation Rules',
    'Plan = Budgets, Commitment Programs',
  ],
  APM: [
    'Application Performance Monitoring = Services, Endpoints, Traces, Recommendations | +Instrument a Service, Settings',
    'Profiles = Explorer, Comparison, Insights',
    'Live Debugger!PREVIEW = Sessions List | Configuration',
    'Data Streams Monitoring = Kafka Console',
    'Dynamic Instrumentation',
    'Database Monitoring',
    'Data Jobs Monitoring',
    'Sensitive Data Scanner',
  ],
  'Digital Experience': [
    'Journey Monitoring!NEW',
    'Synthetic Monitoring & Testing = Tests, +New Test, Test Suites, +New Test Suite, AI Testing!NEW, Results Explorer, Continuous Testing, Test Coverage | Settings',
    'Real User Monitoring = Summary, Operations!NEW, Investigate, Explore | Manage Applications, Generate Metrics, Sensitive Data Scanner',
    'Session Replay = Recordings, Playlists, Heatmaps',
    'Product Analytics',
    'Experiments',
  ],
  'Software Delivery': [
    'CI/CD Optimization!NEW = Overview, Explorer, Flaky Management | Add Repository, Settings',
    'Feature Flags = Feature Flags | Environments',
    'Deployment Gates!NEW',
    'Intro to Code Coverage',
    'Code Security = Vulnerabilities, Repositories | Detection Coverage, Settings',
    'PR Gates!NEW',
    'Delivery Performance = DORA Metrics, AI Impact!PREVIEW | Settings',
  ],
  Security: [
    'Security = Inbox, Signals | Research Feed, Settings',
    'Cloud SIEM = Content Packs, Signals | Detection Rules, Settings',
    'Code Security = Vulnerabilities, Repositories | Detection Coverage, Settings',
    'Cloud Security = Misconfigurations, Vulnerabilities | Inventory, CVE Explorer',
    'App & API Protection = API Inventory, Signals | Policies, Settings',
    'Workload Protection',
    'Sensitive Data Scanner = Findings | Settings',
    'AI Guard!NEW',
  ],
  'Data Observability': ['Data Observability!NEW = Catalog!NEW | Settings', 'Quality Monitoring!NEW = Monitors', 'Jobs Monitoring = Jobs, Queries!PREVIEW', 'Lineage!NEW', 'CI/CD!PREVIEW'],
  'AI Observability': [
    'Agent Observability = Overview, Cost!NEW, Traces, Evaluators, Experiments | AI Gateway!PREVIEW',
    'GPU Monitoring = Summary, Explorer, Capacity Planning!PREVIEW, Training!PREVIEW, Inference!PREVIEW',
    'Agent Console!PREVIEW',
  ],
  Errors: [],
  Metrics: ['Metrics = Overview, Explorer, Catalog, Derived Metrics | Settings!PREVIEW'],
  Logs: [
    'Search & Analytics = Explorer, Live Tail, Notebooks | Archive Search!NEW',
    'Log Configuration = Pipelines, Indexes, Flex Logs Controls, Archiving & Forwarding | Add a Log Source, Generate Metrics, Data Access, Migration!NEW',
    'Observability Pipelines = Volume Control Packs, +New Pipeline',
    'Sensitive Data Scanner',
  ],
  Integrations: ['Integrations', 'Marketplace', 'Reference Tables', 'Source Code Integration', 'Fleet Automation = Fleet View, +Install Agents, Configuration!NEW, Upgrades', 'IDE Plugins'],
};

const link = (l) => `${l.plus ? '+' : ''}${l.label}${l.isNew ? '!NEW' : ''}${l.isPreview ? '!PREVIEW' : ''}`;
const describe = (g) => {
  const head = `${g.title}${g.badge ? `!${g.badge}` : ''}`;
  if (!g.columns.length) return head;
  const [left = [], right = []] = g.columns;
  const l = left.map(link).join(', ');
  const r = right.map(link).join(', ');
  return `${head} =${l ? ` ${l}` : ''}${r ? ` | ${r}` : ''}`;
};

test('every sidebar hover menu matches the reference', () => {
  const items = ITEMS;
  for (const [label, groups] of Object.entries(REFERENCE)) {
    const item = items.find((i) => i.label === label);
    assert.ok(item, `sidebar item ${label}`);
    assert.deepEqual((item.menu ?? []).map(describe), groups, label);
  }
});

test('every menu link points at an app route, never at an external docs site', () => {
  for (const item of ITEMS)
    for (const g of item.menu ?? []) {
      if (g.to) assert.ok(g.to.startsWith('/'), `${item.label} › ${g.title}`);
      for (const col of g.columns) for (const l of col) assert.ok(l.to.startsWith('/'), `${item.label} › ${g.title} › ${l.label}`);
    }
});
