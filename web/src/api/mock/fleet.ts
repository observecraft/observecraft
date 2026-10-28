// In-browser Fleet Automation for mock mode: the demo shop's fictional fleet (Datadog Agents on the
// shop-prod-eu and shop-staging clusters and a few standalone hosts, plus two OpenTelemetry
// Collectors), evaluated like the API: statuses from metric and metadata freshness and failing checks,
// outdated against the newest version per kind, summary and facets over the whole fleet, the query on
// the list. Configurations are already sanitised, as the API returns them.
import type {
  FleetAgent,
  FleetAgentDetail,
  FleetAgentKind,
  FleetAgentRequest,
  FleetCheck,
  FleetConfigurationsRequest,
  FleetFacet,
  FleetLogLine,
  FleetPage,
  FleetProfile,
  FleetSearchRequest,
  FleetServiceCheck,
  FleetStatus,
  FleetVersion,
} from '../fleet/types';
import { parseQuery, type Term } from '../../lib/searchQuery';
import { toYaml } from '../../lib/yaml';
import { compareVersions } from '../../pages/fleet/fleetModel';
import { matchesTerms } from './match';

const MIN = 60_000;
const METRICS_FRESH = 10 * MIN;
const METADATA_FRESH = 60 * MIN;

interface Seed {
  n: number;
  kind: FleetAgentKind;
  version: string;
  env: string | null;
  cluster: string | null;
  os: string;
  platform: string;
  install: string | null;
  integrations: string[];
  /** Minutes before the end of the range of the last metric / metadata (null: never). */
  metricsAgo: number | null;
  metadataAgo: number | null;
  checks?: { name: string; status: 'OK' | 'WARNING' | 'ERROR'; message?: string; loadError?: boolean }[];
  serviceChecks?: FleetServiceCheck[];
  errors?: string[];
  features?: Record<string, boolean>;
}

const K8S = ['kubelet', 'containerd', 'system'];
const SEEDS: Seed[] = [
  {
    n: 1,
    kind: 'datadog_agent',
    version: '7.62.2',
    env: 'prod',
    cluster: 'shop-prod-eu',
    os: 'linux',
    platform: 'ubuntu 24.04',
    install: 'helm',
    integrations: [...K8S, 'redisdb'],
    metricsAgo: 0.5,
    metadataAgo: 6,
  },
  {
    n: 2,
    kind: 'datadog_agent',
    version: '7.62.2',
    env: 'prod',
    cluster: 'shop-prod-eu',
    os: 'linux',
    platform: 'ubuntu 24.04',
    install: 'helm',
    integrations: [...K8S, 'nginx'],
    metricsAgo: 0.5,
    metadataAgo: 3,
  },
  {
    n: 3,
    kind: 'datadog_agent',
    version: '7.61.0',
    env: 'prod',
    cluster: 'shop-prod-eu',
    os: 'linux',
    platform: 'ubuntu 24.04',
    install: 'helm',
    integrations: [...K8S, 'postgres'],
    metricsAgo: 0.5,
    metadataAgo: 8,
    checks: [{ name: 'postgres', status: 'ERROR', message: 'password authentication failed for user "datadog"' }],
    serviceChecks: [{ name: 'postgres.can_connect', status: 'CRITICAL' }],
    errors: ['CORE | ERROR | (pkg/collector/worker/check_logger.go:71 in Error) | check:postgres | Error running check: password authentication failed for user "datadog"'],
  },
  {
    n: 4,
    kind: 'datadog_agent',
    version: '7.62.2',
    env: 'prod',
    cluster: 'shop-prod-eu',
    os: 'linux',
    platform: 'ubuntu 24.04',
    install: 'helm',
    integrations: [...K8S, 'kafka'],
    metricsAgo: 1,
    metadataAgo: 12,
  },
  {
    n: 5,
    kind: 'datadog_agent',
    version: '7.60.1',
    env: 'staging',
    cluster: 'shop-staging',
    os: 'linux',
    platform: 'ubuntu 22.04',
    install: 'helm',
    integrations: [...K8S, 'redisdb'],
    metricsAgo: 34,
    metadataAgo: 4,
    errors: [
      'CORE | ERROR | (comp/forwarder/defaultforwarder/transaction/transaction.go:433 in internalProcess) | Error while processing transaction: error "429 Too Many Requests" while sending transaction, rescheduling it',
    ],
  },
  {
    n: 6,
    kind: 'datadog_agent',
    version: '7.62.2',
    env: 'staging',
    cluster: 'shop-staging',
    os: 'linux',
    platform: 'ubuntu 22.04',
    install: 'helm',
    integrations: [...K8S, 'nginx'],
    metricsAgo: 0.5,
    metadataAgo: 9,
  },
  {
    n: 7,
    kind: 'datadog_agent',
    version: '7.58.2',
    env: 'prod',
    cluster: null,
    os: 'linux',
    platform: 'amzn 2023',
    install: 'install_script_agent7',
    integrations: ['postgres', 'system'],
    metricsAgo: 0.5,
    metadataAgo: 15,
    checks: [{ name: 'postgres', status: 'WARNING', message: 'pg_stat_statements is not loaded: query metrics are unavailable' }],
    features: { feature_logs_enabled: false },
  },
  {
    n: 8,
    kind: 'datadog_agent',
    version: '7.59.0',
    env: 'prod',
    cluster: null,
    os: 'linux',
    platform: 'debian 12',
    install: 'apt',
    integrations: ['nginx', 'system'],
    metricsAgo: 0.5,
    metadataAgo: 20,
    features: { feature_logs_enabled: false },
  },
  {
    n: 9,
    kind: 'datadog_agent',
    version: '7.62.1',
    env: 'prod',
    cluster: null,
    os: 'windows',
    platform: 'Windows Server 2022 Datacenter',
    install: 'windows_msi',
    integrations: ['iis', 'system'],
    metricsAgo: 0.5,
    metadataAgo: 22,
  },
  {
    n: 12,
    kind: 'datadog_agent',
    version: '7.58.2',
    env: 'staging',
    cluster: 'shop-staging',
    os: 'linux',
    platform: 'ubuntu 22.04',
    install: 'helm',
    integrations: [...K8S, 'redisdb', 'postgres'],
    metricsAgo: 104,
    metadataAgo: 100,
    checks: [{ name: 'redisdb', status: 'ERROR', message: 'Error 111 connecting to redis-master:6379. Connection refused.', loadError: false }],
  },
  {
    n: 10,
    kind: 'otel_collector',
    version: '0.163.2',
    env: 'prod',
    cluster: 'shop-prod-eu',
    os: 'linux',
    platform: 'Ubuntu 24.04.1 LTS',
    install: 'observecraft-agent',
    integrations: [],
    metricsAgo: 0.3,
    metadataAgo: null,
  },
  {
    n: 11,
    kind: 'otel_collector',
    version: '0.160.0',
    env: 'staging',
    cluster: null,
    os: 'linux',
    platform: 'Debian GNU/Linux 12 (bookworm)',
    install: null,
    integrations: [],
    metricsAgo: 0.3,
    metadataAgo: null,
  },
];
/** Hosts seen only through a cloud integration (no agent): they lower the coverage. */
const UNCOVERED_HOSTS = 2;

const hostname = (n: number) => `i-0a1b2c3d4e5f6${String(n).padStart(4, '0')}`;

interface Row {
  agent: FleetAgent;
  checks: FleetCheck[];
  serviceChecks: FleetServiceCheck[];
  config: Record<string, unknown> | null;
  errors: FleetLogLine[];
}

function status(now: number, metadataAt: number | null, metricsAt: number | null, checks: FleetCheck[], serviceChecks: FleetServiceCheck[]): [FleetStatus, string[]] {
  const fresh = (t: number | null, w: number) => t != null && t >= now - w;
  const metricsOk = fresh(metricsAt, METRICS_FRESH);
  if (!metricsOk && !fresh(metadataAt, METADATA_FRESH)) return ['offline', ['Not reporting: no metrics in the last 10 minutes']];
  const issues: string[] = [];
  if (!metricsOk) issues.push(metricsAt != null ? 'Metadata still arrives, but no metrics in the last 10 minutes' : 'Metadata arrives, but no metrics from this agent');
  for (const c of checks.filter((c) => c.status !== 'OK')) {
    const what = c.loadError ? 'failed to load' : `is in ${c.status}`;
    issues.push(c.message ? `Check ${c.name} ${what}: ${c.message}` : `Check ${c.name} ${what}`);
  }
  for (const s of serviceChecks.filter((s) => s.status === 'WARNING' || s.status === 'CRITICAL')) issues.push(`Service check ${s.name} is ${s.status}`);
  return [issues.length ? 'issues' : 'ok', issues];
}

function configOf(s: Seed): Record<string, unknown> | null {
  if (s.kind !== 'datadog_agent') return null;
  const features = { feature_apm_enabled: s.cluster != null, feature_logs_enabled: true, feature_process_enabled: s.cluster != null, ...s.features };
  return {
    agent: {
      'agent-flavor': 'agent',
      'install-method': { tool: s.install, tool_version: s.install === 'helm' ? 'helm-3.15.4' : s.install, installer_version: s.install === 'helm' ? 'datadog-3.80.0' : 'install_script-1.37.0' },
      logs: { transport: 'HTTP', auto_multi_line_detection_enabled: false },
      otlp: { enabled: false },
      python: '3.12.6',
    },
    integrations: s.integrations,
    inventory: { ...features, config_site: 'observecraft.example.com', config_dd_url: 'https://intake.observecraft.example.com', flavor: 'agent', install_method_tool: s.install },
    configuration: {
      api_key: '<redacted>',
      site: 'observecraft.example.com',
      dd_url: 'https://intake.observecraft.example.com',
      hostname: hostname(s.n),
      tags: [`env:${s.env}`, 'team:platform'],
      logs_enabled: features.feature_logs_enabled,
      apm_config: { enabled: features.feature_apm_enabled },
      process_config: { process_collection: { enabled: features.feature_process_enabled } },
      ...(s.cluster ? { cluster_name: s.cluster, kubelet_tls_verify: false } : {}),
      ...(s.n === 7 ? { proxy: { https: 'http://<redacted>@proxy.example.com:3128', no_proxy: ['169.254.169.254'] } } : {}),
    },
  };
}

function rows(range: { from: number; to: number }): Row[] {
  const now = Math.min(range.to, Date.now());
  const at = (ago: number | null) => (ago == null ? null : Math.round(now - ago * MIN));
  // In range: metadata up to an hour before it, or metrics in it.
  const seeds = SEEDS.filter((s) => {
    const metadataAt = at(s.metadataAgo);
    const metricsAt = at(s.metricsAgo);
    return (metadataAt != null && metadataAt >= range.from - METADATA_FRESH) || (metricsAt != null && metricsAt >= range.from);
  });
  const out: Row[] = seeds.map((s) => {
    const metricsAt = at(s.metricsAgo);
    const metadataAt = at(s.metadataAgo);
    const dd = s.kind === 'datadog_agent';
    const checks: FleetCheck[] = dd
      ? [...new Set(s.integrations.flatMap((i) => (i === 'system' ? ['cpu', 'disk', 'load', 'memory', 'network'] : [i])))].sort().map((name) => {
          const c = s.checks?.find((c) => c.name === name);
          return { name, status: c?.status ?? 'OK', message: c?.message ?? null, instances: name === 'postgres' || name === 'redisdb' ? 2 : 1, loadError: c?.loadError ?? false };
        })
      : [];
    const agentUp = metricsAt != null && metricsAt >= now - METRICS_FRESH ? 'OK' : 'CRITICAL';
    const serviceChecks: FleetServiceCheck[] = dd ? [{ name: 'datadog.agent.up', status: agentUp }, ...(s.serviceChecks ?? [])].sort((a, b) => a.name.localeCompare(b.name)) : [];
    const [st, issues] = status(now, metadataAt, metricsAt, checks, serviceChecks);
    return {
      agent: {
        id: `${s.kind}:${hostname(s.n)}`,
        hostname: hostname(s.n),
        kind: s.kind,
        version: s.version,
        outdated: false,
        os: s.os,
        platform: s.platform,
        status: st,
        issues,
        integrations: [...s.integrations].sort(),
        env: s.env,
        cluster: s.cluster,
        lastReported: Math.max(metricsAt ?? 0, metadataAt ?? 0) || null,
        installMethod: s.install,
      },
      checks,
      serviceChecks,
      config: configOf(s),
      errors: (s.errors ?? []).map((message, i) => ({ timestamp: now - (i * 7 + 3) * MIN, status: 'error', message })),
    };
  });
  // Outdated: older than the newest release of the same kind.
  for (const kind of ['datadog_agent', 'otel_collector'] as FleetAgentKind[]) {
    const latest = out
      .filter((r) => r.agent.kind === kind && r.agent.version && !r.agent.version.includes('-'))
      .map((r) => r.agent.version!)
      .sort(compareVersions)
      .at(-1);
    for (const r of out) if (r.agent.kind === kind && latest && r.agent.version) r.agent.outdated = compareVersions(r.agent.version, latest) < 0;
  }
  return out.sort((a, b) => a.agent.hostname.localeCompare(b.agent.hostname) || a.agent.kind.localeCompare(b.agent.kind));
}

function tags(a: FleetAgent): Record<string, string> {
  const t: Record<string, string> = { type: a.kind, status: a.status, host: a.hostname, outdated: String(a.outdated) };
  for (const [k, v] of Object.entries({ version: a.version, os: a.os, platform: a.platform, cluster: a.cluster, env: a.env, install_method: a.installMethod })) if (v) t[k] = v;
  return t;
}

function agentMatches(a: FleetAgent, terms: Term[]): boolean {
  for (const t of terms) {
    if (t.kind === 'tag' && t.key === 'integration' && t.values.some((v) => a.integrations.includes(v)) === t.negate) return false;
  }
  const rest = terms.filter((t) => !(t.kind === 'tag' && t.key === 'integration'));
  const tg = tags(a);
  return matchesTerms((k) => tg[k], `${a.hostname} ${a.version ?? ''} ${a.platform ?? ''}`, rest);
}

const FACET_KEYS = ['type', 'status', 'version', 'os', 'integration', 'cluster', 'env', 'install_method'];

function facets(agents: FleetAgent[]): FleetFacet[] {
  return FACET_KEYS.map((key) => {
    const counts = new Map<string, number>();
    for (const a of agents) {
      const values = key === 'integration' ? a.integrations : tags(a)[key] ? [tags(a)[key]] : [];
      for (const v of values) counts.set(v, (counts.get(v) ?? 0) + 1);
    }
    return { key, values: [...counts].map(([value, count]) => ({ value, count })).sort((x, y) => y.count - x.count || x.value.localeCompare(y.value)) };
  }).filter((f) => f.values.length);
}

function versions(agents: FleetAgent[]): FleetVersion[] {
  const counts = new Map<string, FleetVersion>();
  for (const a of agents) {
    const key = `${a.kind} ${a.version ?? ''}`;
    const v = counts.get(key) ?? { kind: a.kind, version: a.version ?? null, count: 0, outdated: false };
    v.count++;
    v.outdated ||= a.outdated;
    counts.set(key, v);
  }
  const order: FleetAgentKind[] = ['datadog_agent', 'otel_collector'];
  return [...counts.values()].sort((a, b) => order.indexOf(a.kind) - order.indexOf(b.kind) || compareVersions(b.version ?? '0', a.version ?? '0'));
}

export async function searchFleet(req: FleetSearchRequest): Promise<FleetPage> {
  const all = rows(req.range).map((r) => r.agent);
  const latest = (k: FleetAgentKind) =>
    all
      .filter((a) => a.kind === k && a.version)
      .map((a) => a.version!)
      .sort(compareVersions)
      .at(-1) ?? null;
  const hosts = new Set(all.map((a) => a.hostname)).size + UNCOVERED_HOSTS;
  const covered = new Set(all.map((a) => a.hostname)).size;
  const terms = parseQuery(req.query ?? '');
  return {
    enabled: all.length > 0,
    summary: {
      agents: all.length,
      outdated: all.filter((a) => a.outdated).length,
      withIssues: all.filter((a) => a.status === 'issues').length,
      offline: all.filter((a) => a.status === 'offline').length,
      coverage: { hosts, covered, pct: hosts ? Math.round((covered * 1000) / hosts) / 10 : null },
    },
    latest: { datadogAgent: latest('datadog_agent'), otelCollector: latest('otel_collector') },
    versions: versions(all),
    facets: facets(all),
    agents: all.filter((a) => agentMatches(a, terms)),
  };
}

export async function getFleetAgent(req: FleetAgentRequest): Promise<FleetAgentDetail> {
  const row = rows(req.range).find((r) => r.agent.id === req.id);
  if (!row) throw new Error(`Agent ${req.id} has not reported in this time range`);
  const services = row.agent.kind === 'datadog_agent' ? 'service:datadog-agent' : 'service:(otelcol-contrib OR otelcol OR observecraft-agent)';
  return {
    agent: row.agent,
    checks: row.checks,
    serviceChecks: row.serviceChecks,
    configYaml: row.config ? toYaml(sortKeys(row.config)) : null,
    fullConfigReported: row.config != null,
    errors: row.errors,
    logsQuery: `${services} host:${row.agent.hostname}`,
  };
}

/** Keys in alphabetical order at every level, as the API's YAML has them. */
function sortKeys(v: unknown): unknown {
  if (Array.isArray(v)) return v.map(sortKeys);
  if (v && typeof v === 'object')
    return Object.fromEntries(
      Object.entries(v)
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([k, x]) => [k, sortKeys(x)]),
    );
  return v;
}

function flatten(prefix: string, v: unknown, out: Record<string, string>) {
  if (v && typeof v === 'object' && !Array.isArray(v)) {
    for (const [k, x] of Object.entries(v)) flatten(prefix ? `${prefix}.${k}` : k, x, out);
  } else if (v != null) {
    out[prefix] = typeof v === 'string' ? v : JSON.stringify(v);
  }
}

export async function listFleetProfiles(req: FleetConfigurationsRequest): Promise<FleetProfile[]> {
  const groups = new Map<string, FleetProfile>();
  for (const r of rows(req.range)) {
    const settings: Record<string, string> = {};
    if (r.config) {
      flatten('', r.config.agent, settings);
      flatten('', r.config.inventory, settings);
    } else settings.install_method = r.agent.installMethod ?? 'unknown';
    for (const k of Object.keys(settings)) if (k === 'python' || k.endsWith('version')) delete settings[k];
    const sorted = Object.fromEntries(Object.entries(settings).sort(([a], [b]) => a.localeCompare(b)));
    const key = JSON.stringify([r.agent.kind, r.agent.integrations, sorted]);
    let id = 0;
    for (const c of key) id = (id * 31 + c.charCodeAt(0)) >>> 0;
    const p = groups.get(key) ?? { id: id.toString(16).padStart(8, '0'), kind: r.agent.kind, integrations: r.agent.integrations, settings: sorted, agents: [], versions: [] };
    p.agents.push({ id: r.agent.id, hostname: r.agent.hostname, version: r.agent.version });
    if (r.agent.version && !p.versions.includes(r.agent.version)) p.versions.push(r.agent.version);
    groups.set(key, p);
  }
  return [...groups.values()]
    .map((p) => ({ ...p, versions: p.versions.sort(compareVersions).reverse() }))
    .sort((a, b) => b.agents.length - a.agents.length || a.kind.localeCompare(b.kind) || a.id.localeCompare(b.id));
}
