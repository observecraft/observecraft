#!/usr/bin/env node
// Page timings of the before/after test (README.md here): the requests the web app sends, replayed
// with a browser's limits (6 connections per origin, cookie session), against the old and the new
// stack in alternation.
//
//   node scripts/final-test/measure.mjs run [--runs 5] [--suites dashboard,pages,concurrent]
//   node scripts/final-test/measure.mjs once --version new --suite dashboard     (one warm load, no restart)
//   node scripts/final-test/measure.mjs summary                                  (medians / p90 of results.jsonl)
//
// Cold = server restarted, NEW's Valkey flushed (FLUSHALL), ClickHouse mark, index mark, uncompressed
// and query caches dropped; then the measured load. Warm = the same load again right after.
// ClickHouse work per load: system.query_log rows of the server's user (SELECTs started inside the
// load's window). Results go to .ft.local/results.jsonl.
import http from 'node:http';
import { execFileSync } from 'node:child_process';
import { appendFileSync, readFileSync, existsSync } from 'node:fs';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const here = fileURLToPath(new URL('.', import.meta.url));
const root = fileURLToPath(new URL('../../', import.meta.url));
const results = `${root}.ft.local/results.jsonl`;
const args = process.argv.slice(2);
const arg = (name, fallback) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : fallback;
};

const STACKS = {
  old: { api: 'http://localhost:18220', ch: 'http://localhost:18201', valkey: null },
  new: { api: 'http://localhost:18320', ch: 'http://localhost:18301', valkey: 'ocfin-valkey' },
};
const ORG = 'load-org-1';
const DASHBOARD = `${ORG}-kubernetes-kubernetes-dashboard`;
const LOGIN = { email: 'perf-test@example.com', password: 'final-test-local-only-pw', org: ORG };

// ---- the web app's own query planning (web/src/api/dashboards/execution.ts, web/src/lib/metricQuery.ts)
const esbuildFrom = [`${root}web/package.json`, `${root}../../../web/package.json`].find((p) => existsSync(p.replace('package.json', 'node_modules/esbuild')));
const esbuild = createRequire(esbuildFrom)('esbuild');
const bundle = async (entry) => {
  const { outputFiles } = esbuild.buildSync({ entryPoints: [`${root}web/src/${entry}`], bundle: true, platform: 'node', format: 'esm', write: false });
  return import(`data:text/javascript;base64,${Buffer.from(outputFiles[0].text).toString('base64')}`);
};
const { collectDashboardQueries } = await bundle('api/dashboards/execution.ts');
const { resolveQuery } = await bundle('lib/metricQuery.ts');

// ---- HTTP as a browser tab: one keep-alive pool of 6 sockets per origin, a session cookie
class User {
  constructor(version) {
    this.stack = STACKS[version];
    this.origin = this.stack.api;
    this.agent = new http.Agent({ keepAlive: true, maxSockets: 6 });
    this.cookie = '';
  }
  request(method, path, body, { sse = false } = {}) {
    const url = new URL(path, this.stack.api);
    const payload = body === undefined ? undefined : Buffer.from(JSON.stringify(body));
    const headers = { accept: sse ? 'text/event-stream' : 'application/json', origin: this.origin, cookie: this.cookie };
    if (payload) Object.assign(headers, { 'content-type': 'application/json', 'content-length': payload.length });
    const started = performance.now();
    return new Promise((resolve, reject) => {
      const req = http.request(url, { method, agent: this.agent, headers }, (res) => {
        const setCookie = res.headers['set-cookie'];
        if (setCookie) this.cookie = setCookie.map((c) => c.split(';')[0]).join('; ');
        if (sse) {
          // Resolves at the first complete `snapshot` event; the stream stays open until close().
          let text = '';
          res.setEncoding('utf8');
          res.on('data', (chunk) => {
            text += chunk;
            const m = /event: snapshot\ndata: (.*)\n\n/.exec(text);
            if (m && !req.done) {
              req.done = true;
              resolve({ status: res.statusCode, ms: performance.now() - started, json: JSON.parse(m[1]), close: () => req.destroy() });
            }
          });
          res.on('end', () => !req.done && reject(new Error(`${path}: stream ended (${res.statusCode}) ${text.slice(0, 300)}`)));
          return;
        }
        const chunks = [];
        res.on('data', (c) => chunks.push(c));
        res.on('end', () => {
          const raw = Buffer.concat(chunks).toString('utf8');
          let json = null;
          try {
            json = raw ? JSON.parse(raw) : null;
          } catch {
            /* not JSON (an error page) */
          }
          resolve({ status: res.statusCode, ms: performance.now() - started, json, bytes: raw.length });
        });
      });
      req.on('error', (e) => (req.done ? undefined : reject(e)));
      req.setTimeout(120_000, () => req.destroy(new Error(`${path}: timeout`)));
      if (payload) req.write(payload);
      req.end();
    });
  }
  async login() {
    const r = await this.request('POST', '/api/v1/auth/login', LOGIN);
    if (r.status !== 200) throw new Error(`login failed: ${r.status} ${JSON.stringify(r.json)}`);
  }
  close() {
    this.agent.destroy();
  }
}

const snapped = (durationMs) => {
  const to = Math.floor(Date.now() / 60000) * 60000;
  return { from: to - durationMs, to };
};
const sliding = (durationMs) => {
  const to = Date.now();
  return { from: to - durationMs, to };
};
const HOUR = 3_600_000;
const DAY = 24 * HOUR;

/** Times named requests started together; returns { total, each, statuses }. */
async function together(entries) {
  const started = performance.now();
  const out = await Promise.all(entries.map(async ([name, run]) => [name, await run()]));
  const each = {};
  const statuses = {};
  for (const [name, r] of out) {
    each[name] = Math.round(r.ms);
    statuses[name] = r.status;
  }
  return { total: Math.round(performance.now() - started), each, statuses, responses: Object.fromEntries(out) };
}

// ---- the Kubernetes - Overview dashboard as the page loads it (see README.md: request list)
async function loadDashboard(user) {
  const started = performance.now();
  const timings = {};
  const me = await user.request('GET', '/api/v1/me'); // AuthGate renders the app after this
  timings.me = Math.round(me.ms);
  const [dash, orgs] = await Promise.all([user.request('GET', `/api/v1/dashboards/${DASHBOARD}`), user.request('GET', '/api/v1/me/orgs')]);
  timings.dashboard = Math.round(dash.ms);
  timings.me_orgs = Math.round(orgs.ms);
  if (dash.status !== 200) throw new Error(`dashboard ${dash.status}`);
  const d = dash.json;
  const values = Object.fromEntries(d.templateVariables.map((t) => [t.name, t.default]));
  const queries = collectDashboardQueries(d.widgets, (q) => resolveQuery(q, d.templateVariables, values), { from: 0, to: HOUR });
  const range = snapped(HOUR);
  // The template variable bar mounts the first five variables (tree.ts visibleVariables).
  const vars = d.templateVariables.slice(0, 5).map((t) => [`tag_values:${t.prefix || '(scope)'}`, () => user.request('GET', `/api/v1/tags/${encodeURIComponent(t.prefix)}/values`)]);
  const kinds = ['pods', 'containers', 'deployments', 'daemonsets', 'statefulsets', 'cronjobs'];
  const phase = await together([
    ['metrics_stream', () => user.request('POST', '/api/v1/query/dashboard/live', { queries, range }, { sse: true })],
    ...vars,
    ['events', () => user.request('POST', '/api/v1/events/search', { range, query: 'source:kubernetes          ', limit: 25, offset: 0 })],
    ...kinds.map((kind) => [`inventory:${kind}`, () => user.request('POST', '/api/v1/orchestration/search', { kind, query: '', limit: 50 })]),
  ]);
  const total = Math.round(performance.now() - started);
  const snapshot = phase.responses.metrics_stream.json;
  phase.responses.metrics_stream.close();
  const withData = snapshot.results.filter((r) => r.series?.length > 0).length;
  const errors = snapshot.results.filter((r) => r.error).length;
  const failed = Object.entries(phase.statuses).filter(([name, s]) => s !== 200 && !name.endsWith('(scope)'));
  return { total, each: { ...timings, ...phase.each }, check: { queries: queries.length, with_data: withData, query_errors: errors, failed } };
}

// ---- other pages
const TAG_KEYS = ['kube_cluster_name', 'kube_namespace', 'kube_deployment', 'kube_daemon_set', 'kube_stateful_set', 'kube_replica_set', 'kube_service', 'kube_node', 'label'];
const PAGES = [
  ...TAG_KEYS.map((key) => [`tag_values:${key}`, (u) => together([[key, () => u.request('GET', `/api/v1/tags/${encodeURIComponent(key)}/values`)]])]),
  ...[
    ['1h', HOUR],
    ['24h', DAY],
  ].flatMap(([label, ms]) =>
    ['avg:system.cpu.user{*} by {host}', 'avg:kubernetes.cpu.usage.total{*} by {host}'].map((query) => [
      `metrics_explorer:${query.split('{')[0].slice(4)}:${label}`,
      (u) =>
        together([
          ['catalog', () => u.request('GET', '/api/v1/metrics')],
          ['timeseries', () => u.request('POST', '/api/v1/query/timeseries', { queries: [query], range: sliding(ms), points: 120 })],
        ]),
    ]),
  ),
  [
    'pods_explorer',
    (u) => together([['pods', () => u.request('POST', '/api/v1/orchestration/search', { kind: 'pods', query: '', groupBy: null, sort: 'name', desc: false, offset: 0, limit: 50, extra: [] })]]),
  ],
  [
    'errors_page',
    (u) => {
      const range = snapped(DAY);
      return together([
        ['issues', () => u.request('POST', '/api/v1/error-tracking/issues/search', { range, query: '', state: 'for_review', source: 'all', sort: 'relevance', limit: 10 })],
        ['facets', () => u.request('POST', '/api/v1/error-tracking/issues/facets', { range, query: '', source: 'all' })],
        ['users', () => u.request('POST', '/api/v1/users/search', { query: '', limit: 500, offset: 0 })],
      ]);
    },
  ],
  ['log_facets_1h', (u) => together([['facets', () => u.request('POST', '/api/v1/logs/facets', { range: sliding(HOUR), query: '', extra: [] })]])],
];

// ---- ClickHouse side
async function ch(version, sql) {
  const res = await fetch(`${STACKS[version].ch}/?user=default`, { method: 'POST', body: sql });
  const text = await res.text();
  if (!res.ok) throw new Error(`ClickHouse ${version}: ${text.slice(0, 300)}`);
  return text;
}
async function chWork(version, fromMs, toMs) {
  await ch(version, 'SYSTEM FLUSH LOGS');
  const text = await ch(
    version,
    `SELECT count() AS queries, sum(read_rows) AS read_rows, sum(read_bytes) AS read_bytes, round(sum(ProfileEvents['OSCPUVirtualTimeMicroseconds']) / 1e6, 3) AS cpu_s
     FROM system.query_log
     WHERE type IN ('QueryFinish', 'ExceptionWhileProcessing') AND user = 'observecraft' AND query_kind = 'Select'
       AND event_date >= yesterday() AND query_start_time_microseconds >= fromUnixTimestamp64Milli(${Math.floor(fromMs)})
       AND query_start_time_microseconds <= fromUnixTimestamp64Milli(${Math.ceil(toMs)})
     FORMAT JSONEachRow`,
  );
  const row = JSON.parse(text.trim().split('\n')[0]);
  return { queries: Number(row.queries), read_rows: Number(row.read_rows), read_bytes: Number(row.read_bytes), cpu_s: Number(row.cpu_s) };
}

// ---- cold start
const sh = (...a) => execFileSync('bash', a, { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] }).toString();
async function coldStart(version) {
  sh(`${here}stack.sh`, version, 'stop', 'server');
  for (const stmt of ['SYSTEM DROP MARK CACHE', 'SYSTEM DROP INDEX MARK CACHE', 'SYSTEM DROP UNCOMPRESSED CACHE', 'SYSTEM DROP QUERY CACHE']) {
    try {
      await ch(version, stmt);
    } catch (e) {
      console.warn(String(e.message).slice(0, 120));
    }
  }
  if (STACKS[version].valkey) execFileSync('docker', ['exec', STACKS[version].valkey, 'valkey-cli', 'FLUSHALL']);
  sh(`${here}stack.sh`, version, 'server');
  for (let i = 0; ; i++) {
    try {
      const res = await fetch(`${STACKS[version].api}/api/v1/me`);
      if (res.status === 401) break;
    } catch {
      /* not listening yet */
    }
    if (i > 600) throw new Error(`${version} server did not come up`);
    await new Promise((r) => setTimeout(r, 100));
  }
  // Let startup work (migrations check, first job ticks) settle, as a server that has been up a while.
  await new Promise((r) => setTimeout(r, 10_000));
}

function record(entry) {
  appendFileSync(results, `${JSON.stringify({ at: new Date().toISOString(), ...entry })}\n`);
  const brief = entry.total !== undefined ? `${entry.total} ms` : '';
  console.log(
    entry.version,
    entry.suite,
    entry.name ?? '',
    entry.mode,
    `run ${entry.run}`,
    brief,
    entry.ch ? `ch ${entry.ch.queries}q ${entry.ch.read_rows} rows ${entry.ch.cpu_s} s` : '',
    entry.check ? JSON.stringify(entry.check) : '',
  );
}

async function measured(version, run) {
  const from = Date.now();
  const r = await run();
  const to = Date.now();
  return { ...r, ch: await chWork(version, from, to) };
}

async function dashboardSuite(version, run) {
  await coldStart(version);
  for (const mode of ['cold', 'warm']) {
    const user = new User(version);
    await user.login();
    const r = await measured(version, () => loadDashboard(user));
    user.close();
    record({ version, suite: 'dashboard', mode, run, ...r });
  }
}

async function concurrentSuite(version, run, users = 3) {
  await coldStart(version);
  for (const mode of ['cold', 'warm']) {
    const tabs = Array.from({ length: users }, () => new User(version));
    for (const t of tabs) await t.login();
    const r = await measured(version, async () => {
      const started = performance.now();
      const each = await Promise.all(tabs.map((t) => loadDashboard(t)));
      return { total: Math.round(performance.now() - started), users: each.map((e) => e.total), check: each.map((e) => e.check.with_data) };
    });
    tabs.forEach((t) => t.close());
    record({ version, suite: 'concurrent', mode, run, ...r });
  }
}

async function pagesSuite(version, run) {
  await coldStart(version);
  for (const mode of ['cold', 'warm']) {
    const user = new User(version);
    await user.login();
    for (const [name, page] of PAGES) {
      const r = await measured(version, () => page(user));
      delete r.responses;
      const failed = Object.entries(r.statuses).filter(([, s]) => s !== 200);
      record({ version, suite: 'pages', name, mode, run, total: r.total, each: r.each, ch: r.ch, check: failed.length ? { failed } : undefined });
    }
    user.close();
  }
}

const SUITES = { dashboard: dashboardSuite, pages: pagesSuite, concurrent: concurrentSuite };

// ---- summary
const quantile = (xs, q) => {
  const s = [...xs].sort((a, b) => a - b);
  if (!s.length) return null;
  const pos = (s.length - 1) * q;
  const lo = Math.floor(pos);
  return Math.round(s[lo] + (s[Math.min(lo + 1, s.length - 1)] - s[lo]) * (pos - lo));
};
function summary() {
  const rows = readFileSync(results, 'utf8')
    .trim()
    .split('\n')
    .map((l) => JSON.parse(l));
  const groups = new Map();
  for (const r of rows) {
    const key = [r.suite, r.name ?? '', r.mode, r.version].join('|');
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(r);
  }
  const out = [];
  for (const [key, rs] of [...groups].sort()) {
    const [suite, name, mode, version] = key.split('|');
    const median = (f) => quantile(rs.map(f), 0.5);
    out.push({
      suite,
      name,
      mode,
      version,
      n: rs.length,
      median_ms: median((r) => r.total),
      p90_ms: quantile(
        rs.map((r) => r.total),
        0.9,
      ),
      ch_queries: median((r) => r.ch.queries),
      ch_rows: median((r) => r.ch.read_rows),
      ch_mib:
        Math.round(
          (quantile(
            rs.map((r) => r.ch.read_bytes),
            0.5,
          ) /
            1048576) *
            10,
        ) / 10,
      ch_cpu_s:
        Math.round(
          quantile(
            rs.map((r) => r.ch.cpu_s * 1000),
            0.5,
          ),
        ) / 1000,
      each: rs[0].each
        ? Object.fromEntries(
            Object.keys(rs[0].each).map((k) => [
              k,
              quantile(
                rs.map((r) => r.each?.[k] ?? 0),
                0.5,
              ),
            ]),
          )
        : undefined,
    });
  }
  for (const o of out) console.log(JSON.stringify(o));
}

const command = args[0];
if (command === 'summary') summary();
else if (command === 'once') {
  const version = arg('--version', 'new');
  const suite = arg('--suite', 'dashboard');
  const user = new User(version);
  await user.login();
  if (suite === 'dashboard') console.log(JSON.stringify(await measured(version, () => loadDashboard(user))));
  else
    for (const [name, page] of PAGES) {
      const r = await measured(version, () => page(user));
      delete r.responses;
      console.log(name, JSON.stringify(r));
    }
  user.close();
} else if (command === 'run') {
  const runs = Number(arg('--runs', 5));
  const suites = arg('--suites', 'dashboard,pages,concurrent').split(',');
  for (let run = 1; run <= runs; run++) {
    for (const suite of suites) {
      // Alternate which version goes first, so neither always follows the other's idle time.
      const order = run % 2 ? ['old', 'new'] : ['new', 'old'];
      for (const version of order) await SUITES[suite](version, run);
    }
  }
} else {
  console.error('usage: measure.mjs run|once|summary');
  process.exit(2);
}
