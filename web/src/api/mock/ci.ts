// In-browser stand-in for the Software Delivery endpoints (see api/ci/client.ts), mirroring the server
// (routes/ci.rs) over pipeline runs generated like oc-tracegen's CI generator (oc-tracegen/src/ci.rs):
// the same example-org repositories, pipelines (ci → lint, unit tests, build, integration tests on
// main; deploy → build image, staging, smoke tests, production; terraform plan/apply), flaky tests that
// fail now and then and get retried on the same commit, and deploys on main behind the DORA metrics.
//
// Time is cut into 10-minute slots; each slot's pushes come from a generator seeded by the slot, so any
// range shows the same runs and a trace id (which starts with its slot) finds its run again. Runs only
// appear once they have finished, as with a CI provider's exporter. Flaky test states are kept in
// memory for the session.
import type { components } from '../generated/schema';
import { USERS } from './errorTracking';
import { rng, type Rand } from './generator';

type S = components['schemas'];
type CiQuery = S['CiQuery'];
type CiExecution = S['CiExecution'];
type CiAggregate = S['CiAggregate'];
type CiFacetValue = S['CiFacetValue'];
type CiKpis = S['CiKpis'];
type DoraSummary = S['DoraSummary'];
type FlakyState = S['FlakyState'];
type TimeRange = S['TimeRange'];

const SEC = 1000;
const MIN = 60 * SEC;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;
const SLOT = 10 * MIN;

const bad = (message: string) => new Error(message);

function hash(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619);
  return h >>> 0;
}

const hex = (r: Rand, chars: number) => Array.from({ length: chars }, () => Math.floor(r() * 16).toString(16)).join('');
const range = (r: Rand, lo: number, hi: number) => lo + Math.floor(r() * (hi - lo));
const chance = (r: Rand, p: number) => r() < p;
const pick = <T>(r: Rand, a: readonly T[]) => a[Math.floor(r() * a.length)];

function groupBy<T>(items: T[], key: (x: T) => string): Map<string, T[]> {
  const out = new Map<string, T[]>();
  for (const x of items) {
    const k = key(x);
    const list = out.get(k);
    if (list) list.push(x);
    else out.set(k, [x]);
  }
  return out;
}

// ---------------------------------------------------------------- the generator's fixtures (oc-tracegen/src/ci.rs)

interface Repo {
  name: string;
  service: string;
  team: string;
  provider: string;
  suite: string;
  /** Chance that a commit genuinely breaks the build. */
  failRate: number;
  /** Tests that sometimes fail for no reason. */
  flaky: string[];
  infra: boolean;
}

const REPOS: Repo[] = [
  {
    name: 'example-org/storefront-api',
    service: 'storefront-api',
    team: 'storefront',
    provider: 'github_actions',
    suite: 'StorefrontApiTest',
    failRate: 0.08,
    flaky: ['returns cached product page', 'paginates search results'],
    infra: false,
  },
  {
    name: 'example-org/order-service',
    service: 'order-service',
    team: 'checkout',
    provider: 'github_actions',
    suite: 'OrderServiceTest',
    failRate: 0.12,
    flaky: ['reserves inventory before payment'],
    infra: false,
  },
  { name: 'example-org/billing', service: 'billing', team: 'billing', provider: 'github_actions', suite: 'InvoiceTest', failRate: 0.05, flaky: [], infra: false },
  {
    name: 'example-org/catalog-service',
    service: 'catalog-service',
    team: 'catalog',
    provider: 'github_actions',
    suite: 'CatalogTest',
    failRate: 0.07,
    flaky: ['reindexes changed prices'],
    infra: false,
  },
  {
    name: 'example-org/inventory-service',
    service: 'inventory-service',
    team: 'checkout',
    provider: 'github_actions',
    suite: 'StockLevelTest',
    failRate: 0.06,
    flaky: ['releases expired reservations'],
    infra: false,
  },
  {
    name: 'example-org/notification-worker',
    service: 'notification-worker',
    team: 'notifications',
    provider: 'github_actions',
    suite: 'test_notifications',
    failRate: 0.1,
    flaky: ['test_retries_on_provider_timeout'],
    infra: false,
  },
  { name: 'example-org/web-frontend', service: 'web-frontend', team: 'storefront', provider: 'github_actions', suite: 'checkout.spec', failRate: 0.09, flaky: ['renders cart drawer'], infra: false },
  { name: 'example-org/infra-terraform', service: 'infra', team: 'platform', provider: 'gitlab', suite: '', failRate: 0.1, flaky: [], infra: true },
];

const TESTS = [
  'creates a record',
  'rejects invalid input',
  'returns 404 for unknown ids',
  'handles concurrent updates',
  'serializes to json',
  'applies discounts',
  'validates currency',
  'retries transient errors',
  'emits audit events',
  'respects rate limits',
  'migrates schema',
  'closes connections on shutdown',
  'parses config',
  'computes totals',
  'authorizes requests',
];
const AUTHORS = ['Jordan Patel', 'Riley Meyer', 'Casey Garcia', 'Morgan Silva', 'Avery Kim', 'Quinn Okafor', 'Taylor Novak', 'Sage Ito'];
const MESSAGES = [
  'Fix rounding in totals',
  'Add retry to provider client',
  'Bump dependencies',
  'Refactor handler wiring',
  'Improve error messages',
  'Add index for lookups',
  'Speed up test suite',
  'Tune connection pool',
  'Remove dead feature flag',
  'Log request ids',
];
const BRANCHES = ['feature/faster-search', 'fix/currency-rounding', 'chore/deps', 'feature/gift-cards', 'fix/timeout-retries', 'feature/saved-carts'];
const STEPS = ['Set up job', 'Checkout', 'Run', 'Post job'];

// ---------------------------------------------------------------- runs

interface Commit {
  sha: string;
  author: string;
  message: string;
  /** When the change was committed (ms). */
  at: number;
}

interface TestCase {
  name: string;
  failed: boolean;
  start: number;
  durationMs: number;
}

interface Job {
  spanId: string;
  name: string;
  stage: string;
  /** build | test | deploy */
  kind: string;
  start: number;
  end: number;
  failed: boolean;
  /** Deploy jobs: staging | production. */
  environment: string;
  /** Step durations (ms), in STEPS order. */
  steps: number[];
  /** Test cases inside the Run step (unit-tests jobs). */
  tests: TestCase[];
}

interface Run {
  traceId: string;
  spanId: string;
  repo: Repo;
  pipeline: string;
  branch: string;
  commit: Commit;
  attempt: number;
  start: number;
  end: number;
  status: 'success' | 'error' | 'canceled';
  jobs: Job[];
  failedFlaky: boolean;
}

function makeRun(r: Rand, traceId: string, repo: Repo, pipeline: string, branch: string, c: Commit, attempt: number, broken: boolean, start: number): Run {
  const deploy = pipeline === 'deploy' || pipeline === 'terraform apply';
  const plan: [string, string, string, number][] = repo.infra
    ? deploy
      ? [
          ['init', 'setup', 'build', 20],
          ['plan', 'plan', 'build', 50],
          ['apply', 'apply', 'deploy', 110],
        ]
      : [
          ['init', 'setup', 'build', 20],
          ['validate', 'verify', 'test', 15],
          ['plan', 'plan', 'build', 55],
        ]
    : deploy
      ? [
          ['build-image', 'build', 'build', 140],
          ['deploy-staging', 'staging', 'deploy', 70],
          ['smoke-tests', 'staging', 'test', 45],
          ['deploy-production', 'production', 'deploy', 90],
        ]
      : [
          ['lint', 'verify', 'test', 25],
          ['unit-tests', 'test', 'test', 120],
          ['build', 'build', 'build', 80],
          ...(branch === 'main' ? [['integration-tests', 'test', 'test', 200] as [string, string, string, number]] : []),
        ];
  const jobs: Job[] = [];
  let t = start + range(r, 2, 15) * SEC;
  let failed = false;
  let failedFlaky = false;
  for (const [name, stage, kind, median] of plan) {
    if (failed) break;
    const job: Job = { spanId: hex(r, 16), name, stage, kind, start: t, end: t, failed: false, environment: '', steps: [], tests: [] };
    let stepT = t + SEC;
    for (const step of STEPS) {
      const d = (step === 'Run' ? Math.floor(median * (0.7 + r() * 0.8)) : range(r, 1, 6)) * SEC;
      if (step === 'Run' && name === 'unit-tests' && repo.suite) {
        let tt = stepT + SEC;
        [...TESTS, ...repo.flaky].forEach((test, i) => {
          const isFlaky = i >= TESTS.length;
          const fail = (isFlaky && chance(r, 0.18)) || (broken && i === 3);
          if (fail) {
            job.failed = true;
            if (isFlaky && !broken) failedFlaky = true;
          }
          const td = range(r, 80, 2500);
          job.tests.push({ name: test, failed: fail, start: tt, durationMs: td });
          tt += td + range(r, 5, 60);
        });
      } else if (step === 'Run' && broken && (name === 'plan' || (name === 'lint' && !repo.suite))) {
        job.failed = true;
      }
      if (step === 'Run' && broken && name === 'build' && chance(r, 0.3)) job.failed = true;
      job.steps.push(d);
      stepT += d + SEC / 2;
    }
    // Deployments occasionally fail on their own (bad config, health checks).
    if (kind === 'deploy' && chance(r, 0.04)) job.failed = true;
    if (kind === 'deploy') job.environment = name.includes('staging') ? 'staging' : 'production';
    job.end = stepT;
    jobs.push(job);
    failed = job.failed;
    t = stepT + range(r, 3, 25) * SEC;
  }
  // A small share of runs are canceled by a newer push.
  const canceled = !failed && branch !== 'main' && chance(r, 0.02);
  return { traceId, spanId: hex(r, 16), repo, pipeline, branch, commit: c, attempt, start, end: t, status: canceled ? 'canceled' : failed ? 'error' : 'success', jobs, failedFlaky };
}

/** The pushes of one slot: a CI run (retried once when only flaky tests failed), then on main a deploy when CI passed. */
function slotRuns(slot: number): Run[] {
  const r = rng(hash(`ci-slot-${slot}`));
  const slotStart = slot * SLOT;
  // Busier during working hours.
  const hour = new Date(slotStart).getUTCHours();
  const busy = hour >= 7 && hour < 19;
  const pushes = (chance(r, busy ? 0.9 : 0.3) ? 1 : 0) + (busy && chance(r, 0.45) ? 1 : 0);
  const runs: Run[] = [];
  for (let p = 0; p < pushes; p++) {
    const start = slotStart + (p * SLOT) / 2 + range(r, 0, SLOT / 2);
    const repo = pick(r, REPOS);
    const branch = chance(r, 0.45) ? 'main' : pick(r, BRANCHES);
    const c: Commit = { sha: hex(r, 40), author: pick(r, AUTHORS), message: pick(r, MESSAGES), at: start - range(r, 15 * 60, 30 * 3600) * SEC };
    const broken = chance(r, repo.failRate);
    const traceId = () => `${slot.toString(16).padStart(8, '0')}${(p * 4 + runs.length).toString(16).padStart(2, '0')}${hex(r, 22)}`;
    const ciName = repo.infra ? 'terraform plan' : 'ci';
    let run = makeRun(r, traceId(), repo, ciName, branch, c, 1, broken, start);
    runs.push(run);
    if (run.failedFlaky && !broken) {
      run = makeRun(r, traceId(), repo, ciName, branch, c, 2, false, run.end + range(r, 30, 300) * SEC);
      runs.push(run);
    }
    if (run.status === 'success' && branch === 'main') runs.push(makeRun(r, traceId(), repo, repo.infra ? 'terraform apply' : 'deploy', branch, c, 1, false, run.end + range(r, 10, 120) * SEC));
  }
  return runs;
}

const cache = new Map<number, Run[]>();

function runsOf(slot: number): Run[] {
  let runs = cache.get(slot);
  if (!runs) cache.set(slot, (runs = slotRuns(slot)));
  return runs;
}

/** Finished runs that started in `[from, to]` (a run starts at most ~2 hours after its slot). */
function runsIn(from: number, to: number): Run[] {
  const now = Date.now();
  const out: Run[] = [];
  for (let slot = Math.floor((from - 2 * HOUR) / SLOT); slot <= Math.floor(to / SLOT); slot++)
    for (const run of runsOf(slot)) if (run.start >= from && run.start <= to && run.end <= now) out.push(run);
  return out.sort((a, b) => a.start - b.start);
}

// ---------------------------------------------------------------- rows

interface Row {
  run: Run;
  job: Job | null;
  exec: CiExecution;
}

function execution(run: Run, job: Job | null): CiExecution {
  const runId = String(Math.floor(run.start / SEC / 7) % 1_000_000);
  return {
    traceId: run.traceId,
    spanId: job ? job.spanId : run.spanId,
    name: job ? job.name : run.pipeline,
    pipelineName: run.pipeline,
    status: job ? (job.failed ? 'error' : 'success') : run.status,
    start: job ? job.start : run.start,
    durationMs: job ? job.end - job.start : run.end - run.start,
    runId,
    runUrl: `https://ci.example.com/${run.repo.name}/runs/${runId}`,
    attempt: String(run.attempt),
    repository: run.repo.name,
    repositoryUrl: `https://git.example.com/${run.repo.name}`,
    branch: run.branch,
    sha: run.commit.sha,
    author: run.commit.author,
    message: run.commit.message,
    provider: run.repo.provider,
    stage: job ? job.stage : '',
    team: run.repo.team,
  };
}

function rowsOf(range: TimeRange, level: S['CiLevel']): Row[] {
  const runs = runsIn(range.from - 6 * HOUR, range.to);
  const rows: Row[] = level === 'job' ? runs.flatMap((run) => run.jobs.map((job) => ({ run, job, exec: execution(run, job) }))) : runs.map((run) => ({ run, job: null, exec: execution(run, null) }));
  return rows.filter((r) => r.exec.start >= range.from && r.exec.start <= range.to);
}

type Facet = 'status' | 'pipeline' | 'repository' | 'branch' | 'provider' | 'team';

const FACET_FIELD: Record<Facet, (e: CiExecution) => string> = {
  status: (e) => e.status,
  pipeline: (e) => e.pipelineName,
  repository: (e) => e.repository,
  branch: (e) => e.branch,
  provider: (e) => e.provider,
  team: (e) => e.team,
};

const TERM_FIELD: Record<string, (e: CiExecution, r: Row) => string> = {
  status: (e) => e.status,
  '@ci.status': (e) => e.status,
  pipeline: (e) => e.pipelineName,
  '@ci.pipeline.name': (e) => e.pipelineName,
  repository: (e) => e.repository,
  repo: (e) => e.repository,
  '@git.repository.name': (e) => e.repository,
  branch: (e) => e.branch,
  '@git.branch': (e) => e.branch,
  provider: (e) => e.provider,
  '@ci.provider.name': (e) => e.provider,
  team: (e) => e.team,
  sha: (e) => e.sha,
  commit: (e) => e.sha,
  '@git.commit.sha': (e) => e.sha,
  job: (_e, r) => r.job?.name ?? '',
  '@ci.job.name': (_e, r) => r.job?.name ?? '',
  stage: (e) => e.stage,
  '@ci.stage.name': (e) => e.stage,
  author: (e) => e.author,
};

/** Executions matching a query, optionally ignoring one facet's filter (for that facet's counts). */
function matching(q: CiQuery, skip?: Facet): Row[] {
  if (!(q.range.from < q.range.to)) throw bad('the time range must end after it starts');
  const f = q.filters;
  const lists: [Facet, string[] | undefined][] = [
    ['status', f?.statuses],
    ['pipeline', f?.pipelines],
    ['repository', f?.repositories],
    ['branch', f?.branches],
    ['provider', f?.providers],
    ['team', f?.teams],
  ];
  const tests: ((r: Row) => boolean)[] = [];
  for (const [facet, values] of lists) if (facet !== skip && values?.length) tests.push((r) => values.includes(FACET_FIELD[facet](r.exec)));
  for (const raw of (q.query ?? '').split(/\s+/).filter(Boolean)) {
    const neg = raw.startsWith('-');
    const word = neg ? raw.slice(1) : raw;
    const i = word.indexOf(':');
    let test: (r: Row) => boolean;
    if (i >= 0 && i < word.length - 1) {
      const key = word.slice(0, i);
      const value = word.slice(i + 1);
      const field = TERM_FIELD[key];
      if (!field) throw bad(`unknown filter '${key}' (use status, pipeline, repo, branch, provider, team, sha, job, stage or author)`);
      if (value.includes('*')) {
        const re = new RegExp(
          `^${value
            .split('*')
            .map((s) => s.replace(/[.+?^${}()|[\]\\]/g, '\\$&'))
            .join('.*')}$`,
          'i',
        );
        test = (r) => re.test(field(r.exec, r));
      } else test = (r) => field(r.exec, r) === value;
    } else {
      const w = word.toLowerCase();
      test = (r) => r.exec.name.toLowerCase().includes(w) || r.exec.repository.toLowerCase().includes(w);
    }
    tests.push(neg ? (r) => !test(r) : test);
  }
  return rowsOf(q.range, q.level ?? 'pipeline').filter((r) => tests.every((t) => t(r)));
}

export function searchExecutions(q: CiQuery, sort: string, offset: number, limit: number): S['CiExecutionPage'] {
  const rows = matching(q).map((r) => r.exec);
  rows.sort(sort === 'duration' ? (a, b) => b.durationMs - a.durationMs : (a, b) => b.start - a.start);
  const from = Math.min(100_000, Math.max(0, offset));
  return { executions: rows.slice(from, from + Math.min(500, Math.max(1, limit))), total: rows.length };
}

function facetValues(values: string[]): CiFacetValue[] {
  const map = new Map<string, number>();
  for (const v of values) if (v !== '') map.set(v, (map.get(v) ?? 0) + 1);
  return [...map.entries()]
    .map(([value, count]) => ({ value, count }))
    .sort((a, b) => b.count - a.count || (a.value < b.value ? -1 : 1))
    .slice(0, 100);
}

export function executionFacets(q: CiQuery): S['CiFacets'] {
  const facet = (f: Facet) => facetValues(matching(q, f).map((r) => FACET_FIELD[f](r.exec)));
  return { status: facet('status'), pipeline: facet('pipeline'), repository: facet('repository'), branch: facet('branch'), provider: facet('provider'), team: facet('team') };
}

function stepFor(r: TimeRange, buckets: number): number {
  const raw = (r.to - r.from) / buckets;
  const steps = [MIN, 5 * MIN, 10 * MIN, 30 * MIN, HOUR, 3 * HOUR, 6 * HOUR, 12 * HOUR, DAY, 7 * DAY];
  return steps.find((s) => s >= raw) ?? 7 * DAY;
}

export function executionCharts(q: CiQuery): S['CiCharts'] {
  const rows = matching(q).map((r) => r.exec);
  const step = stepFor(q.range, 60);
  const first = Math.floor(q.range.from / step) * step;
  const times: number[] = [];
  for (let t = first; t <= q.range.to; t += step) times.push(t);
  const totals = new Map<string, number>();
  for (const e of rows) totals.set(e.name, (totals.get(e.name) ?? 0) + 1);
  const top = [...totals.entries()]
    .sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : 1))
    .slice(0, 8)
    .map((x) => x[0]);
  const by = new Map<string, { dur: number[]; errored: number[]; executions: number[] }>();
  for (const e of rows) {
    const i = Math.floor((e.start - first) / step);
    if (i < 0 || i >= times.length) continue;
    const key = top.includes(e.name) ? e.name : 'other';
    let s = by.get(key);
    if (!s) by.set(key, (s = { dur: times.map(() => 0), errored: times.map(() => 0), executions: times.map(() => 0) }));
    s.dur[i] += e.durationMs;
    s.errored[i] += e.status === 'error' ? 1 : 0;
    s.executions[i]++;
  }
  const series = [...by.entries()]
    .sort((a, b) => (a[0] < b[0] ? -1 : 1))
    .map(([name, s]) => ({ name, duration: s.dur.map((d, i) => (s.executions[i] ? Math.round(d / s.executions[i]) : null)), errored: s.errored, executions: s.executions }))
    .sort((a, b) => b.executions.reduce((x, y) => x + y, 0) - a.executions.reduce((x, y) => x + y, 0));
  return { stepMs: step, times, series };
}

/** Linear-interpolated quantile, as ClickHouse's `quantile` approximates it. */
function quantile(values: number[], p: number): number {
  if (!values.length) return 0;
  const v = [...values].sort((a, b) => a - b);
  const x = (v.length - 1) * p;
  const lo = Math.floor(x);
  return v[lo] + (v[Math.min(lo + 1, v.length - 1)] - v[lo]) * (x - lo);
}

function aggregate(rows: CiExecution[]): CiAggregate[] {
  return [...groupBy(rows, (e) => `${e.name}\u0000${e.repository}`).values()]
    .map((g) => {
      const last = g.reduce((a, b) => (b.start > a.start ? b : a));
      const d = g.map((e) => e.durationMs);
      return {
        name: g[0].name,
        repository: g[0].repository,
        executions: g.length,
        failures: g.filter((e) => e.status === 'error').length,
        avgMs: d.reduce((a, b) => a + b, 0) / d.length,
        p50Ms: quantile(d, 0.5),
        p95Ms: quantile(d, 0.95),
        lastStatus: last.status,
        lastStart: last.start,
        lastTraceId: last.traceId,
      };
    })
    .sort((a, b) => b.executions - a.executions || (a.name + a.repository < b.name + b.repository ? -1 : 1))
    .slice(0, 500);
}

export function aggregations(q: CiQuery): CiAggregate[] {
  return aggregate(matching(q).map((r) => r.exec));
}

export function pipelineDetail(traceId: string): S['CiPipelineDetail'] {
  if (!/^[0-9a-f]{32}$/i.test(traceId)) throw bad('invalid trace id');
  const run = runsOf(parseInt(traceId.slice(0, 8), 16)).find((x) => x.traceId === traceId.toLowerCase());
  if (!run || run.end > Date.now()) throw bad('pipeline execution not found');
  const spans: S['CiSpan'][] = [
    {
      spanId: run.spanId,
      parentSpanId: '',
      name: run.pipeline,
      kind: 'pipeline',
      status: run.status,
      start: run.start,
      durationMs: run.end - run.start,
      stage: '',
      jobType: '',
      environment: '',
      testSuite: '',
    },
  ];
  const span = (o: Partial<S['CiSpan']> & Pick<S['CiSpan'], 'spanId' | 'parentSpanId' | 'name' | 'kind' | 'status' | 'start' | 'durationMs'>): S['CiSpan'] => ({
    stage: '',
    jobType: '',
    environment: '',
    testSuite: '',
    ...o,
  });
  for (const job of run.jobs) {
    const r = rng(hash(`${traceId}/${job.spanId}`));
    spans.push(
      span({
        spanId: job.spanId,
        parentSpanId: run.spanId,
        name: job.name,
        kind: 'job',
        status: job.failed ? 'error' : 'success',
        start: job.start,
        durationMs: job.end - job.start,
        stage: job.stage,
        jobType: job.kind,
        environment: job.environment,
      }),
    );
    let t = job.start + SEC;
    job.steps.forEach((d, i) => {
      const stepId = hex(r, 16);
      spans.push(span({ spanId: stepId, parentSpanId: job.spanId, name: STEPS[i], kind: 'step', status: job.failed && STEPS[i] === 'Run' ? 'error' : 'success', start: t, durationMs: d }));
      if (STEPS[i] === 'Run')
        for (const tc of job.tests)
          spans.push(
            span({
              spanId: hex(r, 16),
              parentSpanId: stepId,
              name: `${run.repo.suite}.${tc.name}`,
              kind: 'test',
              status: tc.failed ? 'error' : 'success',
              start: tc.start,
              durationMs: tc.durationMs,
              testSuite: run.repo.suite,
            }),
          );
      t += d + SEC / 2;
    });
  }
  spans.sort((a, b) => a.start - b.start);
  return { pipeline: execution(run, null), spans };
}

// ---------------------------------------------------------------- overview

function kpis(from: number, to: number): CiKpis {
  const rows = rowsOf({ from, to }, 'pipeline').map((r) => r.exec);
  const d = rows.map((e) => e.durationMs);
  return {
    executions: rows.length,
    failures: rows.filter((e) => e.status === 'error').length,
    p50Ms: quantile(d, 0.5),
    p95Ms: quantile(d, 0.95),
    repositories: new Set(rows.map((e) => e.repository)).size,
  };
}

export function ciOverview(r: TimeRange): S['CiOverview'] {
  if (!(r.from < r.to)) throw bad('the time range must end after it starts');
  const span = r.to - r.from;
  const q: CiQuery = { range: r, level: 'pipeline', query: '', filters: { statuses: [], pipelines: [], repositories: [], branches: [], providers: [], teams: [] } };
  const aggs = aggregations(q);
  return {
    current: kpis(r.from, r.to),
    previous: kpis(r.from - span, r.from),
    flakyTests: flakyRows(r, '').length,
    topFailing: aggs
      .filter((a) => a.failures > 0)
      .sort((a, b) => b.failures - a.failures)
      .slice(0, 5),
    slowest: [...aggs].sort((a, b) => b.p95Ms - a.p95Ms).slice(0, 5),
    recentFailures: searchExecutions({ ...q, filters: { ...q.filters!, statuses: ['error'] } }, 'date', 0, 6).executions,
  };
}

// ---------------------------------------------------------------- flaky tests

interface FlakyRow {
  repository: string;
  suite: string;
  name: string;
  runs: number;
  failures: number;
  avgMs: number;
  flakyCommits: number;
  firstFlake: number;
  lastFlake: number;
  lastFailedTraceId: string;
  team: string;
}

/** A test is flaky when it both passed and failed on the same commit. */
function flakyRows(r: TimeRange, query: string): FlakyRow[] {
  const q = query.toLowerCase();
  const tests = runsIn(r.from - HOUR, r.to).flatMap((run) =>
    run.jobs.flatMap((job) =>
      job.tests
        .filter((t) => t.start >= r.from && t.start <= r.to)
        .filter((t) => !q || t.name.toLowerCase().includes(q) || run.repo.suite.toLowerCase().includes(q) || run.repo.name.toLowerCase().includes(q))
        .map((t) => ({ run, t })),
    ),
  );
  const out: FlakyRow[] = [];
  for (const list of groupBy(tests, (x) => stateKey(x.run.repo.name, x.run.repo.suite, x.t.name)).values()) {
    const flaky = [...groupBy(list, (x) => x.run.commit.sha).values()].filter((g) => g.some((x) => x.t.failed) && g.some((x) => !x.t.failed));
    if (!flaky.length) continue;
    const flakyTimes = flaky.flat().map((x) => x.t.start);
    const failed = list.filter((x) => x.t.failed);
    const lastFailed = failed.reduce((a, b) => (b.t.start > a.t.start ? b : a), failed[0]);
    out.push({
      repository: list[0].run.repo.name,
      suite: list[0].run.repo.suite,
      name: list[0].t.name,
      runs: list.length,
      failures: failed.length,
      avgMs: list.reduce((n, x) => n + x.t.durationMs, 0) / list.length,
      flakyCommits: flaky.length,
      firstFlake: Math.min(...flakyTimes),
      lastFlake: Math.max(...flakyTimes),
      lastFailedTraceId: lastFailed.run.traceId,
      team: list[0].run.repo.team,
    });
  }
  return out.sort((a, b) => b.flakyCommits - a.flakyCommits || b.failures - a.failures).slice(0, 500);
}

const flakyStates = new Map<string, { state: FlakyState; by: string }>();
const stateKey = (repository: string, suite: string, name: string) => `${repository}\u0000${suite}\u0000${name}`;

export function flakyTests(r: TimeRange, query: string, states: FlakyState[]): S['FlakyPage'] {
  if (!(r.from < r.to)) throw bad('the time range must end after it starts');
  const counts: Record<string, number> = { active: 0, quarantined: 0, disabled: 0, fixed: 0 };
  const tests: S['FlakyTest'][] = [];
  for (const row of flakyRows(r, query.trim())) {
    const s = flakyStates.get(stateKey(row.repository, row.suite, row.name));
    const state = s?.state ?? 'active';
    counts[state]++;
    if (states.length && !states.includes(state)) continue;
    tests.push({ ...row, state, failureRate: row.runs ? (row.failures * 100) / row.runs : 0, stateUpdatedBy: s?.by ?? null });
  }
  return { tests, counts };
}

export function setFlakyState(repository: string, suite: string, name: string, state: FlakyState): void {
  if ([repository, suite, name].some((s) => !s || s.length > 500)) throw bad('repository, suite and name are required');
  flakyStates.set(stateKey(repository, suite, name), { state, by: USERS[0].name });
}

// ---------------------------------------------------------------- DORA

interface Deploy {
  service: string;
  repository: string;
  start: number;
  end: number;
  failed: boolean;
  committedAt: number;
}

interface Incident {
  service: string;
  declaredAt: number;
  resolvedAt: number | null;
}

const median = (v: number[]) => (v.length ? [...v].sort((a, b) => a - b)[Math.floor(v.length / 2)] : null);

function deploysIn(from: number, to: number, env: string): Deploy[] {
  return runsIn(from - 6 * HOUR, to).flatMap((run) =>
    run.jobs
      .filter((j) => j.environment === env && j.start >= from && j.start <= to)
      .map((j) => ({ service: run.repo.service, repository: run.repo.name, start: j.start, end: j.end, failed: j.failed, committedAt: run.commit.at })),
  );
}

/** Incidents follow some failed production deploys and a rare successful one (declared shortly after). */
function incidentsIn(from: number, to: number): Incident[] {
  const now = Date.now();
  return deploysIn(from - DAY, to, 'production').flatMap((d) => {
    const r = rng(hash(`incident/${d.service}/${d.start}`));
    if (!chance(r, d.failed ? 0.15 : 0.004)) return [];
    const declaredAt = d.end + range(r, 5, 40) * MIN;
    const resolvedAt = declaredAt + range(r, 20, 180) * MIN;
    return declaredAt >= from && declaredAt <= to ? [{ service: d.service, declaredAt, resolvedAt: resolvedAt <= now ? resolvedAt : null }] : [];
  });
}

function summarize(deploys: Deploy[], days: number, incidents: Incident[]): DoraSummary {
  const ok = deploys.filter((d) => !d.failed);
  const failed = deploys.length - ok.length;
  const lead = median(ok.filter((d) => d.committedAt > 0).map((d) => d.end - d.committedAt));
  // A successful deployment followed by an incident on the same service within a day also counts as a failed change.
  const caused = ok.filter((d) => incidents.some((i) => i.service === d.service && i.declaredAt >= d.end && i.declaredAt - d.end <= DAY)).length;
  const cfr = deploys.length ? ((failed + caused) * 100) / deploys.length : null;
  const services = new Set(deploys.map((d) => d.service));
  const restore = median(incidents.filter((i) => services.has(i.service) && i.resolvedAt !== null).map((i) => i.resolvedAt! - i.declaredAt));
  const perDay = ok.length / Math.max(1, days);
  const ratings: Record<string, string> = {
    deploymentFrequency: perDay >= 1 ? 'elite' : perDay >= 1 / 7 ? 'high' : perDay >= 1 / 30 ? 'medium' : 'low',
  };
  if (lead !== null) ratings.leadTime = lead < DAY ? 'elite' : lead < 7 * DAY ? 'high' : lead < 30 * DAY ? 'medium' : 'low';
  if (cfr !== null) ratings.changeFailureRate = cfr <= 5 ? 'elite' : cfr <= 10 ? 'high' : cfr <= 15 ? 'medium' : 'low';
  if (restore !== null) ratings.timeToRestore = restore < HOUR ? 'elite' : restore < DAY ? 'high' : restore < 7 * DAY ? 'medium' : 'low';
  return {
    deployments: deploys.length,
    failedDeployments: failed,
    deploymentsPerDay: Math.round(perDay * 100) / 100,
    leadTimeMs: lead,
    changeFailureRate: cfr === null ? null : Math.round(cfr * 10) / 10,
    timeToRestoreMs: restore,
    ratings,
  };
}

export function doraMetrics(r: TimeRange, env: string | null, services: string[]): S['DoraResponse'] {
  if (!(r.from < r.to)) throw bad('the time range must end after it starts');
  const deploys = deploysIn(r.from, r.to, env ?? 'production').filter((d) => !services.length || services.includes(d.service));
  const now = Date.now();
  const recent = runsIn(now - 30 * DAY, now).flatMap((run) => run.jobs.map((j) => j.environment));
  const environments = [...new Set(recent.filter(Boolean))].sort();
  const incidents = incidentsIn(r.from, r.to);
  const days = (r.to - r.from) / DAY;
  const perDay = new Map<number, { n: number; failed: number; lead: number[] }>();
  for (let d = Math.floor(r.from / DAY); d <= Math.floor(r.to / DAY); d++) perDay.set(d * DAY, { n: 0, failed: 0, lead: [] });
  for (const d of deploys) {
    const e = perDay.get(Math.floor(d.start / DAY) * DAY)!;
    e.n++;
    if (d.failed) e.failed++;
    else if (d.committedAt > 0) e.lead.push(d.end - d.committedAt);
  }
  const byService = groupBy(
    [...deploys].sort((a, b) => (a.service < b.service ? -1 : a.service > b.service ? 1 : 0)),
    (d) => d.service,
  );
  const deployed = new Set(deploys.map((d) => d.service));
  return {
    summary: summarize(deploys, days, incidents),
    days: [...perDay.entries()].map(([day, e]) => ({ day, deployments: e.n, failed: e.failed, leadTimeMs: median(e.lead) })),
    services: [...byService.entries()]
      .map(([service, ds]) => ({ service, repository: ds[0].repository, lastDeploy: Math.max(...ds.map((d) => d.start)), summary: summarize(ds, days, incidents) }))
      .sort((a, b) => b.summary.deployments - a.summary.deployments),
    environments,
    incidents: incidents.filter((i) => deployed.has(i.service)).length,
  };
}

// ---------------------------------------------------------------- repositories

/** Repositories that reported pipeline runs in the last 30 days. */
export function ciRepositories(): S['CiRepository'][] {
  const now = Date.now();
  const rows = rowsOf({ from: now - 30 * DAY, to: now }, 'pipeline').map((r) => r.exec);
  return [...groupBy(rows, (e) => e.repository).entries()]
    .sort((a, b) => (a[0] < b[0] ? -1 : 1))
    .map(([repository, list]) => {
      const last = list.reduce((a, b) => (b.start > a.start ? b : a));
      return {
        repository,
        url: list[0].repositoryUrl,
        provider: list[0].provider,
        pipelines: [...new Set(list.map((e) => e.pipelineName))].sort(),
        runs: list.length,
        failures: list.filter((e) => e.status === 'error').length,
        lastRun: last.start,
        lastStatus: last.status,
      };
    });
}
