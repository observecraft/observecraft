// Fleet Automation helpers without React: configuration diffs, integration logos, version order and
// the upgrade instructions per install method (scripts/fleet.test.mjs covers them).
import type { FleetAgentKind } from '../../api/fleet/types';

export interface DiffLine {
  kind: 'same' | 'add' | 'del';
  text: string;
  /** Line number on the left (removed or unchanged lines). */
  left?: number;
  /** Line number on the right (added or unchanged lines). */
  right?: number;
}

/** Line diff of two texts (longest common subsequence); removed lines come before added ones. */
export function lineDiff(a: string, b: string): DiffLine[] {
  const x = a.split('\n');
  const y = b.split('\n');
  const n = x.length;
  const m = y.length;
  // lcs[i][j]: common lines of x[i..] and y[j..].
  const lcs: number[][] = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) lcs[i][j] = x[i] === y[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
  }
  const out: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < n || j < m) {
    if (i < n && j < m && x[i] === y[j]) {
      out.push({ kind: 'same', text: x[i], left: i + 1, right: j + 1 });
      i++;
      j++;
    } else if (i < n && (j >= m || lcs[i + 1][j] >= lcs[i][j + 1])) {
      out.push({ kind: 'del', text: x[i], left: i + 1 });
      i++;
    } else {
      out.push({ kind: 'add', text: y[j], right: j + 1 });
      j++;
    }
  }
  return out;
}

export function diffStats(diff: DiffLine[]): { added: number; removed: number } {
  return { added: diff.filter((d) => d.kind === 'add').length, removed: diff.filter((d) => d.kind === 'del').length };
}

/** Datadog check names whose logo key differs from the name. */
const CHECK_LOGOS: Record<string, string> = {
  redisdb: 'redis',
  postgres: 'postgresql',
  mysql: 'mysql',
  mongo: 'mongodb',
  kubelet: 'kubernetes',
  kubernetes_state: 'kubernetes',
  kubernetes_state_core: 'kubernetes',
  kube_apiserver_metrics: 'kubernetes',
  docker: 'docker',
  containerd: 'containerd',
  nginx: 'nginx',
  nginx_ingress_controller: 'nginx',
  kafka: 'apachekafka',
  kafka_consumer: 'apachekafka',
  rabbitmq: 'rabbitmq',
  elastic: 'elasticsearch',
  cassandra: 'apachecassandra',
  etcd: 'etcd',
  consul: 'consul',
  vault: 'vault',
  envoy: 'envoyproxy',
  istio: 'istio',
  openmetrics: 'prometheus',
  prometheus: 'prometheus',
  tomcat: 'apachetomcat',
  apache: 'apache',
  gunicorn: 'gunicorn',
  airflow: 'apacheairflow',
  argocd: 'argo',
  nvidia_dcgm: 'nvidia',
  gpu: 'nvidia',
  system: 'linux',
};

/** The IntegrationLogo key of a check: a brand logo when we have one, else a lettered tile. */
export function integrationLogo(check: string): string {
  return CHECK_LOGOS[check] ?? `tile:${check}`;
}

/** -1, 0 or 1 like a comparator; `7.58.2-1` is a package revision, `7.63.0-rc.1` a pre-release. */
export function compareVersions(a: string, b: string): number {
  const parse = (v: string) => {
    const label = (v.trim().split(/\s+/).pop() ?? '').replace(/^v/, '').split('+')[0];
    const dash = label.indexOf('-');
    const core = (dash < 0 ? label : label.slice(0, dash)).split('.').map((p) => Number(p) || 0);
    const pre = dash < 0 ? '' : label.slice(dash + 1);
    return { core, pre: /^\d+$/.test(pre) ? '' : pre };
  };
  const x = parse(a);
  const y = parse(b);
  for (let i = 0; i < Math.max(x.core.length, y.core.length); i++) {
    const d = (x.core[i] ?? 0) - (y.core[i] ?? 0);
    if (d) return Math.sign(d);
  }
  if (x.pre === y.pre) return 0;
  if (!x.pre) return 1;
  if (!y.pre) return -1;
  return x.pre.localeCompare(y.pre, undefined, { numeric: true });
}

export interface VersionGroup<A> {
  kind: FleetAgentKind;
  version: string;
  agents: A[];
}

/** Outdated agents per kind and version, oldest version first (Upgrades). */
export function outdatedByVersion<A extends { kind: FleetAgentKind; version?: string | null; outdated: boolean }>(agents: A[]): VersionGroup<A>[] {
  const groups = new Map<string, VersionGroup<A>>();
  for (const a of agents) {
    if (!a.outdated || !a.version) continue;
    const key = `${a.kind} ${a.version}`;
    const g = groups.get(key) ?? { kind: a.kind, version: a.version, agents: [] };
    g.agents.push(a);
    groups.set(key, g);
  }
  const kindOrder: FleetAgentKind[] = ['datadog_agent', 'otel_collector'];
  return [...groups.values()].sort((a, b) => kindOrder.indexOf(a.kind) - kindOrder.indexOf(b.kind) || compareVersions(a.version, b.version));
}

export type GuideKey = 'helm' | 'operator' | 'linux' | 'docker' | 'windows' | 'observecraft-agent' | 'otel' | 'unknown';

/** Which upgrade instructions fit an agent's install method. */
export function installGuide(method: string | null | undefined, kind: FleetAgentKind): GuideKey {
  const m = (method ?? '').toLowerCase();
  if (m === 'observecraft-agent') return 'observecraft-agent';
  if (kind === 'otel_collector') return 'otel';
  if (m.includes('operator')) return 'operator';
  if (m.includes('helm')) return 'helm';
  if (m.includes('docker') || m.includes('ecs') || m.includes('container')) return 'docker';
  if (m.includes('msi') || m.includes('windows')) return 'windows';
  if (/install_script|apt|yum|dnf|zypper|dpkg|rpm|deb|chef|puppet|ansible|salt/.test(m)) return 'linux';
  return 'unknown';
}

export interface UpgradeGuide {
  title: string;
  intro: string;
  steps: { text: string; code?: string }[];
}

/** How to upgrade, per install method, in our own words. ObserveCraft never upgrades agents itself. */
export const UPGRADE_GUIDES: Record<GuideKey, UpgradeGuide> = {
  helm: {
    title: 'Datadog Helm chart',
    intro: 'Agents deployed with the Datadog Helm chart run the image tag the chart sets. Upgrade the release to move every node at once.',
    steps: [
      { text: 'Refresh the chart repository.', code: 'helm repo update datadog' },
      {
        text: 'Upgrade the release, keeping your values. Pin a version with agents.image.tag, or leave it out to take the chart default.',
        code: 'helm upgrade <release> datadog/datadog -n <namespace> --reuse-values \\\n  --set agents.image.tag=<version>',
      },
      { text: 'Watch the DaemonSet roll out; the agents report their new version within a few minutes.', code: 'kubectl rollout status daemonset/<release>-datadog -n <namespace>' },
    ],
  },
  operator: {
    title: 'Datadog Operator',
    intro: 'The Operator reconciles the DatadogAgent resource. Change the image there and it replaces the agent pods.',
    steps: [
      { text: 'Set the version in the DatadogAgent resource.', code: 'spec:\n  override:\n    nodeAgent:\n      image:\n        tag: <version>' },
      { text: 'Apply it and let the Operator roll the pods.', code: 'kubectl apply -f datadog-agent.yaml' },
    ],
  },
  linux: {
    title: 'Linux package',
    intro: 'Agents installed with the install script or a package manager upgrade like any other package on the host.',
    steps: [
      { text: 'Debian and Ubuntu:', code: 'sudo apt-get update && sudo apt-get install --only-upgrade datadog-agent' },
      { text: 'RHEL, Amazon Linux, Rocky and other RPM systems:', code: 'sudo yum makecache && sudo yum update datadog-agent' },
      { text: 'The package restarts the agent; check it afterwards.', code: 'sudo datadog-agent status' },
    ],
  },
  docker: {
    title: 'Container',
    intro: 'A containerised agent upgrades by running a newer image.',
    steps: [
      { text: 'Pull the new image and recreate the container with the same environment and mounts.', code: 'docker pull gcr.io/datadoghq/agent:<version>' },
      { text: 'On ECS, update the task definition with the new image tag and deploy the service.' },
    ],
  },
  windows: {
    title: 'Windows installer',
    intro: 'On Windows the agent upgrades in place with the installer of the new version.',
    steps: [{ text: 'Run the MSI of the new version; it keeps the existing configuration.', code: 'msiexec /qn /i datadog-agent-7-latest.amd64.msi' }],
  },
  'observecraft-agent': {
    title: 'observecraft-agent Helm chart',
    intro: 'The chart runs the official OpenTelemetry Collector image. Upgrading the chart moves the collectors to the version it ships.',
    steps: [
      { text: 'Upgrade the release, keeping your values.', code: 'helm upgrade observecraft-agent oci://ghcr.io/observecraft/charts/observecraft-agent \\\n  -n observecraft --reuse-values' },
      { text: 'Or pin a Collector version explicitly.', code: '--set image.tag=<version>' },
    ],
  },
  otel: {
    title: 'OpenTelemetry Collector',
    intro: 'Collectors you deployed yourself upgrade where they run: change the image tag of the container, or install the newer otelcol-contrib package.',
    steps: [
      { text: 'Kubernetes: change the image of the DaemonSet or Deployment.', code: 'kubectl set image daemonset/<name> <container>=otel/opentelemetry-collector-contrib:<version>' },
      { text: 'Packages: install the newer otelcol-contrib release from the Collector releases page and restart the service.', code: 'sudo systemctl restart otelcol-contrib' },
    ],
  },
  unknown: {
    title: 'Other install methods',
    intro: 'These agents don’t say how they were installed. Upgrade them with the tool that deployed them (configuration management, image pipeline or package manager).',
    steps: [{ text: 'After the upgrade the agent reports its new version with its next host metadata (within 30 minutes).' }],
  },
};

const KIND_LABELS: Record<FleetAgentKind, string> = { datadog_agent: 'Datadog Agent', otel_collector: 'OTel Collector' };
export const kindLabel = (k: FleetAgentKind) => KIND_LABELS[k];

/** "3 min ago", "2 hr ago", "4 days ago". */
export function ago(ms: number | null | undefined, now = Date.now()): string {
  if (ms == null) return '—';
  const s = Math.max(0, Math.round((now - ms) / 1000));
  if (s < 60) return 'just now';
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min ago`;
  const h = Math.round(m / 60);
  if (h < 48) return `${h} hr ago`;
  return `${Math.round(h / 24)} days ago`;
}
