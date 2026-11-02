// The AWS, Azure and Google Cloud overview dashboards (scripts/cloud-dashboards.mjs).
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import test from 'node:test';
import { CLOUDS, buildDashboards } from './cloud-dashboards.mjs';

const seed = (name) => JSON.parse(readFileSync(new URL(`../server/seeds/${name}`, import.meta.url), 'utf8'));
const dashboards = buildDashboards();

/** Every widget of a dashboard with its container's widgets (for overlap checks). */
function containers(widgets, out = []) {
  out.push(widgets);
  for (const w of widgets) if (w.definition.type === 'group') containers(w.definition.widgets, out);
  return out;
}
const all = (widgets) => containers(widgets).flat();
const queries = (d) => all(d.widgets).flatMap((w) => (w.definition.type === 'integration' ? (w.definition.original.requests ?? []).flatMap((r) => r.queries ?? []) : []));
const metricsOf = (d) => queries(d).flatMap((q) => [...(q.query ?? '').matchAll(/^(?:[a-z]+:)?([a-z0-9_.]+)\{/g)].map((m) => m[1]));

test('one overview per cloud, installed as integration dashboards', () => {
  assert.deepEqual(
    dashboards.map((d) => [d.id, d.kind, d.logo]),
    CLOUDS.map((c) => [`${c}-overview`, 'integration', c]),
  );
  for (const d of dashboards) {
    assert.equal(d.source.repository, 'observecraft');
    assert.ok(metricsOf(d).length >= 25, `${d.id} charts the cloud's services`);
  }
});

test('the metrics are the ones our crawlers publish', () => {
  const aws = new Set(
    seed('aws_integration.json').namespaces.map((n) =>
      n.namespace
        .toLowerCase()
        .replace(/^aws\//, '')
        .replace(/[^a-z0-9]+/g, '_'),
    ),
  );
  const azure = seed('azure_integration.json').resource_types.map((t) => t.prefix);
  const gcp = new Set(seed('gcp_integration.json').services.map((s) => s.short));
  for (const d of dashboards)
    for (const m of metricsOf(d)) {
      const [provider, service, ...rest] = m.split('.');
      assert.ok(rest.length, `${d.id}: ${m} has a metric name`);
      if (provider === 'aws') {
        assert.ok(aws.has(service), `${d.id}: ${m} is in a namespace we crawl`);
        assert.match(rest.join('.'), /^[a-z0-9_]+(\.(sum|maximum))?$/, `${d.id}: ${m} is a CloudWatch metric with one of our statistics`);
      } else if (provider === 'azure') {
        assert.ok(
          azure.some((p) => m.startsWith(`${p}.`)),
          `${d.id}: ${m} is a resource type we crawl`,
        );
        assert.match(rest.join('.'), /^[a-z0-9_]+(\.(total|maximum))?$/, `${d.id}: ${m} has one of our aggregations`);
      } else {
        assert.equal(provider, 'gcp', `${d.id}: ${m}`);
        assert.ok(gcp.has(service), `${d.id}: ${m} is a service we crawl`);
      }
    }
});

test('queries use only the dashboard’s template variables', () => {
  for (const d of dashboards) {
    const vars = new Set(d.templateVariables.map((v) => v.name));
    assert.ok(vars.size >= 2, `${d.id} filters by account and region`);
    for (const q of queries(d)) for (const [, v] of (q.query ?? q.query_string ?? '').matchAll(/\$([a-z_]+)/g)) assert.ok(vars.has(v), `${d.id}: $${v}`);
  }
});

test('widgets fit the 12-column grid without overlapping', () => {
  for (const d of dashboards)
    for (const ws of containers(d.widgets)) {
      for (const w of ws) assert.ok(w.layout.x >= 0 && w.layout.x + w.layout.w <= 12 && w.layout.h > 0, `${d.id}: ${w.id} fits`);
      for (const [i, a] of ws.entries())
        for (const b of ws.slice(i + 1)) {
          const [p, r] = [a.layout, b.layout];
          assert.ok(p.x + p.w <= r.x || r.x + r.w <= p.x || p.y + p.h <= r.y || r.y + r.h <= p.y, `${d.id}: ${a.id} overlaps ${b.id}`);
        }
    }
});

test('banners and service tiles are our artwork', () => {
  for (const d of dashboards) {
    const images = all(d.widgets).filter((w) => w.definition.type === 'image');
    assert.ok(
      images.some((w) => w.definition.src === `/images/integrations/${d.logo}-banner.svg`),
      `${d.id} has its banner`,
    );
    assert.equal(images.filter((w) => w.definition.src?.startsWith('/images/integrations/tiles/')).length, 6, `${d.id} shows six service tiles`);
    for (const w of images) assert.ok(existsSync(new URL(`../web/public${w.definition.src}`, import.meta.url)), w.definition.src);
  }
});

test('the seeds are up to date with the generator', () => {
  for (const d of dashboards) assert.deepEqual(seed(`integrations/${d.logo}/dashboards.json`), [d], `${d.id}: run node scripts/cloud-dashboards.mjs`);
});

test('mock mode has data for every metric the dashboards chart', async () => {
  const { CLOUD_METRICS } = await import('../web/src/api/mock/cloudMetrics.ts');
  for (const d of dashboards) for (const m of metricsOf(d)) assert.ok(CLOUD_METRICS[m], `${d.id}: mock/cloudMetrics.ts defines ${m}`);
});

test('notes and query values keep their natural height', () => {
  for (const d of dashboards)
    for (const w of all(d.widgets)) {
      if (w.definition.type === 'note' && w.definition.align === 'center' && !w.definition.content.includes('starting point')) assert.equal(w.layout.h, 2, `${d.id}: ${w.id} is a one-line note`);
      if (w.definition.type === 'note' && w.definition.align === 'left') assert.equal(w.layout.h, 3, `${d.id}: ${w.id} is a tip beside query values`);
      if (w.definition.type === 'integration' && w.definition.original.type === 'query_value') assert.equal(w.layout.h, 3, `${d.id}: ${w.id}`);
    }
});

test('the metrics’ units are seeded for the server and match the mock', async () => {
  const { CLOUD_UNITS, unitsCsv } = await import('./cloud-dashboards.mjs');
  const { CLOUD_METRICS } = await import('../web/src/api/mock/cloudMetrics.ts');
  assert.equal(readFileSync(new URL('../server/seeds/integrations/cloud/units.csv', import.meta.url), 'utf8'), unitsCsv(), 'run node scripts/cloud-dashboards.mjs');
  const known = ['byte', 'percent', 'fraction', 'millisecond', 'second'];
  for (const [name, unit] of Object.entries(CLOUD_UNITS)) {
    assert.ok(known.includes(unit), `${name}: ${unit}`);
    assert.equal(CLOUD_METRICS[name]?.unit, unit, `mock ${name}`);
  }
  // Charts take units from the metrics: no custom unit except on a formula of unitless counts.
  for (const d of dashboards)
    for (const w of all(d.widgets).filter((w) => w.definition.original?.custom_unit)) assert.match(w.definition.original.requests[0].formulas[0].formula, /\//, `${d.id}: ${w.id}`);
});
