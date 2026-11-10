// Mock mode (make web-mock) draws every widget of the Kubernetes tools' dashboards (Vault, Gatekeeper, External Secrets,
// Argo CD, Traefik Mesh, cert-manager, ingress-nginx, …), the datastores' (PostgreSQL, PgBouncer, Redis, ClickHouse) and
// Kafka's (Kafka, Strimzi, Confluent Platform): their metrics, service checks, logs, security signals and Data Streams
// topologies, template variables at their defaults.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

// The panel code and the mock behind it import without extensions and read import.meta.env, as the web build does: bundle them.
const require = createRequire(new URL('../web/package.json', import.meta.url));
const { outputFiles } = require('esbuild').buildSync({
  stdin: {
    contents: [
      "export { queryIntegrationPanel } from './api/integrations/panels';",
      "export { searchLogPage, logPatterns } from './api/client';",
      "export { resolveQuery, resolveSearchQuery } from './lib/metricQuery';",
      "export { hostmapQuery } from './api/integrations/widgetData';",
      "export { checkStatusTiles } from './api/integrations/checkStatus';",
      "export { evaluateGroups } from './api/mock/metricsEngine';",
      "export { queryTopology } from './api/integrations/topology';",
    ].join('\n'),
    resolveDir: fileURLToPath(new URL('../web/src', import.meta.url)),
    loader: 'ts',
  },
  bundle: true,
  platform: 'node',
  format: 'esm',
  write: false,
  define: { 'import.meta.env': '{}' },
  logLevel: 'error',
});
const web = await import(`data:text/javascript;base64,${Buffer.from(outputFiles[0].text).toString('base64')}`);
const mock = JSON.parse(readFileSync(new URL('../web/src/api/integrations/coreDashboards.generated.json', import.meta.url), 'utf8'));

/** Widgets whose data mock mode can't have, and why. */
const KNOWN = new Map([
  // Hosts tagged field:apps:argocd by their owner: a tag of the user's own, not one the integration sends.
  ['Host Reporting Argo CD Metrics by Cluster', 'user tag'],
  // Redis's pipeline maps its severities (. - * #) to debug, info and warn: no log of its is an error, in Datadog too.
  ['Redis - Overview / Error Logs', 'no error status'],
  // Upstream's query reads `short_image::*kube-controller-manager` (a doubled colon): no tag value matches it.
  ['Kube Controller Manager memory usage vs requests', 'upstream query typo'],
]);

const widgets = (items) => items.flatMap((w) => (w.definition.type === 'group' ? widgets(w.definition.widgets) : [w.definition]));

async function hasData(d, def) {
  const range = { from: Date.now() - 3_600_000, to: Date.now() };
  const resolve = (q) => web.resolveQuery(q, d.templateVariables, {});
  if (def.type === 'log_stream') return (await web.searchLogPage({ range, query: web.resolveSearchQuery(def.query, d.templateVariables, {}) }, 50)).logs.length > 0;
  if (def.type !== 'integration' || def.original.type === 'free_text') return true;
  const o = def.original;
  if (o.type === 'list_stream' && o.requests?.[0]?.query?.data_source === 'logs_pattern_stream')
    return (await web.logPatterns({ range, query: web.resolveSearchQuery(o.requests[0].query.query_string, d.templateVariables, {}) })).length > 0;
  if (o.type === 'topology_map') return (await web.queryTopology(o.requests?.[0], resolve, range)).edges.length > 0;
  if (o.type === 'hostmap') return (await web.queryIntegrationPanel({ type: 'query_table', requests: [{ q: web.hostmapQuery(o) }] }, resolve, range)).length > 0;
  const series = await web.queryIntegrationPanel(o, resolve, range);
  if (o.type === 'check_status') return web.checkStatusTiles(o, series).length > 0;
  return series.some((s) => s.points.length > 0);
}

for (const title of [
  'Vault - Overview (OpenMetricsV2)',
  'Vault - Overview (Legacy)',
  'Gatekeeper Overview',
  'External Secrets Operator - Overview',
  'Argo CD Overview',
  'Traefik Mesh Overview',
  'Cert Manager Overview',
  'Postgres - Metrics',
  'Postgres - Overview',
  'PgBouncer - Metrics',
  'Redis - Overview',
  'ClickHouse Overview',
  'Kafka, Zookeeper and Kafka Consumer Overview',
  'Kafka KRaft Overview',
  'Strimzi Overview',
  'Confluent Platform Overview',
  'Kubernetes NGINX Ingress Controller - Overview',
  'Kubernetes API Server Overview',
  'Kubernetes Scheduler Overview',
  'Kubernetes Controller Manager Overview',
  'CoreDNS',
  'CoreDNS [V2]',
  'KEDA Overview',
  'Karpenter Overview',
  'Temporal Server Overview',
  'vLLM Overview',
  'LEGACY - DCGM Exporter (Nvidia GPU Monitoring) Overview',
]) {
  test(`every widget of ${title} has data in mock mode`, async () => {
    const d = mock.dashboards.find((x) => x.title === title);
    assert.ok(d, title);
    const empty = [];
    for (const def of widgets(d.widgets)) {
      if (KNOWN.has(def.title) || KNOWN.has(`${title} / ${def.title}`)) continue;
      try {
        if (!(await hasData(d, def))) empty.push(def.title);
      } catch (error) {
        empty.push(`${def.title}: ${error.message}`);
      }
    }
    assert.deepEqual(empty, []);
  });
}

test('a mock integration has as many instances of a metric as the tags its queries read tell apart', async () => {
  const d = mock.dashboards.find((x) => x.title === 'Argo CD Overview');
  const range = { from: Date.now() - 3_600_000, to: Date.now() };
  const resolve = (q) => web.resolveQuery(q, d.templateVariables, {});
  const controllers = widgets(d.widgets).find((w) => w.title === 'App Controllers').original;
  const [series] = await web.queryIntegrationPanel(controllers, resolve, range);
  assert.equal(series.points.at(-1)[1], 4, 'one application controller per host, not one per gRPC service the API server answers');
});

test('the mock reads a tag taking one of several values (code:(4* OR 5*)) as the server does', () => {
  const range = { from: Date.now() - 3_600_000, to: Date.now() };
  const codes = (q) =>
    web
      .evaluateGroups(q, range, 10)
      .map((g) => g.tags.code)
      .sort();
  const all = codes('sum:traefik_mesh.entrypoint.requests.count{*} by {code}');
  const errors = codes('sum:traefik_mesh.entrypoint.requests.count{code:(4* OR 5*)} by {code}');
  assert.ok(errors.length > 0 && errors.length < all.length, `${errors} of ${all}`);
  assert.ok(errors.every((c) => /^[45]/.test(c)));
  assert.deepEqual(
    codes('sum:traefik_mesh.entrypoint.requests.count{!code:(4* OR 5*)} by {code}'),
    all.filter((c) => !/^[45]/.test(c)),
  );
});

test('running totals the dashboards take the change of (diff, per_minute, derivative) only grow in the mock', () => {
  const range = { from: Date.now() - 3_600_000, to: Date.now() };
  for (const metric of ['redis.net.rejected', 'redis.keys.evicted', 'clickhouse.events.LogError.total']) {
    const groups = web.evaluateGroups(`sum:${metric}{*} by {host}`, range, 60);
    assert.ok(groups.length > 0, metric);
    for (const g of groups) {
      const values = g.points.map((p) => p[1]);
      assert.ok(
        values.every((v, i) => i === 0 || v >= values[i - 1]),
        `${metric} ${JSON.stringify(g.tags)}: ${values.slice(0, 6)}`,
      );
      assert.ok(values.at(-1) > values[0], metric);
    }
  }
});

test('a Data Streams topology widget narrows the map to its queue and environment', async () => {
  const range = { from: Date.now() - 3_600_000, to: Date.now() };
  const d = mock.dashboards.find((x) => x.title === 'Kafka KRaft Overview');
  const def = widgets(d.widgets).find((w) => w.original?.type === 'topology_map').original;
  const all = await web.queryTopology(def.requests[0], (q) => web.resolveQuery(q, d.templateVariables, {}), range);
  const orders = await web.queryTopology(def.requests[0], (q) => web.resolveQuery(q, d.templateVariables, { topic: 'orders.created', env: 'prod' }), range);
  const topics = (g) => g.nodes.filter((n) => n.kind === 'topic').map((n) => n.name);
  assert.ok(topics(all).length > 1, 'every Kafka topic without a topic chosen');
  assert.deepEqual(topics(orders), ['orders.created']);
  assert.ok(
    orders.edges.every((e) => e.source === 'topic:kafka:orders.created' || e.target === 'topic:kafka:orders.created'),
    JSON.stringify(orders.edges.map((e) => [e.source, e.target])),
  );
  assert.ok(
    orders.nodes.some((n) => n.kind === 'service'),
    'with its producers and consumers',
  );
});
