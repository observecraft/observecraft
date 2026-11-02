// Every integration's dashboards from integrations-core (scripts/import-integration-dashboards.mjs --all), installed for
// an org once the integration sends data.
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import test from 'node:test';
import { gunzipSync } from 'node:zlib';
import { bannerFor, convertDashboard } from './import-integration-dashboards.mjs';

const dir = new URL('../server/seeds/integrations/core/', import.meta.url);
const bundle = () =>
  gunzipSync(readFileSync(new URL('dashboards.ndjson.gz', dir)))
    .toString('utf8')
    .trim()
    .split('\n')
    .map((line) => JSON.parse(line));

test('one line per integration, with its metric prefix and dashboards', () => {
  const lines = bundle();
  assert.ok(lines.length >= 250, `${lines.length} integrations with dashboards`);
  const postgres = lines.find((l) => l.logo === 'postgres');
  assert.equal(postgres.prefix, 'postgresql.');
  assert.deepEqual(postgres.dashboards.map((d) => d.title).sort(), ['Postgres - Metrics', 'Postgres - Overview']);
  for (const l of lines) {
    for (const d of l.dashboards) {
      assert.equal(d.logo, l.logo, d.id);
      assert.equal(d.kind, 'integration', d.id);
    }
  }
  const ids = lines.flatMap((l) => l.dashboards.map((d) => d.id));
  assert.equal(new Set(ids).size, ids.length, 'dashboard ids are unique');
  assert.ok(!lines.some((l) => l.logo === 'kubernetes'), 'Kubernetes keeps its own bundle');
  assert.ok(
    lines.some((l) => l.logo === 'calico'),
    'manifest paths written ./assets/… are imported too',
  );
  assert.equal(lines.find((l) => l.logo === 'wazuh').dashboards.length, 8, 'file names with brackets or & are imported too');
  for (const id of ids) assert.match(id, /^[a-z0-9_-]+$/, id);
});

test('images Datadog hosts become the integration’s logo or banner, which exist', () => {
  const walk = (ws, out = []) => {
    for (const w of ws) {
      if (w.definition.type === 'image' && w.definition.src) out.push(w.definition.src);
      else if (w.definition.type === 'image') assert.ok(w.definition.logo, 'an image without artwork shows the logo');
      if (w.definition.widgets) walk(w.definition.widgets, out);
    }
    return out;
  };
  for (const l of bundle())
    for (const d of l.dashboards) for (const src of walk(d.widgets)) if (!src.startsWith('data:')) assert.ok(existsSync(new URL(`../web/public${src}`, import.meta.url)), `${d.id}: ${src}`);
  assert.equal(bannerFor('kubernetes'), '/images/integrations/kubernetes-banner.svg');
  assert.equal(bannerFor('postgres'), null, 'no banner of ours: the image shows the integration logo');
});

test('log streams and monitor lists become our native widgets', () => {
  const doc = {
    title: 'X',
    layout_type: 'ordered',
    widgets: [
      { id: 1, definition: { type: 'list_stream', title: 'Logs', requests: [{ response_format: 'event_list', query: { data_source: 'logs_stream', query_string: 'source:postgresql' } }] } },
      { id: 2, definition: { type: 'log_stream', title: 'Old logs', query: 'source:x status:error' } },
      { id: 3, definition: { type: 'manage_status', title: 'Monitors', query: 'tag:integration:postgres' } },
      { id: 4, definition: { type: 'list_stream', title: 'Events', requests: [{ response_format: 'event_list', query: { data_source: 'event_stream', query_string: 'x' } }] } },
    ],
  };
  const defs = convertDashboard(doc, 'postgres', 'assets/dashboards/x.json', 'sha').widgets.map((w) => w.definition);
  assert.deepEqual(defs[0], { type: 'log_stream', title: 'Logs', query: 'source:postgresql', limit: 50 });
  assert.deepEqual(defs[1], { type: 'log_stream', title: 'Old logs', query: 'source:x status:error', limit: 50 });
  assert.deepEqual(defs[2], { type: 'monitor_summary', title: 'Monitors', query: 'tag:integration:postgres' });
  assert.equal(defs[3].type, 'integration', 'other streams stay as they are');
});

test('groups are as tall as the dashboard draws them, so columns stay aligned', async () => {
  const { groupHeight } = await import('../web/src/pages/dashboards/tree.ts');
  const check = (ws, id) => {
    for (const w of ws)
      if (w.definition.type === 'group') {
        const d = w.definition;
        assert.ok(w.layout.h >= groupHeight(d.widgets, d.compact, d.showTitle !== false), `${id}: ${d.title} is ${w.layout.h} rows, drawn ${groupHeight(d.widgets, d.compact, d.showTitle !== false)}`);
        check(d.widgets, id);
      }
  };
  for (const l of bundle()) for (const d of l.dashboards) check(d.widgets, d.id);
});

test('no two widgets of a dashboard overlap', () => {
  const check = (ws, id) => {
    for (const [i, a] of ws.entries()) {
      for (const b of ws.slice(i + 1)) {
        const [p, r] = [a.layout, b.layout];
        assert.ok(p.x + p.w <= r.x || r.x + r.w <= p.x || p.y + p.h <= r.y || r.y + r.h <= p.y, `${id}: ${a.definition.title ?? a.id} overlaps ${b.definition.title ?? b.id}`);
      }
      if (a.definition.type === 'group') check(a.definition.widgets, id);
    }
  };
  for (const l of bundle()) for (const d of l.dashboards) if (d.layoutType !== 'free') check(d.widgets, d.id);
});

test('widgets side by side upstream stay side by side when a group above them grows', () => {
  const group = (id, x, y, width, height, inner) => ({
    id,
    definition: { type: 'group', title: `G${id}`, layout_type: 'ordered', widgets: [{ id: id * 10, definition: { type: 'note', content: 'x' }, layout: { x: 0, y: 0, width: 12, height: inner } }] },
    layout: { x, y, width, height },
  });
  // The right group of the first row needs a row more than upstream gives it.
  const doc = { title: 'X', layout_type: 'ordered', widgets: [group(1, 0, 0, 8, 5, 3), group(2, 8, 0, 4, 5, 4), group(3, 0, 5, 6, 5, 2), group(4, 6, 5, 6, 5, 2)] };
  const [a, b, c, d] = convertDashboard(doc, 'x', 'assets/dashboards/x.json', 'sha').widgets.map((w) => w.layout);
  assert.equal(a.y, b.y, 'first row');
  assert.equal(a.h, b.h, 'groups of a row are as tall as each other, so compaction keeps the next row aligned');
  assert.equal(c.y, d.y, 'second row stays one row');
  assert.ok(c.y >= Math.max(a.y + a.h, b.y + b.h), 'below the first row');
});

test('an ordered dashboard flows in its list order, as Datadog draws it (layout y is for its column view)', () => {
  const g = (id, layout, inner = 3) => ({
    id,
    definition: { type: 'group', title: `G${id}`, layout_type: 'ordered', widgets: [{ id: id * 10, definition: { type: 'note', content: 'x' }, layout: { x: 0, y: 0, width: 12, height: inner } }] },
    ...(layout ? { layout } : {}),
  });
  // Postgres - Overview: About | Current Activity, Resource Utilization (no layout), …, then Checkpoints | Logs at y 0.
  const doc = {
    title: 'X',
    layout_type: 'ordered',
    widgets: [
      g(1, { x: 0, y: 0, width: 8, height: 5 }),
      g(2, { x: 8, y: 0, width: 4, height: 5 }),
      g(3, null),
      g(4, { x: 0, y: 22, width: 12, height: 5, is_column_break: true }),
      g(5, { x: 0, y: 0, width: 6, height: 5 }),
      g(6, { x: 6, y: 0, width: 6, height: 5 }),
    ],
  };
  const [a, b, c, d, e, f] = convertDashboard(doc, 'x', 'assets/dashboards/x.json', 'sha').widgets.map((w) => w.layout);
  assert.deepEqual([a.y, b.y, b.x], [0, 0, 8], 'first row');
  assert.ok(c.y >= a.y + a.h && c.w === 12, 'a widget without layout takes a row');
  assert.ok(d.y >= c.y + c.h, 'then the next one');
  assert.ok(e.y >= d.y + d.h && e.y === f.y && f.x === 6, 'the last two share the last row');
});

test('the units of every integration’s metrics are seeded for the server', () => {
  const units = readFileSync(new URL('units.csv', dir), 'utf8');
  assert.match(units, /^postgresql\.replication_delay_bytes,byte$/m);
  assert.match(units, /^# Metric units from DataDog\/integrations-core@/);
  assert.ok(existsSync(new URL('LICENSE', dir)));
});

test('mock mode carries the dashboards of the integrations it shows installed, with their metrics’ units', () => {
  const mock = JSON.parse(readFileSync(new URL('../web/src/api/integrations/coreDashboards.generated.json', import.meta.url), 'utf8'));
  assert.ok(mock.dashboards.some((d) => d.title === 'Postgres - Overview'));
  assert.ok(mock.dashboards.length < 40, 'only a few, not the whole bundle');
  assert.equal(mock.units['postgresql.replication_delay_bytes'], 'byte');
});

test('mock mode has data for every metric those dashboards chart, tagged by their template variables', async () => {
  const { CORE_METRICS, CORE_SETS } = await import('../web/src/api/mock/coreMetrics.ts');
  const mock = JSON.parse(readFileSync(new URL('../web/src/api/integrations/coreDashboards.generated.json', import.meta.url), 'utf8'));
  const postgres = mock.dashboards.find((d) => d.title === 'Postgres - Overview');
  const text = JSON.stringify(postgres.widgets);
  for (const [, metric] of text.matchAll(/(?:avg|sum|max|min):(postgresql\.[a-z0-9_.]+)\{/g)) assert.ok(CORE_METRICS[metric], metric);
  const def = CORE_METRICS['postgresql.percent_usage_connections'];
  assert.ok(def && def.unit === 'fraction', 'units from metadata.csv');
  const tags = Object.keys(CORE_SETS[def.set][0]);
  for (const v of postgres.templateVariables.filter((v) => v.prefix)) assert.ok(tags.includes(v.prefix), `${v.name} is a tag of the mock entities`);
});
