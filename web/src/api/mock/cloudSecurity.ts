// In-browser Cloud Security for mock mode: the demo hosts' and images' SBOM packages (as oc-tracegen's
// sbom.rs sends them) matched against the demo's fictional advisories, misconfigurations of the
// fictional shop-prod cluster's workloads (the server's cloud_security_rules.rs), the inventory and a
// CVE explorer over the same advisories.
import type {
  AdvisoryDetail,
  CloudSecuritySummary,
  CvePage,
  CveQuery,
  FindingStatus,
  InfraPackage,
  InfraVulnerability,
  MisconfigFinding,
  MisconfigPage,
  MisconfigRuleStatus,
  ResourceCategory,
} from '../cloudSecurity/types';

const DAY = 86_400_000;
const now = Date.now();
const delay = <T>(v: T, ms = 150): Promise<T> => new Promise((r) => setTimeout(() => r(structuredClone(v)), ms));
const triage = new Map<string, string>();
const RANK = ['low', 'medium', 'high', 'critical'];

// ---------------------------------------------------------------- advisories

interface Adv {
  id: string;
  cve: string;
  summary: string;
  score: number;
  cwe: string;
  ecosystem: string;
  pkg: string;
  fixed: string;
  daysAgo: number;
}
const ADVISORIES: Adv[] = [
  {
    id: 'OCSA-DEMO-2026-0101',
    cve: 'CVE-2026-90101',
    summary: 'libexample-ssl3: certificate chain verification bypass',
    score: 9.8,
    cwe: 'CWE-295',
    ecosystem: 'Debian:12',
    pkg: 'libexample-ssl3',
    fixed: '3.0.15-1~deb12u1',
    daysAgo: 18,
  },
  {
    id: 'OCSA-DEMO-2026-0102',
    cve: 'CVE-2026-90102',
    summary: 'libexample-ssl3: out-of-bounds read in the record layer',
    score: 9.1,
    cwe: 'CWE-125',
    ecosystem: 'Debian:12',
    pkg: 'libexample-ssl3',
    fixed: '3.0.14-1~deb12u2',
    daysAgo: 64,
  },
  {
    id: 'OCSA-DEMO-2026-0103',
    cve: 'CVE-2026-90103',
    summary: 'example-zlib: heap overflow when inflating crafted streams',
    score: 8.8,
    cwe: 'CWE-787',
    ecosystem: 'Debian:12',
    pkg: 'example-zlib',
    fixed: '1.2.13.dfsg-1+deb12u1',
    daysAgo: 40,
  },
  {
    id: 'OCSA-DEMO-2026-0104',
    cve: 'CVE-2026-90104',
    summary: 'examplecurl: credentials leaked on cross-origin redirects',
    score: 5.3,
    cwe: 'CWE-200',
    ecosystem: 'Debian:12',
    pkg: 'examplecurl',
    fixed: '7.88.1-10+deb12u8',
    daysAgo: 9,
  },
  {
    id: 'OCSA-DEMO-2026-0105',
    cve: 'CVE-2026-90105',
    summary: 'example-musl: stack overflow in the DNS resolver',
    score: 9.8,
    cwe: 'CWE-121',
    ecosystem: 'Alpine:v3.19',
    pkg: 'example-musl',
    fixed: '1.2.4-r3',
    daysAgo: 27,
  },
  {
    id: 'OCSA-DEMO-2026-0106',
    cve: 'CVE-2026-90106',
    summary: 'example-busybox: command injection in the wget applet',
    score: 8.1,
    cwe: 'CWE-78',
    ecosystem: 'Alpine:v3.19',
    pkg: 'example-busybox',
    fixed: '1.36.1-r16',
    daysAgo: 3,
  },
  {
    id: 'OCSA-DEMO-2026-0107',
    cve: 'CVE-2026-90107',
    summary: 'example-sudo: local privilege escalation through the editor option',
    score: 7.8,
    cwe: 'CWE-269',
    ecosystem: 'Ubuntu:22.04:LTS',
    pkg: 'example-sudo',
    fixed: '1.9.9-1ubuntu2.5',
    daysAgo: 33,
  },
  {
    id: 'OCSA-DEMO-2026-0108',
    cve: 'CVE-2026-90108',
    summary: 'example-kernel-tools: use after free in the perf helper',
    score: 7.8,
    cwe: 'CWE-416',
    ecosystem: 'Ubuntu:22.04:LTS',
    pkg: 'example-kernel-tools',
    fixed: '5.15.0-125.135',
    daysAgo: 120,
  },
  {
    id: 'OCSA-DEMO-2026-0004',
    cve: 'CVE-2026-90004',
    summary: 'example-json: unbounded token growth in the data input parser',
    score: 7.5,
    cwe: 'CWE-400',
    ecosystem: 'Maven',
    pkg: 'com.example.json:json-core',
    fixed: '2.19.3',
    daysAgo: 12,
  },
  {
    id: 'OCSA-DEMO-2026-0099',
    cve: 'CVE-2026-90099',
    summary: 'example-xml: entity expansion denial of service',
    score: 6.5,
    cwe: 'CWE-776',
    ecosystem: 'Maven',
    pkg: 'com.example.xml:xml-parser',
    fixed: '4.2.1',
    daysAgo: 70,
  },
];
const severityOf = (s: number) => (s >= 9 ? 'critical' : s >= 7 ? 'high' : s >= 4 ? 'medium' : 'low');

// ---------------------------------------------------------------- SBOMs

const HOSTS = ['i-0a1b2c3d4e5f60001', 'i-0a1b2c3d4e5f60002', 'i-0a1b2c3d4e5f60003', 'i-0a1b2c3d4e5f60004', 'i-0a1b2c3d4e5f60005', 'i-0a1b2c3d4e5f60006'];
const HOST_PACKAGES: [string, string][] = [
  ['bash', '5.1-6ubuntu1.1'],
  ['coreutils', '8.32-4.1ubuntu1.2'],
  ['openssh-server', '1:8.9p1-3ubuntu0.10'],
  ['example-sudo', '1.9.9-1ubuntu2.4'],
  ['example-kernel-tools', '5.15.0-119.129'],
  ['ca-certificates', '20240203~22.04.1'],
];
const DEBIAN: [string, string][] = [
  ['libexample-ssl3', '3.0.11-1~deb12u2'],
  ['example-zlib', '1.2.13.dfsg-1'],
  ['examplecurl', '7.88.1-10+deb12u5'],
  ['libc6', '2.36-9+deb12u7'],
  ['tzdata', '2024a-0+deb12u1'],
];
const ALPINE: [string, string][] = [
  ['example-musl', '1.2.4-r2'],
  ['example-busybox', '1.36.1-r15'],
  ['ca-certificates', '20240226-r0'],
  ['libcrypto3', '3.1.4-r5'],
];
const IMAGES: [string, string, string, boolean][] = [
  ['storefront-api', '1.42.0', 'debian', true],
  ['storefront-api', '1.41.3', 'debian', false],
  ['order-service', '3.8.1', 'debian', true],
  ['catalog-service', '2.4.0', 'debian', true],
  ['billing', '5.0.2', 'debian', true],
  ['api-gateway', '0.19.4', 'alpine', true],
  ['notification-worker', '1.7.0', 'alpine', true],
];

interface Entity {
  id: string;
  kind: 'host' | 'container_image';
  name: string;
  host: string;
  running: boolean;
  os: string;
  packages: [string, string, string][]; // ecosystem, name, version
}
const ENTITIES: Entity[] = [
  ...HOSTS.map((h) => ({
    id: h,
    kind: 'host' as const,
    name: h,
    host: h,
    running: true,
    os: 'ubuntu 22.04',
    packages: HOST_PACKAGES.map(([n, v]) => ['Ubuntu:22.04:LTS', n, v] as [string, string, string]),
  })),
  ...IMAGES.map(([name, tag, base, running], i) => ({
    id: `sha256:${name.slice(0, 2)}${i}${'0'.repeat(60)}`,
    kind: 'container_image' as const,
    name: `registry.example.com/${name}:${tag}`,
    host: HOSTS[i % HOSTS.length],
    running,
    os: base === 'alpine' ? 'alpine 3.19' : 'debian 12',
    packages: [
      ...(base === 'alpine' ? ALPINE.map(([n, v]) => ['Alpine:v3.19', n, v] as [string, string, string]) : DEBIAN.map(([n, v]) => ['Debian:12', n, v] as [string, string, string])),
      ...(name === 'storefront-api' ? [['Maven', 'com.example.json:json-core', '2.15.0'] as [string, string, string]] : []),
    ],
  })),
];

export function allVulns(): InfraVulnerability[] {
  const out: InfraVulnerability[] = [];
  for (const e of ENTITIES) {
    for (const [eco, name, version] of e.packages) {
      for (const a of ADVISORIES.filter((x) => x.ecosystem === eco && x.pkg === name)) {
        const key = `csm-vuln:${a.id}:${e.id}:${name}`;
        out.push({
          key,
          advisoryId: a.id,
          cve: a.cve,
          aliases: [a.cve],
          title: a.summary,
          severity: severityOf(a.score),
          score: a.score,
          cwes: [a.cwe],
          ecosystem: eco,
          package: name,
          version,
          fixed: a.fixed,
          resourceId: e.id,
          resourceName: e.name,
          resourceKind: e.kind,
          host: e.host,
          env: 'prod',
          os: e.os,
          running: e.running,
          exploitAvailable: a.id === 'OCSA-DEMO-2026-0105',
          status: triage.get(key) ?? 'open',
          firstSeen: now - Math.min(a.daysAgo, 20) * DAY,
          lastSeen: now - 600_000,
        });
      }
    }
  }
  return out.sort((a, b) => (b.score ?? 0) - (a.score ?? 0) || a.key.localeCompare(b.key));
}

const keep = (s: string, f: FindingStatus) => f === 'all' || s === f;

export async function vulnerabilities(status: FindingStatus): Promise<InfraVulnerability[]> {
  return delay(allVulns().filter((v) => keep(v.status, status)));
}

// ---------------------------------------------------------------- misconfigurations

const RULES: Omit<MisconfigRuleStatus, 'evaluated' | 'failing'>[] = [
  {
    id: 'ocs-k8s-001',
    name: 'Containers should not run privileged',
    severity: 'critical',
    description: 'A privileged container has every capability of the host and can escape to the node.',
    remediation: 'Remove `securityContext.privileged: true`; grant only the capabilities the container needs.',
    frameworks: ['pod-security-standards:baseline', 'mitre:T1611-escape-to-host'],
  },
  {
    id: 'ocs-k8s-002',
    name: "Pods should not share the host's namespaces",
    severity: 'high',
    description: "Sharing the host network, PID or IPC namespace lets the pod see and reach the node's processes and traffic.",
    remediation: 'Set `hostNetwork`, `hostPID` and `hostIPC` to false.',
    frameworks: ['pod-security-standards:baseline'],
  },
  {
    id: 'ocs-k8s-003',
    name: 'Containers should not add dangerous capabilities',
    severity: 'high',
    description: "Capabilities such as SYS_ADMIN or NET_ADMIN give a container control over the node's kernel or network.",
    remediation: 'Drop ALL capabilities and add back only the ones the process needs (NET_BIND_SERVICE, …).',
    frameworks: ['pod-security-standards:baseline'],
  },
  {
    id: 'ocs-k8s-004',
    name: 'Pods should not mount host paths',
    severity: 'medium',
    description: "A hostPath volume exposes the node's file system to the pod.",
    remediation: 'Use persistent volumes, ConfigMaps or emptyDir volumes instead of hostPath.',
    frameworks: ['pod-security-standards:baseline'],
  },
  {
    id: 'ocs-k8s-005',
    name: 'Containers should not run as root',
    severity: 'medium',
    description: 'A process running as root inside the container is root on the node if it escapes.',
    remediation: 'Set `runAsNonRoot: true` and a non-zero `runAsUser` in the pod or container security context.',
    frameworks: ['pod-security-standards:restricted'],
  },
  {
    id: 'ocs-k8s-006',
    name: 'Containers should not allow privilege escalation',
    severity: 'medium',
    description: 'Without `allowPrivilegeEscalation: false`, a process can gain more privileges than its parent (setuid binaries).',
    remediation: 'Set `securityContext.allowPrivilegeEscalation: false` on every container.',
    frameworks: ['pod-security-standards:restricted'],
  },
  {
    id: 'ocs-k8s-007',
    name: 'Container images should be pinned',
    severity: 'medium',
    description: 'Images without a tag or tagged `latest` change under you: what runs is not what was reviewed and scanned.',
    remediation: 'Reference images by a version tag or, better, by digest.',
    frameworks: ['supply-chain'],
  },
  {
    id: 'ocs-k8s-008',
    name: 'Load balancers should restrict source addresses',
    severity: 'medium',
    description: 'A LoadBalancer service without source ranges is reachable from the whole internet.',
    remediation: 'Set `loadBalancerSourceRanges`, make the load balancer internal, or put it behind an ingress with authentication.',
    frameworks: ['network-exposure'],
  },
  {
    id: 'ocs-k8s-009',
    name: 'Containers should have a read-only root file system',
    severity: 'low',
    description: 'A writable root file system lets an attacker drop tools and persist changes in the container.',
    remediation: 'Set `readOnlyRootFilesystem: true` and mount writable emptyDir volumes where the process writes.',
    frameworks: ['pod-security-standards:restricted'],
  },
  {
    id: 'ocs-k8s-010',
    name: 'Containers should have resource limits',
    severity: 'low',
    description: 'Without memory and CPU limits, one container can starve the others on its node.',
    remediation: 'Set `resources.limits.memory` and `resources.limits.cpu` on every container.',
    frameworks: ['availability'],
  },
  {
    id: 'ocs-k8s-011',
    name: 'Pods should not mount the service account token unless they need it',
    severity: 'low',
    description: 'A mounted token lets anyone with code execution in the pod call the Kubernetes API as its service account.',
    remediation: "Set `automountServiceAccountToken: false` on pods that don't call the API.",
    frameworks: ['kubernetes-hardening'],
  },
  {
    id: 'ocs-k8s-012',
    name: 'Workloads should not run in the default namespace',
    severity: 'low',
    description: 'The default namespace usually has no network policies, quotas or RBAC boundaries of its own.',
    remediation: 'Move the workload to a namespace of its own.',
    frameworks: ['kubernetes-hardening'],
  },
  {
    id: 'ocs-k8s-013',
    name: 'Services should not use NodePort',
    severity: 'low',
    description: 'A NodePort service opens the port on every node of the cluster.',
    remediation: 'Use a ClusterIP service behind an ingress or an internal load balancer.',
    frameworks: ['network-exposure'],
  },
  {
    id: 'ocs-host-001',
    name: 'Hosts should run a supported operating system',
    severity: 'high',
    description: 'A distribution past its end of life no longer gets security fixes.',
    remediation: 'Upgrade the host to a supported release of its distribution.',
    frameworks: ['patch-management'],
  },
  {
    id: 'ocs-host-002',
    name: 'The Datadog Agent should be up to date',
    severity: 'low',
    description: 'Older Agents miss security fixes and the features Cloud Security relies on (host SBOMs).',
    remediation: 'Upgrade the Agent to 7.50 or later.',
    frameworks: ['patch-management'],
  },
];

/** (workload, kind, namespace, service, failing rule ids with details) */
const WORKLOADS: [string, string, string, string, [string, string][]][] = [
  [
    'storefront-api',
    'kube_deployment',
    'storefront',
    'storefront-api',
    [
      ['ocs-k8s-006', 'containers that may gain privileges: storefront-api'],
      ['ocs-k8s-009', 'containers with a writable root file system: storefront-api'],
      ['ocs-k8s-011', 'the service account token is mounted into the pods'],
    ],
  ],
  [
    'order-service',
    'kube_deployment',
    'storefront',
    'order-service',
    [
      ['ocs-k8s-005', 'containers that may run as root: order-service'],
      ['ocs-k8s-010', 'containers without memory or CPU limits: order-service'],
    ],
  ],
  [
    'billing',
    'kube_deployment',
    'payments',
    'billing',
    [
      ['ocs-k8s-005', 'containers that may run as root: billing, migrate'],
      ['ocs-k8s-006', 'containers that may gain privileges: billing, migrate'],
    ],
  ],
  [
    'catalog-service',
    'kube_statefulset',
    'catalog',
    'catalog-service',
    [
      ['ocs-k8s-007', 'containers with a floating image tag: search-indexer'],
      ['ocs-k8s-009', 'containers with a writable root file system: catalog-service, search-indexer'],
    ],
  ],
  [
    'log-shipper',
    'kube_daemonset',
    'observability',
    'log-shipper',
    [
      ['ocs-k8s-001', 'privileged containers: shipper'],
      ['ocs-k8s-002', "shares the host's network, PID namespaces"],
      ['ocs-k8s-004', 'mounts host paths: /var/log, /var/lib/docker/containers'],
    ],
  ],
  [
    'node-debugger',
    'kube_daemonset',
    'default',
    '',
    [
      ['ocs-k8s-001', 'privileged containers: debugger'],
      ['ocs-k8s-003', 'containers adding dangerous capabilities: debugger'],
      ['ocs-k8s-012', 'runs in the default namespace'],
    ],
  ],
  [
    'nightly-report',
    'kube_cronjob',
    'payments',
    'billing',
    [
      ['ocs-k8s-007', 'containers with a floating image tag: report'],
      ['ocs-k8s-010', 'containers without memory or CPU limits: report'],
    ],
  ],
  ['api-gateway', 'kube_service', 'storefront', 'api-gateway', [['ocs-k8s-008', 'load balancer reachable from any address']]],
];

export function allMisconfigs(): MisconfigFinding[] {
  const out: MisconfigFinding[] = [];
  WORKLOADS.forEach(([name, type, ns, service, fails], i) => {
    for (const [rule, detail] of fails) {
      const r = RULES.find((x) => x.id === rule)!;
      const id = `uid-${i}-${name}`;
      const key = `misconfig:${rule}:${id}`;
      out.push({
        key,
        ruleId: rule,
        ruleName: r.name,
        severity: r.severity,
        description: r.description,
        remediation: r.remediation,
        frameworks: r.frameworks,
        detail,
        resourceId: id,
        resourceName: name,
        resourceType: type,
        cluster: 'shop-prod',
        namespace: ns,
        account: '123456789012',
        region: 'eu-central-1',
        env: 'prod',
        service: service || null,
        team: ns === 'payments' ? 'payments' : 'storefront',
        status: triage.get(key) ?? 'open',
        lastSeen: now - 120_000,
      });
    }
  });
  out.push({
    key: 'misconfig:ocs-host-002:host:i-0a1b2c3d4e5f60006',
    ruleId: 'ocs-host-002',
    ruleName: RULES[14].name,
    severity: 'low',
    description: RULES[14].description,
    remediation: RULES[14].remediation,
    frameworks: RULES[14].frameworks,
    detail: 'Datadog Agent 7.45.1 is older than 7.50',
    resourceId: 'host:i-0a1b2c3d4e5f60006',
    resourceName: 'i-0a1b2c3d4e5f60006',
    resourceType: 'host',
    cluster: null,
    namespace: null,
    account: '123456789012',
    region: 'eu-central-1',
    env: 'staging',
    service: null,
    team: 'platform',
    status: triage.get('misconfig:ocs-host-002:host:i-0a1b2c3d4e5f60006') ?? 'open',
    lastSeen: now - 60_000,
  });
  return out.sort((a, b) => RANK.indexOf(b.severity) - RANK.indexOf(a.severity) || a.ruleId.localeCompare(b.ruleId));
}

export async function misconfigurations(status: FindingStatus): Promise<MisconfigPage> {
  const all = allMisconfigs();
  const rules = RULES.map((r) => ({
    ...r,
    evaluated: r.id.startsWith('ocs-host') ? HOSTS.length : r.id === 'ocs-k8s-008' || r.id === 'ocs-k8s-013' ? 14 : 22,
    failing: all.filter((f) => f.ruleId === r.id).length,
  }));
  return delay({ findings: all.filter((f) => keep(f.status, status)), rules });
}

export async function setTriage(key: string, status: 'open' | 'muted'): Promise<void> {
  triage.set(key, status);
  return delay(undefined, 60);
}

// ---------------------------------------------------------------- inventory

export async function resources(): Promise<ResourceCategory[]> {
  const rows: ResourceCategory[] = [
    {
      category: 'Containers',
      count: 186,
      types: { kube_pod: 162, container: 24 },
      regions: ['eu-central-1'],
      envs: ['prod', 'staging'],
      teams: ['payments', 'platform', 'storefront'],
      services: ['api-gateway', 'billing', 'catalog-service', 'order-service', 'storefront-api'],
    },
    {
      category: 'Orchestration',
      count: 74,
      types: { kube_deployment: 18, kube_statefulset: 4, kube_daemonset: 5, kube_cronjob: 6, kube_job: 31, kube_namespace: 9, kube_cluster: 1 },
      regions: ['eu-central-1'],
      envs: ['prod', 'staging'],
      teams: ['payments', 'platform', 'storefront'],
      services: ['billing', 'catalog-service', 'order-service', 'storefront-api'],
    },
    { category: 'Networking', count: 16, types: { kube_service: 14, kube_ingress: 2 }, regions: ['eu-central-1'], envs: ['prod'], teams: ['storefront'], services: ['api-gateway', 'storefront-api'] },
    { category: 'Hosts', count: HOSTS.length + 4, types: { host: HOSTS.length, kube_node: 4 }, regions: ['eu-central-1', 'us-east-1'], envs: ['prod', 'staging'], teams: ['platform'], services: [] },
    {
      category: 'Storage',
      count: 9,
      types: { kube_persistent_volume_claim: 6, kube_persistent_volume: 3 },
      regions: ['eu-central-1'],
      envs: ['prod'],
      teams: ['catalog', 'payments'],
      services: ['catalog-service'],
    },
    {
      category: 'Container Images',
      count: IMAGES.length,
      types: { running: IMAGES.filter((i) => i[3]).length, 'not running': IMAGES.filter((i) => !i[3]).length },
      regions: [],
      envs: ['prod'],
      teams: [],
      services: [],
    },
  ];
  return delay(rows);
}

export async function packages(): Promise<InfraPackage[]> {
  const vulns = allVulns();
  const m = new Map<string, InfraPackage>();
  for (const e of ENTITIES) {
    for (const [eco, name, version] of e.packages) {
      const k = `${eco}|${name}|${version}`;
      const p = m.get(k) ?? { ecosystem: eco, name, version, resources: 0, running: 0, vulnerabilities: 0, maxSeverity: null, firstSeen: now - 20 * DAY };
      p.resources++;
      if (e.running) p.running++;
      m.set(k, p);
    }
  }
  for (const p of m.values()) {
    const vs = vulns.filter((v) => v.ecosystem === p.ecosystem && v.package === p.name && v.version === p.version);
    p.vulnerabilities = new Set(vs.map((v) => v.advisoryId)).size;
    p.maxSeverity = vs.map((v) => v.severity).sort((a, b) => RANK.indexOf(b) - RANK.indexOf(a))[0] ?? null;
  }
  return delay([...m.values()].sort((a, b) => a.name.localeCompare(b.name)));
}

export async function cves(q: CveQuery): Promise<CvePage> {
  const vulns = allVulns();
  const text = (q.query ?? '').trim().toLowerCase();
  const rows = ADVISORIES.map((a) => {
    const res = new Set(vulns.filter((v) => v.advisoryId === a.id).map((v) => v.resourceId));
    return {
      id: a.id,
      cve: a.cve,
      summary: a.summary,
      severity: severityOf(a.score),
      score: a.score,
      packages: [`${a.ecosystem}:${a.pkg}`],
      published: now - a.daysAgo * DAY,
      impactedLibraries: a.ecosystem === 'Maven' && a.id === 'OCSA-DEMO-2026-0004' ? 2 : 0,
      impactedResources: res.size,
      exploited: a.id === 'OCSA-DEMO-2026-0105',
    };
  })
    .filter((r) => (!text || `${r.id} ${r.cve} ${r.summary} ${r.packages.join(' ')}`.toLowerCase().includes(text)) && (!q.severity || r.severity === q.severity) && (!q.exploited || r.exploited))
    .filter((r) => (q.impact === 'impacted' ? r.impactedResources + r.impactedLibraries > 0 : q.impact === 'not_impacted' ? r.impactedResources + r.impactedLibraries === 0 : true))
    .sort((a, b) => Number(b.impactedResources + b.impactedLibraries > 0) - Number(a.impactedResources + a.impactedLibraries > 0) || (b.published ?? 0) - (a.published ?? 0));
  const offset = q.offset ?? 0;
  return delay({ total: rows.length, rows: rows.slice(offset, offset + (q.limit ?? 25)) });
}

export async function advisory(id: string): Promise<AdvisoryDetail> {
  const a = ADVISORIES.find((x) => x.id === id);
  if (!a) throw new Error('advisory not found');
  return delay({
    id: a.id,
    aliases: [a.cve],
    summary: a.summary,
    details: `${a.summary}. Versions before ${a.fixed} are affected; upgrade to ${a.fixed} or later. This is a fictional advisory of the ObserveCraft demo.`,
    severity: severityOf(a.score),
    score: a.score,
    vector: 'CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H',
    cwes: [a.cwe],
    refs: [{ type: 'ADVISORY', url: `https://advisories.example.com/${a.id}` }],
    published: now - a.daysAgo * DAY,
    modified: now - a.daysAgo * DAY,
    source: 'demo',
    packages: [{ ecosystem: a.ecosystem, package: a.pkg, ranges: [{ introduced: '0', fixed: a.fixed }], fixed: a.fixed }],
    vulnerabilities: [],
    servicesAnalyzed: 5,
    repositoriesAnalyzed: 4,
  });
}

// ---------------------------------------------------------------- summary

export async function summary(): Promise<CloudSecuritySummary> {
  const v = allVulns().filter((x) => x.status === 'open');
  const uniq = (f: (x: InfraVulnerability) => boolean) => new Set(v.filter(f).map((x) => x.advisoryId)).size;
  const high = (x: InfraVulnerability) => ['critical', 'high'].includes(x.severity);
  const bySev: Record<string, number> = { critical: 0, high: 0, medium: 0, low: 0 };
  for (const x of v) bySev[x.severity]++;
  const mis: Record<string, number> = { critical: 0, high: 0, medium: 0, low: 0 };
  for (const f of allMisconfigs().filter((x) => x.status === 'open')) mis[f.severity]++;
  return delay({
    vulnerabilities: {
      total: uniq(() => true),
      running: uniq((x) => x.running),
      exploitAvailable: uniq((x) => x.exploitAvailable),
      fixAvailable: uniq((x) => !!x.fixed),
      highOrCritical: uniq(high),
      highExploitRisk: uniq((x) => x.running && x.exploitAvailable && high(x)),
    },
    coverage: {
      hosts: HOSTS.length,
      hostsScanned: HOSTS.length - 1,
      images: IMAGES.length,
      imagesRunning: IMAGES.filter((i) => i[3]).length,
      clusters: 1,
      resourcesEvaluated: 42,
      accounts: ['aws:123456789012'],
    },
    misconfigurations: mis,
    vulnerabilitiesBySeverity: bySev,
    inventory: await resources(),
  });
}
