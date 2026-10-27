import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { convertDashboard, PINNED_REVISION } from './import-integration-dashboards.mjs';
import { formula, metricPlan, preparePanelQuery } from '../web/src/api/integrations/evaluate.ts';
import { resolveQuery } from '../web/src/lib/metricQuery.ts';

test('explicit template values resolve StatefulSet filters and preserve the selected workload', () => {
  const variables = [{ name: 'kube_stateful_set', prefix: 'kube_stateful_set', default: '*' }];
  const query = 'sum:kubernetes.cpu.requests{kube_stateful_set:$kube_stateful_set.value}';
  assert.equal(resolveQuery(query, variables, {}), 'sum:kubernetes.cpu.requests{kube_stateful_set:*}');
  assert.equal(resolveQuery(query, variables, { kube_stateful_set: 'catalog-store' }), 'sum:kubernetes.cpu.requests{kube_stateful_set:catalog-store}');
});

const s = (name, values) => [{ name, points: values.map((v, i) => [i * 1000, v]) }];
test('formula precedence, scaling, missing points and division by zero', () => {
  assert.deepEqual(formula('(query1 / query2) * 100', { query1: s('*', [2, 4]), query2: s('*', [4, 0]) })[0].points, [[0, 50]]);
  assert.deepEqual(formula('1 - query1 * 2', { query1: s('*', [0.2]) })[0].points, [[0, 0.6]]);
  assert.throws(() => formula('globalThis.process.exit()', {}));
});
test('group alignment does not combine unrelated pods', () => {
  const inputs = { a: [...s('pod:a', [4]), ...s('pod:b', [8])], b: [...s('pod:b', [2]), ...s('pod:a', [4])] };
  assert.deepEqual(
    formula('a / b', inputs).map((s) => s.points[0][1]),
    [1, 4],
  );
  assert.equal(formula('count_nonzero(a)', inputs)[0].points[0][1], 2);
});
test('query modifiers remain explicit and unknown modifiers are rejected', () => {
  const p = metricPlan('sum:kubernetes.cpu.requests{$cluster} by {kube_node}.fill(null).rollup(avg, 20)');
  assert.deepEqual(
    p.operations.map((o) => o.name),
    ['fill', 'rollup'],
  );
  assert.equal(metricPlan('sum:a{*}.weighted()').operations[0].name, 'weighted');
  assert.throws(() => metricPlan('sum:a{*}.unsupported()'), /unsupported/);
});
test('import preserves upstream requests, formulas and thresholds without image assets', () => {
  const definition = {
    type: 'query_value',
    title: 'Available replicas',
    requests: [
      {
        queries: [{ name: 'query1', data_source: 'metrics', query: 'sum:kubernetes_state.deployment.replicas_available{*}' }],
        formulas: [{ formula: 'query1 / 2' }],
        conditional_formats: [{ comparator: '>', value: 2, palette: 'white_on_red' }],
      },
    ],
  };
  const d = convertDashboard(
    { title: 'Deployments', widgets: [{ id: 1, definition, layout: { x: 0, y: 0, width: 6, height: 2 } }, { definition: { type: 'image', url: 'https://example.com/proprietary.png' } }] },
    'kubernetes',
    'assets/dashboards/test.json',
    PINNED_REVISION,
  );
  assert.deepEqual(d.widgets[0].definition.original, definition);
  assert.equal(d.widgets.length, 2);
  assert.equal(d.widgets[1].definition.src, '/images/dashboard-kubernetes-capacity.svg');
  assert.equal(d.widgets[0].layout.w, 6);
});
test('all 12 bundled dashboards contain real upstream panels and no embedded artwork', () => {
  const raw = readFileSync(new URL('../server/seeds/integrations/kubernetes/dashboards.json', import.meta.url), 'utf8');
  const docs = JSON.parse(raw);
  assert.equal(docs.length, 12);
  assert.equal(new Set(docs.map((d) => d.id)).size, 12);
  assert.ok(!raw.includes('data:image') && !raw.includes('banner_img'));
  for (const d of docs) {
    assert.equal(d.source.revision, PINNED_REVISION);
    assert.ok(d.widgets.length);
    assert.ok(raw.includes('kubernetes_state.deployment.replicas'));
  }
});

test('free layout retains chart heights instead of compressing grid units as pixels', () => {
  const d = convertDashboard(
    { title: 'ReplicaSets', layout_type: 'free', widgets: [{ id: 1, definition: { type: 'timeseries', requests: [] }, layout: { x: 0, y: 57, width: 59, height: 14 } }] },
    'kubernetes',
    'assets/dashboards/test.json',
    PINNED_REVISION,
  );
  assert.equal(d.widgets[0].layout.y, 57);
  assert.equal(d.widgets[0].layout.h, 14);
});

test('children fill their own group width, and headers do not collide with later groups', () => {
  const group = (id, x, y, width) => ({
    id,
    layout: { x, y, width, height: 3 },
    definition: { type: 'group', widgets: [{ id: id + 10, definition: { type: 'timeseries', requests: [] }, layout: { x: 0, y: 0, width, height: 3 } }] },
  });
  const d = convertDashboard({ title: 'Nodes', widgets: [group(1, 0, 0, 4), group(2, 4, 0, 8), group(3, 0, 3, 12)] }, 'kubernetes', 'assets/dashboards/test.json', PINNED_REVISION);
  assert.equal(d.widgets[0].definition.widgets[0].layout.w, 12);
  assert.equal(d.widgets[1].definition.widgets[0].layout.w, 12);
  assert.ok(d.widgets[2].layout.y >= d.widgets[0].layout.y + d.widgets[0].layout.h);
});

test('every bundled panel has usable height and stays inside its canvas without overlap', () => {
  const docs = JSON.parse(readFileSync(new URL('../server/seeds/integrations/kubernetes/dashboards.json', import.meta.url), 'utf8'));
  function check(widgets, cols) {
    for (let i = 0; i < widgets.length; i++) {
      const a = widgets[i].layout;
      assert.ok(a.h >= 4, `collapsed panel ${widgets[i].id}`);
      assert.ok(a.x >= 0 && a.x + a.w <= cols, `overflow ${widgets[i].id}`);
      for (const other of widgets.slice(i + 1)) {
        const b = other.layout;
        assert.ok(!(a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y), `overlap ${widgets[i].id} / ${other.id}`);
      }
      if (widgets[i].definition.type === 'group') check(widgets[i].definition.widgets, 12);
    }
  }
  for (const doc of docs) check(doc.widgets, doc.layoutType === 'free' ? 48 : 12);
});

test('top ranks complete grouped series without evaluating strings as code', () => {
  const inputs = { q: [...s('pod:a', [9, 1]), ...s('pod:b', [2, 8])] };
  assert.equal(formula("top(q, 1, 'last', 'desc')", inputs)[0].name, 'pod:b');
  assert.equal(formula('top(exclude_null(q), 1, "sum", "asc")', inputs).length, 1);
  assert.throws(() => formula("top(q, -1, 'last', 'desc')", inputs));
});

test('weighted reaches the shared backend plan instead of being silently discarded', () => {
  const result = preparePanelQuery('sum:a{*}.fill(null).weighted()', (q) => q, { from: 0, to: 60000 });
  assert.equal(result.text, 'sum:a{*}.weighted()');
  assert.throws(() => preparePanelQuery('avg:a{*}.weighted()', (q) => q, { from: 0, to: 60000 }));
});

test('capacity dashboard has distinct authored guidance and a local illustration', () => {
  const docs = JSON.parse(readFileSync(new URL('../server/seeds/integrations/kubernetes/dashboards.json', import.meta.url)));
  const capacity = docs.find((d) => d.id === 'kubernetes-kubernetes-cpu-memory');
  const notes = capacity.widgets.filter((w) => w.definition.type === 'note').map((w) => w.definition.content);
  assert.equal(new Set(notes).size, notes.length);
  assert.ok(capacity.widgets.some((w) => w.definition.type === 'image'));
  assert.ok(!JSON.stringify(docs).includes('Collect kubernetes metrics and use the template variables'));
});

test('exclude_null removes missing groups but preserves zero and punctuation in real tags', () => {
  const inputs = {
    q: [
      { name: 'kube_job:inventory-reconcile', tags: { kube_job: 'inventory-reconcile' }, points: [[0, 0]] },
      { name: 'kube_job:N/A', tags: { kube_job: 'N/A' }, points: [[0, 99]] },
      { name: 'kube_job:legacy', points: [[0, 7]] },
      { name: 'kube_job:N/A', points: [[0, 8]] },
      { name: 'kube_job:job, label:part:N/A', tags: { kube_job: 'job', label: 'part:N/A' }, points: [[0, 1]] },
    ],
  };
  const result = formula('exclude_null(q)', inputs);
  assert.equal(result.length, 3);
  assert.equal(result[0].points[0][1], 0);
});

test('CPU formula conversions carry units without modifying the original input', () => {
  const q = [{ name: '*', unit: 'n{core}', points: [[0, 2600000000]] }];
  const result = formula('q / 1000000000', { q });
  assert.equal(result[0].unit, '{core}');
  assert.equal(result[0].points[0][1], 2.6);
  assert.equal(q[0].points[0][1], 2600000000);
});
