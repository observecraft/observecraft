// In-browser Deployment Gates and PR Gates for mock mode. State lives in memory for the session;
// validation and verdicts follow the server (routes/deployment_gates.rs, routes/pr_gates.rs). Monitors,
// span error rates per version and CI results per commit are small fictional fixtures.
import type {
  DeploymentGate,
  DeploymentGateInput,
  GateEvaluation,
  GateEvaluationRequest,
  GateMonitorState,
  GateRule,
  GateRuleResult,
  PrGateEvaluation,
  PrGateEvaluationRequest,
  PrGateRule,
  PrGateRuleInput,
  PrGateRuleResult,
} from '../deliveryGates/client';
import { PR_SOURCES, gateVerdict, monitorMatches } from '../../pages/delivery/gatesModel';

const MIN = 60_000;
const HOUR = 60 * MIN;

const fail = (message: string): never => {
  throw new Error(message);
};

interface Monitor {
  id: number;
  name: string;
  tags: string[];
  status: string;
}

/** The monitors gates look at; `status` is their status now. */
const MONITORS: Monitor[] = [
  { id: 101, name: 'High error rate on storefront-api', tags: ['service:storefront-api', 'env:prod', 'team:storefront'], status: 'ok' },
  { id: 102, name: 'p95 latency of storefront-api checkout', tags: ['service:storefront-api', 'env:prod', 'team:storefront'], status: 'ok' },
  { id: 103, name: 'order-service queue depth', tags: ['service:order-service', 'env:prod', 'team:checkout'], status: 'alert' },
  { id: 104, name: 'order-service error budget burn', tags: ['service:order-service', 'env:prod', 'team:checkout'], status: 'warn' },
  { id: 105, name: 'billing invoice failures', tags: ['service:billing', 'env:prod', 'team:billing'], status: 'ok' },
  { id: 106, name: 'catalog-service search errors', tags: ['service:catalog-service', 'env:prod', 'team:catalog'], status: 'ok' },
];

/** Versions each service ran, oldest first, and their error rates (0–1). */
const VERSIONS: Record<string, [string, number][]> = {
  'storefront-api': [
    ['2.12.3', 0.006],
    ['2.13.0', 0.007],
    ['2.14.0', 0.043],
  ],
  'order-service': [
    ['5.8.1', 0.011],
    ['5.9.0', 0.012],
  ],
  billing: [
    ['1.30.2', 0.004],
    ['1.31.0', 0.021],
  ],
  'catalog-service': [
    ['3.4.0', 0.009],
    ['3.5.0', 0.008],
  ],
};

function hash(s: string): number {
  let h = 2166136261;
  for (const c of s) h = Math.imul(h ^ c.charCodeAt(0), 16777619) >>> 0;
  return h;
}

const pct = (r: number) => `${(r * 100).toFixed(1)}%`;

function monitorResult(rule: GateRule, statuses: Map<number, string>): GateRuleResult {
  const matched = MONITORS.filter((m) => monitorMatches(rule.query ?? '', m.name, m.tags));
  const failing: GateMonitorState[] = matched.map((m) => ({ id: m.id, name: m.name, status: statuses.get(m.id) ?? m.status })).filter((m) => m.status === 'alert' || m.status === 'nodata');
  const base = { name: rule.name ?? '', type: rule.type, dryRun: rule.dryRun ?? false, matchedMonitors: matched.length, monitors: failing };
  if (!matched.length) return { ...base, status: 'passed', reason: `No monitor matches '${rule.query}'` };
  if (!failing.length) return { ...base, status: 'passed', reason: `None of the ${matched.length} matching monitors is in ALERT or NO DATA` };
  return { ...base, status: 'failed', reason: `${failing.length} of the ${matched.length} matching monitors are in ALERT or NO DATA` };
}

function faultyResult(rule: GateRule, service: string, requested: string | undefined): GateRuleResult {
  const base = { name: rule.name ?? '', type: rule.type, dryRun: rule.dryRun ?? false, monitors: [], matchedMonitors: 0 };
  const history = VERSIONS[service] ?? [];
  const index = requested ? history.findIndex(([v]) => v === requested) : history.length - 1;
  const window = rule.windowMinutes ?? 30;
  if (index < 0)
    return {
      ...base,
      status: 'passed',
      reason: requested ? `No spans of version ${requested} in the last ${window} minutes: nothing to judge yet` : `No versioned spans in the last ${window} minutes: nothing to judge yet`,
    };
  const [version, errorRate] = history[index];
  const requests = 300 + (hash(service + version) % 900);
  const known = { ...base, version, errorRate, requests };
  if (requests < (rule.minRequests ?? 50)) return { ...known, status: 'passed', reason: `Version ${version} served ${requests} requests, fewer than the ${rule.minRequests} needed to judge it` };
  const previous = history[index - 1] ?? history[index + 1];
  if (!previous) return { ...known, status: 'passed', reason: `No previous version of the service to compare ${version} with` };
  const increase = (errorRate - previous[1]) * 100;
  const allowed = rule.maxErrorRateIncrease ?? 1;
  const faulty = increase > allowed;
  return {
    ...known,
    previousVersion: previous[0],
    previousErrorRate: previous[1],
    status: faulty ? 'failed' : 'passed',
    reason: `Error rate ${pct(errorRate)} on ${version} vs ${pct(previous[1])} on ${previous[0]} (${increase >= 0 ? '+' : ''}${increase.toFixed(1)} points; ${faulty ? 'more than the' : 'within the'} +${allowed} allowed)`,
  };
}

function normalizeRules(rules: GateRule[]): GateRule[] {
  return rules.map((r, i) => {
    const query = (r.query ?? '').trim();
    const name = (r.name ?? '').trim() || (r.type === 'monitor' ? `Monitors ${query}` : 'Faulty deployment detection');
    if (r.type === 'monitor' && !query) fail(`Rule ${i + 1}: the monitor query is required, e.g. service:billing env:prod`);
    const windowMinutes = r.windowMinutes ?? 30;
    if (r.type === 'faulty_deployment' && (windowMinutes < 1 || windowMinutes > 1440)) fail(`Rule ${i + 1}: the window is 1 to 1440 minutes`);
    return { type: r.type, name, query, dryRun: r.dryRun ?? false, windowMinutes, maxErrorRateIncrease: r.maxErrorRateIncrease ?? 1, minRequests: r.minRequests ?? 50 };
  });
}

let state: { gates: DeploymentGate[]; evaluations: GateEvaluation[]; rules: PrGateRule[]; seq: number } | null = null;

function evaluate(gate: DeploymentGate, version: string | undefined, at: number, statuses: Map<number, string>, by: string): GateEvaluation {
  const rules = gate.rules.map((r) => (r.type === 'monitor' ? monitorResult(r, statuses) : faultyResult(r, gate.service, version)));
  const status = gateVerdict(gate.dryRun, rules);
  const s = db();
  return {
    id: `ev-${String(s.seq++).padStart(4, '0')}`,
    gateId: gate.id,
    service: gate.service,
    env: gate.env,
    identifier: gate.identifier,
    version: version ?? '',
    status,
    pass: status !== 'failed',
    rules,
    requestedBy: by,
    startedAt: at,
    durationMs: 180 + (hash(gate.id + at) % 1400),
  };
}

function db() {
  if (state) return state;
  const now = Date.now();
  const gate = (id: string, service: string, env: string, identifier: string, dryRun: boolean, rules: GateRule[], by: string, ago: number): DeploymentGate => ({
    id,
    service,
    env,
    identifier,
    dryRun,
    rules: normalizeRules(rules),
    createdBy: by,
    updatedBy: by,
    createdAt: now - ago,
    updatedAt: now - ago,
    lastStatus: null,
    lastEvaluatedAt: null,
  });
  state = {
    gates: [
      gate(
        'dg-0001',
        'storefront-api',
        'prod',
        'default',
        false,
        [
          { type: 'monitor', query: 'service:storefront-api env:prod' },
          { type: 'faulty_deployment', windowMinutes: 30 },
        ],
        'Alex Doe',
        9 * 24 * HOUR,
      ),
      gate('dg-0002', 'order-service', 'prod', 'pre-deploy', false, [{ type: 'monitor', name: 'Checkout monitors', query: 'service:order-service team:checkout' }], 'Sam Lee', 6 * 24 * HOUR),
      gate('dg-0003', 'billing', 'staging', 'default', true, [{ type: 'faulty_deployment', windowMinutes: 15, maxErrorRateIncrease: 0.5 }], 'Robin Park', 2 * 24 * HOUR),
    ],
    evaluations: [],
    rules: [],
    seq: 1,
  };
  const [storefront, orders, billing] = state.gates;
  // Monitor states at those times (now differs: 102 recovered, 103 is still in alert).
  const earlier = new Map([
    [102, 'alert'],
    [103, 'alert'],
  ]);
  const calm = new Map([[103, 'ok']]);
  const seeded: [DeploymentGate, string, number, Map<number, string>][] = [
    [storefront, '2.12.3', 70 * HOUR, calm],
    [orders, '5.8.1', 52 * HOUR, calm],
    [storefront, '2.13.0', 30 * HOUR, earlier],
    [storefront, '2.13.0', 29 * HOUR, calm],
    [billing, '1.31.0', 20 * HOUR, calm],
    [orders, '5.9.0', 6 * HOUR, earlier],
    [storefront, '2.14.0', 2 * HOUR, calm],
    [billing, '1.30.2', 40 * MIN, calm],
  ];
  for (const [g, version, ago, statuses] of seeded) state.evaluations.unshift(evaluate(g, version, now - ago, statuses, 'ci-deployer'));
  return state;
}

function withLast(g: DeploymentGate): DeploymentGate {
  const last = db().evaluations.find((e) => e.gateId === g.id);
  return { ...g, lastStatus: last?.status ?? null, lastEvaluatedAt: last?.startedAt ?? null };
}

function validateGate(input: DeploymentGateInput, self?: string): Pick<DeploymentGate, 'service' | 'env' | 'identifier' | 'dryRun' | 'rules'> {
  const service = input.service.trim();
  const env = input.env.trim();
  const identifier = (input.identifier ?? '').trim() || 'default';
  if (!service) fail('The service is required (at most 200 characters)');
  if (!env) fail('The environment is required (at most 200 characters)');
  if (!/^[A-Za-z0-9._-]{1,100}$/.test(identifier)) fail("Identifiers are up to 100 letters, digits, '-', '_' and '.' (e.g. pre-deploy)");
  if (!input.rules.length || input.rules.length > 20) fail('A gate has 1 to 20 rules');
  if (db().gates.some((g) => g.id !== self && g.service === service && g.env === env && g.identifier === identifier)) {
    fail(`A gate for ${service} in ${env} with the identifier '${identifier}' already exists`);
  }
  return { service, env, identifier, dryRun: input.dryRun ?? false, rules: normalizeRules(input.rules) };
}

export async function listGates(): Promise<DeploymentGate[]> {
  return [...db().gates].sort((a, b) => `${a.service} ${a.env} ${a.identifier}`.localeCompare(`${b.service} ${b.env} ${b.identifier}`)).map(withLast);
}

export async function createGate(input: DeploymentGateInput): Promise<DeploymentGate> {
  const s = db();
  const now = Date.now();
  const gate: DeploymentGate = {
    id: `dg-${String(s.seq++).padStart(4, '0')}`,
    ...validateGate(input),
    createdBy: 'You',
    updatedBy: 'You',
    createdAt: now,
    updatedAt: now,
    lastStatus: null,
    lastEvaluatedAt: null,
  };
  s.gates.push(gate);
  return withLast(gate);
}

export async function updateGate(id: string, input: DeploymentGateInput): Promise<DeploymentGate> {
  const s = db();
  const i = s.gates.findIndex((g) => g.id === id);
  if (i < 0) fail('Deployment gate not found');
  s.gates[i] = { ...s.gates[i], ...validateGate(input, id), updatedBy: 'You', updatedAt: Date.now() };
  return withLast(s.gates[i]);
}

export async function deleteGate(id: string): Promise<void> {
  const s = db();
  s.gates = s.gates.filter((g) => g.id !== id);
  s.evaluations = s.evaluations.map((e) => (e.gateId === id ? { ...e, gateId: null } : e));
}

export async function evaluateGate(req: GateEvaluationRequest): Promise<GateEvaluation> {
  const identifier = req.identifier?.trim() || 'default';
  const gate = db().gates.find((g) => g.service === req.service.trim() && g.env === req.env.trim() && g.identifier === identifier);
  if (!gate) fail(`No deployment gate for ${req.service} in ${req.env} with the identifier '${identifier}'`);
  const ev = evaluate(gate as DeploymentGate, req.version?.trim() || undefined, Date.now(), new Map(), 'You');
  db().evaluations.unshift(ev);
  return ev;
}

export async function listEvaluations(): Promise<GateEvaluation[]> {
  return db().evaluations.map((e) => ({ ...e, rules: e.rules.map((r) => ({ ...r, monitors: r.monitors?.map((m) => ({ ...m })) })) }));
}

export async function getEvaluation(id: string): Promise<GateEvaluation> {
  const e = db().evaluations.find((x) => x.id === id) ?? fail('Evaluation not found');
  return { ...e, rules: e.rules.map((r) => ({ ...r, monitors: (r.monitors ?? []).map((m) => ({ ...m, currentStatus: MONITORS.find((x) => x.id === m.id)?.status ?? null })) })) };
}

// ================================================================ PR gates

function validateRule(input: PrGateRuleInput, self?: string): PrGateRuleInput {
  const name = input.name.trim();
  if (!name || name.length > 200) fail('The rule name is required (at most 200 characters)');
  if (db().rules.some((r) => r.id !== self && r.name.toLowerCase() === name.toLowerCase())) fail(`A rule named '${name}' already exists`);
  const threshold = input.threshold ?? 0;
  if (threshold < 0 || threshold > 100_000) fail('The threshold is 0 to 100,000');
  const repositories = [...new Set((input.repositories ?? []).map((r) => r.trim()).filter(Boolean))];
  return { ...input, name, threshold, repositories, minSeverity: input.minSeverity ?? 'high', blocking: input.blocking ?? true, enabled: input.enabled ?? true };
}

export async function listPrRules(): Promise<PrGateRule[]> {
  return [...db().rules].sort((a, b) => a.name.toLowerCase().localeCompare(b.name.toLowerCase()));
}

export async function createPrRule(input: PrGateRuleInput): Promise<PrGateRule> {
  const s = db();
  const v = validateRule(input);
  const now = Date.now();
  const rule: PrGateRule = {
    id: `pr-${String(s.seq++).padStart(4, '0')}`,
    name: v.name,
    source: v.source,
    repositories: v.repositories ?? [],
    threshold: v.threshold ?? 0,
    minSeverity: v.minSeverity ?? 'high',
    blocking: v.blocking ?? true,
    enabled: v.enabled ?? true,
    evaluable: PR_SOURCES[v.source].evaluable,
    createdBy: 'You',
    updatedBy: 'You',
    createdAt: now,
    updatedAt: now,
  };
  s.rules.push(rule);
  return rule;
}

export async function updatePrRule(id: string, input: PrGateRuleInput): Promise<PrGateRule> {
  const s = db();
  const i = s.rules.findIndex((r) => r.id === id);
  if (i < 0) fail('PR gate rule not found');
  const v = validateRule(input, id);
  s.rules[i] = {
    ...s.rules[i],
    name: v.name,
    source: v.source,
    repositories: v.repositories ?? [],
    threshold: v.threshold ?? 0,
    minSeverity: v.minSeverity ?? 'high',
    blocking: v.blocking ?? true,
    enabled: v.enabled ?? true,
    evaluable: PR_SOURCES[v.source].evaluable,
    updatedBy: 'You',
    updatedAt: Date.now(),
  };
  return s.rules[i];
}

export async function deletePrRule(id: string): Promise<void> {
  const s = db();
  s.rules = s.rules.filter((r) => r.id !== id);
}

const normalizeRepo = (r: string) =>
  r
    .trim()
    .toLowerCase()
    .replace(/\/+$/, '')
    .replace(/\.git$/, '')
    .replace(/^[a-z]+:\/\/[^/]+\//, '')
    .replace(/^[^@/]+@[^:]+:/, '');

const KNOWN_REPOS = ['example-org/storefront-api', 'example-org/order-service', 'example-org/billing', 'example-org/catalog-service', 'example-org/inventory-service', 'example-org/web-frontend'];

export async function evaluatePr(req: PrGateEvaluationRequest): Promise<PrGateEvaluation> {
  const repository = normalizeRepo(req.repository);
  const sha = req.sha.trim().toLowerCase();
  if (!repository) fail('repository is required, e.g. example-org/billing');
  if (!/^[0-9a-f]{7,64}$/.test(sha)) fail("sha is the commit's hexadecimal hash");
  const h = hash(repository + sha);
  const ci = KNOWN_REPOS.includes(repository)
    ? { pipelines: 1 + (h % 3), failedPipelines: h % 4 === 0 ? 1 : 0, tests: 80 + (h % 120), failedTests: h % 4 === 0 ? 1 + (h % 2) : 0, flakyTests: (h >> 3) % 3 === 0 ? 1 : 0 }
    : { pipelines: 0, failedPipelines: 0, tests: 0, failedTests: 0, flakyTests: 0 };
  const rules = db().rules.filter((r) => r.enabled && (!r.repositories.length || r.repositories.some((x) => normalizeRepo(x) === repository)));
  const results: PrGateRuleResult[] = rules.map((r) => {
    const base = { ruleId: r.id, name: r.name, source: r.source, blocking: r.blocking, threshold: r.threshold };
    const meta = PR_SOURCES[r.source];
    if (!meta.evaluable) return { ...base, status: 'not_evaluated', count: null, reason: `Needs Code Security to count ${meta.finding}: not evaluated` };
    if (!ci.pipelines && !ci.tests) return { ...base, status: 'no_data', count: null, reason: 'No CI run of this commit is known yet' };
    const count = r.source === 'pipeline_failures' ? ci.failedPipelines : r.source === 'test_failures' ? ci.failedTests : ci.flakyTests;
    const failed = count > r.threshold;
    return { ...base, status: failed ? 'failed' : 'passed', count, reason: `${count} ${meta.finding} on this commit (${failed ? 'more than the' : 'within the'} ${r.threshold} allowed)` };
  });
  const status = results.some((r) => r.blocking && r.status === 'failed') ? 'failed' : 'passed';
  return { repository, sha, status, pass: status === 'passed', rules: results, ci };
}
