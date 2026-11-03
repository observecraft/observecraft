// In-browser Assist runs for mock mode: investigations, security verdicts, agent runs and coding
// sessions that progress over a few seconds like the server's (routes/assist_runs.rs): queued, then
// their steps one by one, then the conclusion. Everything lives in memory for the session.
import type {
  AgentDraft,
  AgentInput,
  AgentView,
  AnalyzedSignal,
  CodeChange,
  CodeConnections,
  CodeSettings,
  CodeUsage,
  Conclusion,
  CreditSettings,
  CreditsView,
  InvestigationMonitor,
  Memory,
  NewMemory,
  NewRepo,
  PullRequest,
  RepoView,
  RunDetail,
  RunKind,
  RunReport,
  RunReportRequest,
  RunSearch,
  RunSettings,
  RunStep,
  RunSummary,
  StartRun,
  UsageReport,
} from '../assist/types';
import type { TimeRange } from '../types';
import { searchMonitors } from './monitors';
import { signalsBetween } from './security';

const DAY = 86_400_000;
const QUEUED_MS = 700;
const STEP_MS = 900;
const ME = 'Alex Morgan';

const uuid = () => (globalThis.crypto?.randomUUID ? globalThis.crypto.randomUUID() : `id-${Math.random().toString(36).slice(2)}-${Date.now()}`);

interface Script {
  steps: RunStep[];
  conclusion: Conclusion;
  change?: CodeChange;
}

interface MockRun {
  summary: Omit<RunSummary, 'status' | 'startedAt' | 'finishedAt' | 'verdict' | 'rootCause' | 'error'>;
  /** Seeded runs took this long (live ones play their steps). */
  tookMs?: number;
  script: Script;
  cancelledAt?: number;
  feedback?: string;
  pullRequest?: Record<string, unknown>;
}

const tool = (name: string, input: Record<string, unknown>, summary: string): RunStep => ({
  type: 'tool',
  id: uuid(),
  name,
  input: input as RunStep extends { input: infer I } ? I : never,
  status: 'ok',
  summary,
});
const text = (t: string): RunStep => ({ type: 'text', text: t });

function investigationScript(title: string, monitorId?: number | null): Script {
  const service = /billing/i.test(title) ? 'billing' : /inventory/i.test(title) ? 'inventory-service' : /storefront|checkout/i.test(title) ? 'storefront-api' : 'order-service';
  return {
    steps: [
      text(`Looking at what changed around the alert on **${service}**.`),
      ...(monitorId ? [tool('get_monitor', { id: monitorId }, `{"name":"${title}","status":"alert"}`)] : []),
      tool('search_logs', { query: `service:${service} status:error`, from: '1h' }, '{"total":612,"logs":[{"message":"TimeoutError: inventory-service did not answer within 2s"}]}'),
      tool('list_services', { from: '1h' }, `[{"service":"${service}","errorRate":0.062,"p95":2100}]`),
      tool('query_metrics', { queries: [`avg:trace.http.request.duration{service:${service}}`], from: '4h' }, '[{"series":"p95","last":2.1,"min":0.18,"max":2.4}]'),
      tool('search_traces', { query: `service:${service} status:error`, from: '1h' }, '{"traces":[{"resource":"POST /api/orders","duration":2100}]}'),
      text('The errors started within a minute of a deploy of **inventory-service 3.2.1**; its spans show connection pool exhaustion.'),
    ],
    conclusion: {
      summary: `**${service}** started failing at 09:12 UTC: calls to **inventory-service** time out after 2 s since its deploy of version 3.2.1 at 09:11, which reduced its database connection pool from 50 to 10.`,
      verdict: 'conclusive',
      confidence: 'high',
      rootCause: 'inventory-service 3.2.1 shrank its database connection pool (50 → 10)',
      evidence: [
        {
          title: '612 TimeoutError logs on ' + service,
          detail: 'All name inventory-service as the callee, from 09:12 UTC.',
          link: `/logs?query=${encodeURIComponent(`service:${service} status:error`)}`,
        },
        { title: 'p95 latency 180 ms → 2.1 s', detail: 'POST /api/orders, aligned with the deploy.', link: `/apm/services/${service}` },
        { title: 'inventory-service pool saturated', detail: 'db.pool.waiting at 40+ since 09:11.', link: '/metrics/explorer' },
      ],
      nextSteps: ['Roll back inventory-service to 3.2.0, or raise DB_POOL_SIZE back to 50', 'Add a monitor on db.pool.waiting for inventory-service'],
    },
  };
}

function securityScript(title: string, severity: string, signalId: string): Script {
  const malicious = /miner|malware|exfil|reverse shell|ransom/i.test(title);
  const suspicious = !malicious && (severity === 'critical' || /root|admin|public|brute|without mfa/i.test(title));
  const steps = [
    tool('get_security_signal', { id: signalId }, `{"title":"${title.replace(/"/g, "'")}","severity":"${severity}"}`),
    tool('search_logs', { query: malicious ? 'source:runtime-security-agent' : 'source:okta @evt.outcome:FAILURE', from: '1d' }, malicious ? '{"total":38}' : '{"total":14}'),
    tool('search_security_signals', { query: title.split(' ').slice(0, 3).join(' '), from: '7d' }, '{"signals":3}'),
  ];
  if (malicious)
    return {
      steps: [...steps, text('The process connects to a known mining pool and was started from a container that never ran it before.')],
      conclusion: {
        summary:
          'A **xmrig** process started in the `billing-worker` container at 08:41 UTC, connects to a known mining pool (**pool.example-mining.net:3333**) and uses 3.9 of 4 CPUs. Nothing in the image or the deploy history explains it: treat the host as compromised.',
        verdict: 'malicious',
        confidence: 'high',
        rootCause: 'Unexpected cryptominer in the billing-worker container',
        evidence: [
          { title: 'xmrig executed from /tmp/.x', detail: 'Parent: sh, started by the web user.', link: '/logs?query=source%3Aruntime-security-agent' },
          { title: 'Mining pool DNS lookups', detail: 'pool.example-mining.net resolved 214 times.' },
          { title: 'CPU at 98% since 08:41', link: '/infrastructure' },
        ],
        nextSteps: ['Isolate the host and rotate the credentials the container held', 'Declare a security incident', 'Rebuild the image from a clean base'],
      },
    };
  return {
    steps: [
      ...steps,
      text(suspicious ? 'The activity comes from an address never seen for this account, outside working hours.' : 'Every attempt comes from the corporate VPN range used daily by this account.'),
    ],
    conclusion: suspicious
      ? {
          summary:
            'The activity comes from **203.0.113.45**, an address never used by this account before, at 03:12 local time, right after 14 failed attempts. Treat it as a likely account takeover attempt.',
          verdict: 'suspicious',
          confidence: 'medium',
          rootCause: 'Repeated failures then success from an unknown address',
          evidence: [
            { title: '14 failed attempts in 6 minutes', detail: 'Then one success.', link: '/logs?query=%40evt.outcome%3AFAILURE' },
            { title: 'First seen address', detail: '203.0.113.45 has no earlier logins in 30 days.' },
          ],
          nextSteps: ['Suspend the session and reset the account’s credentials', 'Declare a security incident if the account reached production'],
        }
      : {
          summary: 'The activity follows a timeout on the user’s phone and comes from the corporate VPN (**198.51.100.0/24**), used by 1,240 accounts daily. Benign.',
          verdict: 'benign',
          confidence: 'high',
          evidence: [
            { title: 'Corporate VPN range', detail: '198.51.100.0/24: 59,862 logins in 30 days across the org.' },
            { title: 'Same device and browser as usual', detail: 'Chrome 141 on macOS, seen daily.' },
          ],
          nextSteps: ['Archive the signal as benign'],
        },
  };
}

function agentScript(name: string, skills: string[]): Script {
  const steps: RunStep[] = [text(`Running **${name}**.`)];
  if (skills.includes('errors')) steps.push(tool('search_error_issues', { from: '1d' }, '{"issues":[{"title":"TimeoutError in order-service","firstSeen":"today"}]}'));
  if (skills.includes('logs')) steps.push(tool('search_logs', { query: 'status:error', from: '1d' }, '{"total":1284}'));
  if (skills.includes('monitors')) steps.push(tool('search_monitors', { statuses: ['alert'] }, '{"monitors":[{"name":"High error rate on order-service"}]}'));
  if (skills.includes('apm')) steps.push(tool('list_services', { from: '1d' }, '[{"service":"order-service","errorRate":0.062}]'));
  return {
    steps,
    conclusion: {
      summary: '2 new error issues since yesterday. **TimeoutError in order-service** (612 events) needs attention; **ValueError: invalid coupon code** in storefront-api is user input and can wait.',
      verdict: 'needs_attention',
      confidence: 'high',
      evidence: [{ title: 'TimeoutError in order-service', detail: 'First seen today, 612 events.', link: '/errors' }],
      nextSteps: ['Assign the order-service issue to @team-orders'],
    },
  };
}

const CART_BEFORE = `class CartsController < ApplicationController
  # Finds the cart in progress for this session
  def find_cart_in_progress
    customer = Customer.includes(:carts).find_by(session_id: cookies[:customer_session_id])
    customer.carts.each do |cart|
      return cart if cart.checked_out == false
    end
    nil
  end
end
`;
const CART_AFTER = `class CartsController < ApplicationController
  # Finds the cart in progress for this session
  def find_cart_in_progress
    customer = Customer.find_by(session_id: cookies[:customer_session_id])
    return nil if customer.nil?

    Cart.where(customer_id: customer.id, checked_out: false).limit(1).first
  end
end
`;

function codeScript(repo: string, prompt: string): Script {
  return {
    steps: [
      text(`Looking for the code behind: ${prompt}`),
      tool('search_repo_code', { repo, query: 'find_cart_in_progress' }, '{"matches":[{"path":"app/controllers/carts_controller.rb"}]}'),
      tool('read_repo_file', { repo, path: 'app/controllers/carts_controller.rb' }, '{"path":"app/controllers/carts_controller.rb","content":"class CartsController …"}'),
      tool('search_logs', { query: 'service:storefront-api @db.statement:*carts*', from: '1d' }, '{"total":4120}'),
      tool('propose_change', { title: 'Query the open cart instead of loading every cart' }, 'Proposed. Now conclude.'),
    ],
    conclusion: {
      summary: '`find_cart_in_progress` loads every cart of the customer and filters them in Ruby, a collection scan on each checkout. The change queries the open cart directly.',
      verdict: 'change_proposed',
      confidence: 'high',
      rootCause: 'Cart lookup loads all carts with includes(:carts)',
      evidence: [{ title: '4,120 slow cart queries in a day', link: '/logs?query=service%3Astorefront-api' }],
      nextSteps: ['Review and open the pull request', 'Add an index on carts(customer_id, checked_out)'],
    },
    change: {
      title: 'Query the open cart instead of loading every cart',
      description: '`find_cart_in_progress` loaded every cart with `includes(:carts)` and filtered in Ruby. It now asks the database for the one open cart.',
      files: [{ path: 'app/controllers/carts_controller.rb', before: CART_BEFORE, after: CART_AFTER }],
    },
  };
}

// ---------------------------------------------------------------- state

let runs: MockRun[] | null = null;
let settings: RunSettings = { knowledge: '', runsPerHour: 30, automaticRunsPerDay: 100, notify: [], securityAutoSeverities: [], securityRunAs: null };
let memories: Memory[] = [];
let auto = new Set<number>();
let agents: AgentView[] | null = null;
let repos: RepoView[] = [{ repo: 'example-org/storefront', provider: 'github', services: ['storefront-api', 'order-service'], addedBy: ME, addedAt: Date.now() - 20 * DAY }];
let codeSettings: CodeSettings = { branchPrefix: 'assist/', draft: true, autoPullRequest: false };
let credits: CreditSettings = { monthlyLimit: 50_000, userDailyLimit: 2_000, inputPricePerMtok: 5, outputPricePerMtok: 25 };

function seed(kind: RunKind, title: string, subject: Record<string, unknown>, script: Script, ago: number, trigger = 'manual'): MockRun {
  return {
    summary: {
      id: uuid(),
      kind,
      title,
      trigger,
      subject: subject as RunSummary['subject'],
      createdBy: trigger === 'manual' ? ME : trigger === 'monitor' ? 'Auto-investigate' : 'Schedule',
      createdAt: Date.now() - ago,
      feedback: null,
      inputTokens: 18_000 + Math.round(ago % 9_000),
      outputTokens: 1_400,
    },
    script,
  };
}

function db(): MockRun[] {
  if (runs) return runs;
  runs = [
    seed(
      'investigation',
      'High error rate on order-service',
      { monitorId: 1, groupKey: 'service:order-service', value: 42, alertAt: Date.now() - 2 * 3_600_000 },
      investigationScript('High error rate on order-service', 1),
      2 * 3_600_000,
      'monitor',
    ),
    seed('investigation', 'Why did checkout latency spike this morning?', { question: 'Why did checkout latency spike this morning?' }, investigationScript('checkout latency'), 26 * 3_600_000),
    seed('investigation', 'Billing payment failures', { monitorId: 2 }, investigationScript('Billing payment failures', 2), 4 * DAY, 'monitor'),
    seed(
      'code',
      'Cart lookups are slow in checkout',
      { repo: 'example-org/storefront', prompt: 'Cart lookups are slow in checkout' },
      codeScript('example-org/storefront', 'Cart lookups are slow in checkout'),
      3 * DAY,
    ),
  ];
  runs[3].pullRequest = { url: 'https://github.com/example-org/storefront/pull/4129', number: 4129, branch: 'assist/query-the-open-cart-8c6f0a1b', draft: true, state: 'merged' };
  return runs;
}

function view(r: MockRun, now = Date.now()): RunDetail {
  const elapsed = now - r.summary.createdAt;
  const steps = r.script.steps;
  const doneAt = r.summary.createdAt + (r.tookMs ?? QUEUED_MS + steps.length * STEP_MS);
  const cancelled = r.cancelledAt != null && r.cancelledAt < doneAt;
  const at = cancelled ? r.cancelledAt! : now;
  const shown = Math.max(0, Math.min(steps.length, Math.floor((at - r.summary.createdAt - QUEUED_MS) / STEP_MS) + 1));
  const status = cancelled ? 'cancelled' : elapsed < QUEUED_MS ? 'queued' : now < doneAt ? 'running' : 'concluded';
  const visible =
    elapsed < QUEUED_MS
      ? []
      : steps
          .slice(0, status === 'concluded' ? steps.length : shown)
          .map((s, i, all) => (status === 'running' && i === all.length - 1 && s.type === 'tool' ? { ...s, status: 'running', summary: '' } : s));
  const concluded = status === 'concluded';
  return {
    ...r.summary,
    status,
    startedAt: elapsed < QUEUED_MS ? null : r.summary.createdAt + QUEUED_MS,
    finishedAt: concluded ? doneAt : cancelled ? r.cancelledAt! : null,
    verdict: concluded ? r.script.conclusion.verdict : null,
    rootCause: concluded ? (r.script.conclusion.rootCause ?? null) : null,
    error: null,
    feedback: r.feedback ?? null,
    steps: visible,
    conclusion: concluded ? r.script.conclusion : null,
    change: concluded ? (r.script.change ?? null) : null,
    pullRequest: (r.pullRequest as RunDetail['pullRequest']) ?? null,
  };
}

const summaryOf = (r: MockRun): RunSummary => {
  const { steps: _s, conclusion: _c, change: _ch, pullRequest: _p, ...rest } = view(r);
  return rest;
};

function find(id: string): MockRun {
  const r = db().find((x) => x.summary.id === id);
  if (!r) throw new Error('No such run');
  return r;
}

function checkLimits() {
  const hour = db().filter((r) => r.summary.createdAt > Date.now() - 3_600_000).length;
  if (hour >= settings.runsPerHour) throw new Error(`This organization started ${hour} Assist runs in the last hour, its limit (Investigation → Settings → Rate Limits)`);
}

function add(kind: RunKind, title: string, subject: Record<string, unknown>, script: Script, trigger = 'manual'): RunSummary {
  checkLimits();
  const r = seed(kind, title, subject, script, 0, trigger);
  r.summary.createdAt = Date.now();
  db().unshift(r);
  return summaryOf(r);
}

// ---------------------------------------------------------------- runs

function monitorsList() {
  return searchMonitors({ creators: [], drafts: false, limit: 500, muted: null, offset: 0, priorities: [], query: '', recipients: [], sort: 'name', statuses: [], teams: [], types: [] } as never)
    .monitors;
}

export async function startRun(r: StartRun): Promise<RunSummary> {
  if (r.kind === 'investigation' && r.monitorId != null) {
    const m = monitorsList().find((x) => x.id === r.monitorId);
    if (!m) throw new Error('No such monitor');
    return add('investigation', m.name, { monitorId: m.id }, investigationScript(m.name, m.id));
  }
  if (r.kind === 'investigation' && r.question?.trim()) {
    const q = r.question.trim();
    return add('investigation', q.split('\n')[0].slice(0, 100), { question: q }, investigationScript(q));
  }
  if (r.kind === 'security' && r.signalId) {
    const s = signalsBetween(Date.now() - 14 * DAY, Date.now()).find((x) => x.id === r.signalId);
    if (!s) throw new Error(`no signal ${r.signalId}`);
    return add('security', s.title, { signalId: s.id }, securityScript(s.title, s.severity, s.id));
  }
  throw new Error('Give a monitor, a question (up to 4000 characters) or a signal');
}

export async function searchRuns(q: RunSearch): Promise<RunSummary[]> {
  return db()
    .filter((r) => r.summary.kind === q.kind && (q.monitorId == null || (r.summary.subject as { monitorId?: number }).monitorId === q.monitorId))
    .map(summaryOf)
    .filter((r) => !q.status || r.status === q.status)
    .slice(0, q.limit ?? 100);
}

export async function getRun(id: string): Promise<RunDetail> {
  return view(find(id));
}

export async function cancelRun(id: string): Promise<void> {
  const r = find(id);
  if (['queued', 'running'].includes(view(r).status)) r.cancelledAt = Date.now();
}

export async function runFeedback(id: string, feedback: 'helpful' | 'not_helpful', remember?: string): Promise<void> {
  const r = find(id);
  r.feedback = feedback;
  if (remember?.trim()) await addMemory({ monitorId: (r.summary.subject as { monitorId?: number }).monitorId ?? null, text: remember.trim() });
}

function bucketed(items: number[], from: number, to: number) {
  const step = to - from <= 2 * DAY ? 3_600_000 : DAY;
  const out = new Map<number, number>();
  for (const at of items) {
    const b = Math.floor(at / step) * step;
    out.set(b, (out.get(b) ?? 0) + 1);
  }
  return [...out.entries()].sort((a, b) => a[0] - b[0]).map(([at, count]) => ({ at, count }));
}

const counted = (names: string[]) => {
  const m = new Map<string, number>();
  for (const n of names) m.set(n, (m.get(n) ?? 0) + 1);
  return [...m.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).map(([name, count]) => ({ name, count }));
};

export async function runReport(req: RunReportRequest): Promise<RunReport> {
  const rs = db()
    .map((r) => view(r))
    .filter((r) => r.kind === req.kind && r.createdAt >= req.range.from && r.createdAt < req.range.to && (req.monitorId == null || (r.subject as { monitorId?: number }).monitorId === req.monitorId));
  const concluded = rs.filter((r) => r.status === 'concluded');
  return {
    total: rs.length,
    concluded: concluded.length,
    failed: rs.filter((r) => r.status === 'failed').length,
    meanSecondsToConclusion: concluded.length ? concluded.reduce((a, r) => a + ((r.finishedAt ?? r.createdAt) - r.createdAt) / 1000, 0) / concluded.length + 140 : null,
    helpful: rs.filter((r) => r.feedback === 'helpful').length,
    notHelpful: rs.filter((r) => r.feedback === 'not_helpful').length,
    overTime: bucketed(
      rs.map((r) => r.createdAt),
      req.range.from,
      req.range.to,
    ),
    byTrigger: counted(rs.map((r) => r.trigger)),
    byVerdict: counted(rs.map((r) => r.verdict ?? r.status)),
    byMonitor: counted(rs.filter((r) => (r.subject as { monitorId?: number }).monitorId != null).map((r) => r.title)),
  };
}

// ---------------------------------------------------------------- investigation settings

export async function getRunSettings(): Promise<RunSettings> {
  return structuredClone(settings);
}

export async function putRunSettings(s: RunSettings): Promise<void> {
  if (s.knowledge.length > 20_000) throw new Error('The knowledge base is limited to 20000 characters');
  if (s.runsPerHour < 1 || s.runsPerHour > 1000) throw new Error('Runs per hour is 1 to 1000; automatic runs per day 0 to 10000');
  settings = { ...s, notify: s.notify.map((h) => h.trim().replace(/^@/, '')).filter(Boolean), securityRunAs: s.securityAutoSeverities.length ? 'me' : null };
}

export async function listMemories(): Promise<Memory[]> {
  return memories.map((m) => ({ ...m, monitorName: m.monitorId != null ? (monitorsList().find((x) => x.id === m.monitorId)?.name ?? null) : null }));
}

export async function addMemory(m: NewMemory): Promise<void> {
  if (!m.text.trim()) throw new Error('Write what to remember');
  memories.unshift({ id: uuid(), monitorId: m.monitorId ?? null, monitorName: null, text: m.text.trim(), sourceRun: null, createdBy: ME, createdAt: Date.now() });
}

export async function deleteMemory(id: string): Promise<void> {
  memories = memories.filter((m) => m.id !== id);
}

export async function investigationMonitors(): Promise<InvestigationMonitor[]> {
  const now = Date.now();
  return monitorsList().map((m, i) => ({
    id: m.id,
    name: m.name,
    type: m.type,
    status: m.status,
    priority: m.priority ?? null,
    tags: m.tags,
    teams: m.teams,
    supported: m.type !== 'synthetics',
    autoInvestigate: auto.has(m.id),
    investigations: db().filter((r) => r.summary.kind === 'investigation' && (r.summary.subject as { monitorId?: number }).monitorId === m.id).length,
    memories: memories.filter((x) => x.monitorId === m.id).length,
    recentAlerts: m.status === 'alert' || i % 4 === 0 ? [{ at: now - (i + 1) * 47 * 60_000, group: m.tags.find((t) => t.startsWith('service:')) ?? '*', value: 12 + i }] : [],
  }));
}

export async function setMonitorAuto(id: number, on: boolean): Promise<void> {
  if (on) auto.add(id);
  else auto.delete(id);
}

// ---------------------------------------------------------------- security analyst

export async function analyzedSignals(range: TimeRange): Promise<AnalyzedSignal[]> {
  const signals = signalsBetween(range.from, range.to)
    .sort((a, b) => b.createdAt - a.createdAt)
    .slice(0, 200);
  // Seeded verdicts on the newest high and critical signals.
  for (const s of signals.filter((x) => x.severity === 'critical' || x.severity === 'high').slice(0, 4)) {
    if (!db().some((r) => r.summary.kind === 'security' && (r.summary.subject as { signalId?: string }).signalId === s.id)) {
      db().push(seed('security', s.title, { signalId: s.id }, securityScript(s.title, s.severity, s.id), Math.max(60_000, Date.now() - s.createdAt - 60_000), 'signal'));
    }
  }
  return signals.map((s) => {
    const run = db().find((r) => r.summary.kind === 'security' && (r.summary.subject as { signalId?: string }).signalId === s.id);
    return { id: s.id, title: s.title, severity: s.severity, state: s.state, ruleName: s.ruleName, createdAt: s.createdAt, run: run ? summaryOf(run) : null };
  });
}

// ---------------------------------------------------------------- agents

function agentsDb(): AgentView[] {
  if (agents) return agents;
  const now = Date.now();
  const base = { allowActions: false, timezone: 'UTC', enabled: true, createdBy: ME, kind: 'agent', repo: null, lastRunAt: null, lastRunStatus: null, lastVerdict: null, runs: 0 };
  agents = [
    {
      ...base,
      id: uuid(),
      name: 'Daily Error Triager',
      description: 'Finds new error issues every morning and says which need attention.',
      instructions: 'List error issues first seen in the last day. For each, say whether it needs attention (customer impact, growing) or can wait, and who owns the service.',
      skills: ['errors', 'logs', 'apm'],
      tags: ['team:orders'],
      schedule: '0 9 * * 1-5',
      nextRunAt: now + 14 * 3_600_000,
      createdAt: now - 9 * DAY,
      updatedAt: now - 2 * DAY,
      runs: 7,
      lastRunAt: now - 10 * 3_600_000,
      lastRunStatus: 'concluded',
      lastVerdict: 'needs_attention',
    },
    {
      ...base,
      id: uuid(),
      name: 'Log Analyzer',
      description: 'Finds root causes in logs without manual digging.',
      instructions: 'Given a service, find the most frequent error patterns in its logs over the last 4 hours and group them by cause.',
      skills: ['logs'],
      tags: ['team:platform'],
      schedule: null,
      nextRunAt: null,
      createdAt: now - 5 * DAY,
      updatedAt: now - 5 * DAY,
    },
  ];
  return agents;
}

function agentRuns(id: string) {
  return db().filter((r) => (r.summary.subject as { agentId?: string }).agentId === id);
}

function withRuns(a: AgentView): AgentView {
  const rs = agentRuns(a.id).map((r) => view(r));
  if (!rs.length) return a;
  return { ...a, runs: a.runs + rs.length, lastRunAt: rs[0].createdAt, lastRunStatus: rs[0].status, lastVerdict: rs[0].verdict };
}

function checkAgent(a: AgentInput) {
  if (!a.name.trim() || a.name.length > 100) throw new Error('The name is 1 to 100 characters');
  if (!a.instructions.trim()) throw new Error('The instructions are 1 to 8000 characters');
  if (a.schedule && a.schedule.trim().split(/\s+/).length !== 5) throw new Error('Schedule: a cron has five fields (minute hour day month weekday)');
  if (a.kind === 'code' && !a.repo) throw new Error('A code automation needs a repository');
}

const nextRun = (schedule?: string | null) => (schedule?.trim() ? Date.now() + 6 * 3_600_000 : null);

export async function listAgents(): Promise<AgentView[]> {
  return agentsDb().map(withRuns);
}

export async function getAgent(id: string): Promise<AgentView> {
  const a = agentsDb().find((x) => x.id === id);
  if (!a) throw new Error('No such agent');
  return withRuns(a);
}

export async function createAgent(a: AgentInput): Promise<AgentView> {
  checkAgent(a);
  const now = Date.now();
  const v: AgentView = {
    id: uuid(),
    kind: a.kind ?? 'agent',
    repo: a.repo ?? null,
    name: a.name.trim(),
    description: a.description?.trim() ?? '',
    instructions: a.instructions.trim(),
    skills: a.skills,
    allowActions: a.allowActions ?? false,
    tags: a.tags ?? [],
    schedule: a.schedule?.trim() || null,
    timezone: a.timezone || 'UTC',
    enabled: a.enabled ?? true,
    nextRunAt: nextRun(a.schedule),
    createdBy: ME,
    createdAt: now,
    updatedAt: now,
    runs: 0,
    lastRunAt: null,
    lastRunStatus: null,
    lastVerdict: null,
  };
  agentsDb().unshift(v);
  return v;
}

export async function updateAgent(id: string, a: AgentInput): Promise<AgentView> {
  checkAgent(a);
  const cur = await getAgent(id);
  const v = {
    ...cur,
    ...a,
    description: a.description ?? '',
    schedule: a.schedule?.trim() || null,
    nextRunAt: nextRun(a.schedule),
    updatedAt: Date.now(),
    repo: a.repo ?? cur.repo,
    kind: cur.kind,
  } as AgentView;
  agents = agentsDb().map((x) => (x.id === id ? { ...v, runs: x.runs, lastRunAt: x.lastRunAt, lastRunStatus: x.lastRunStatus, lastVerdict: x.lastVerdict } : x));
  return withRuns(agents.find((x) => x.id === id)!);
}

export async function deleteAgent(id: string): Promise<void> {
  agents = agentsDb().filter((x) => x.id !== id);
}

export async function runAgent(id: string, input?: string): Promise<RunSummary> {
  const a = await getAgent(id);
  if (a.kind === 'code' && a.repo) return add('code', a.name, { agentId: id, repo: a.repo, prompt: a.instructions, input: input ?? '' }, codeScript(a.repo, a.instructions));
  return add('agent', a.name, { agentId: id, input: input ?? '' }, agentScript(a.name, a.skills));
}

export async function draftAgent(description: string): Promise<AgentDraft> {
  const d = description.toLowerCase();
  if (!d.trim()) throw new Error('Describe the agent in 1 to 4000 characters');
  const skills = ['logs', 'metrics', 'apm', 'monitors', 'incidents', 'errors', 'kubernetes', 'security'].filter(
    (s) => d.includes(s.replace(/s$/, '')) || (s === 'errors' && /error|exception/.test(d)) || (s === 'apm' && /latency|service|trace/.test(d)),
  );
  const daily = /daily|every (day|morning)|each morning/.test(d);
  const weekly = /weekly|every week|monday/.test(d);
  const topic = /secur|signal|threat/.test(d)
    ? 'Security'
    : /error|exception/.test(d)
      ? 'Error'
      : /latency|slow|performance/.test(d)
        ? 'Performance'
        : /incident/.test(d)
          ? 'Incident'
          : /log/.test(d)
            ? 'Log'
            : 'Ops';
  const role = /incident/.test(d) ? 'Responder' : /report|summar|dashboard/.test(d) ? 'Reporter' : /triage|new|which/.test(d) ? 'Triager' : 'Analyzer';
  return {
    name: `${daily ? 'Daily ' : weekly ? 'Weekly ' : ''}${topic} ${role}`,
    description: description.trim().split('\n')[0].slice(0, 140),
    instructions: `On each run:\n1. ${description.trim()}\n2. Check the data with your tools; don't guess.\n3. Report what needs attention first, with links to the evidence, then what can wait.`,
    skills: skills.length ? skills : ['logs', 'errors'],
    schedule: daily ? '0 9 * * *' : weekly ? '0 9 * * 1' : null,
  };
}

// ---------------------------------------------------------------- credits and usage

const APPS: [string, number][] = [
  ['chat', 1],
  ['investigation', 0.6],
  ['security', 0.25],
  ['agent', 0.3],
  ['code', 0.15],
];

function usageRows(range: TimeRange) {
  const rows: { app: string; user: string; at: number; input: number; output: number }[] = [];
  let s = 7;
  const rand = () => (s = (s * 1_103_515_245 + 12_345) >>> 0) / 4_294_967_296;
  const users = [ME, 'Jordan Lee', 'Sam Patel', 'Riley Chen'];
  for (let t = Math.floor(Date.now() / DAY) * DAY - 30 * DAY; t <= Date.now(); t += DAY) {
    for (const [app, weight] of APPS) {
      const n = Math.floor(rand() * 6 * weight);
      for (let i = 0; i < n; i++)
        rows.push({
          app,
          user: users[Math.floor(rand() * users.length)],
          at: t + Math.floor(rand() * DAY * 0.9),
          input: 6_000 + Math.floor(rand() * 20_000),
          output: 400 + Math.floor(rand() * 2_000),
        });
    }
  }
  for (const r of db()) rows.push({ app: r.summary.kind, user: ME, at: r.summary.createdAt, input: r.summary.inputTokens, output: r.summary.outputTokens });
  return rows.filter((r) => r.at >= range.from && r.at < range.to && r.at <= Date.now());
}

const toCredits = (tokens: number) => Math.ceil(tokens / 1000);

export async function getCredits(): Promise<CreditsView> {
  const now = new Date();
  const month = Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), 1);
  const day = Math.floor(Date.now() / DAY) * DAY;
  const rows = usageRows({ from: month, to: Date.now() + 1 });
  return {
    ...credits,
    usedThisMonth: toCredits(rows.reduce((a, r) => a + r.input + r.output, 0)),
    myUsedToday: toCredits(rows.filter((r) => r.user === ME && r.at >= day).reduce((a, r) => a + r.input + r.output, 0)),
    tokensPerCredit: 1000,
  };
}

export async function putCredits(c: CreditSettings): Promise<void> {
  if ((c.monthlyLimit ?? 0) < 0 || (c.userDailyLimit ?? 0) < 0) throw new Error("Limits can't be negative");
  credits = { ...c };
}

export async function usage(range: TimeRange): Promise<UsageReport> {
  const rows = usageRows(range);
  const cost = (i: number, o: number) =>
    credits.inputPricePerMtok != null && credits.outputPricePerMtok != null ? (i * credits.inputPricePerMtok + o * credits.outputPricePerMtok) / 1_000_000 : null;
  const sum = (rs: typeof rows) => [rs.reduce((a, r) => a + r.input, 0), rs.reduce((a, r) => a + r.output, 0)];
  const [input, output] = sum(rows);
  const days = new Map<number, typeof rows>();
  for (const r of rows) days.set(Math.floor(r.at / DAY) * DAY, [...(days.get(Math.floor(r.at / DAY) * DAY) ?? []), r]);
  const users = new Map<string, typeof rows>();
  for (const r of rows) users.set(r.user, [...(users.get(r.user) ?? []), r]);
  return {
    users: users.size,
    inputTokens: input,
    outputTokens: output,
    credits: toCredits(input + output),
    estCost: cost(input, output),
    daily: [...days.entries()].sort((a, b) => a[0] - b[0]).map(([at, rs]) => ({ at, users: new Set(rs.map((r) => r.user)).size, credits: toCredits(sum(rs)[0] + sum(rs)[1]) })),
    byApp: APPS.map(([app]) => {
      const rs = rows.filter((r) => r.app === app);
      const [i, o] = sum(rs);
      return { app, users: new Set(rs.map((r) => r.user)).size, uses: rs.length, inputTokens: i, outputTokens: o, credits: toCredits(i + o), estCost: cost(i, o) };
    }),
    byUser: [...users.entries()].map(([name, rs]) => ({ userId: name, name, uses: rs.length, credits: toCredits(sum(rs)[0] + sum(rs)[1]) })).sort((a, b) => b.credits - a.credits),
  };
}

// ---------------------------------------------------------------- code

export async function codeConnections(): Promise<CodeConnections> {
  return { github: 'assist-bot', gitlab: null };
}

export async function listRepos(): Promise<RepoView[]> {
  return structuredClone(repos);
}

export async function addRepo(r: NewRepo): Promise<void> {
  const repo = r.repo.trim().replace(/\.git$/, '');
  if (!/^[\w.-]+(\/[\w.-]+)+$/.test(repo)) throw new Error(`${repo} isn't a repository name like owner/name`);
  if (r.provider === 'gitlab') throw new Error('No gitlab connection: add one in Developer Portal → Settings');
  repos = [...repos.filter((x) => x.repo !== repo), { repo, provider: r.provider, services: r.services ?? [], addedBy: ME, addedAt: Date.now() }].sort((a, b) => a.repo.localeCompare(b.repo));
}

export async function removeRepo(repo: string): Promise<void> {
  repos = repos.filter((r) => r.repo !== repo);
}

export async function getCodeSettings(): Promise<CodeSettings> {
  return { ...codeSettings };
}

export async function putCodeSettings(s: CodeSettings): Promise<void> {
  if (!/^[\w./-]{0,40}$/.test(s.branchPrefix) || s.branchPrefix.startsWith('/')) throw new Error('The branch prefix uses letters, digits, - _ . / (up to 40)');
  codeSettings = { ...s };
}

export async function startCodeSession(repo: string, prompt: string): Promise<RunSummary> {
  if (!repos.some((r) => r.repo === repo)) throw new Error(`${repo} isn't added to Assist Code (Code → Settings → Repositories)`);
  if (!prompt.trim()) throw new Error('Describe the change in 1 to 8000 characters');
  return add('code', prompt.trim().split('\n')[0].slice(0, 100), { repo, prompt: prompt.trim() }, codeScript(repo, prompt.trim()));
}

export async function openPullRequest(id: string): Promise<PullRequest> {
  const r = find(id);
  const v = view(r);
  if (r.pullRequest) throw new Error('A pull request was already opened for this session');
  if (!v.change) throw new Error('This session proposed no change');
  const slug = v.change.title
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '')
    .slice(0, 48);
  const pr = {
    url: `https://github.com/${(v.subject as { repo: string }).repo}/pull/${4130 + db().indexOf(r)}`,
    number: 4130 + db().indexOf(r),
    branch: `${codeSettings.branchPrefix}${slug}-${id.slice(0, 8)}`,
    draft: codeSettings.draft,
  };
  r.pullRequest = { ...pr, state: 'open' };
  return pr;
}

export async function codeUsage(range: TimeRange): Promise<CodeUsage> {
  const rs = db().filter((r) => r.summary.kind === 'code' && r.summary.createdAt >= range.from && r.summary.createdAt < range.to);
  return {
    sessions: rs.length,
    pullRequestsOpened: rs.filter((r) => r.pullRequest).length,
    pullRequestsMerged: rs.filter((r) => r.pullRequest?.state === 'merged').length,
    sessionsPerDay: bucketed(
      rs.map((r) => r.summary.createdAt),
      0,
      10 * DAY,
    ),
    pullRequestsPerDay: bucketed(
      rs.filter((r) => r.pullRequest).map((r) => r.summary.createdAt),
      0,
      10 * DAY,
    ),
  };
}

/** Test hook: start over. */
export function resetAssistRunsMock(): void {
  runs = null;
  agents = null;
  memories = [];
  auto = new Set();
  settings = { knowledge: '', runsPerHour: 30, automaticRunsPerDay: 100, notify: [], securityAutoSeverities: [], securityRunAs: null };
  repos = [{ repo: 'example-org/storefront', provider: 'github', services: ['storefront-api', 'order-service'], addedBy: ME, addedAt: Date.now() - 20 * DAY }];
  codeSettings = { branchPrefix: 'assist/', draft: true, autoPullRequest: false };
  credits = { monthlyLimit: 50_000, userDailyLimit: 2_000, inputPricePerMtok: 5, outputPricePerMtok: 25 };
}
