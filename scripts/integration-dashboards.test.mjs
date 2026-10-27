import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { convertDashboard, metricUnits, PINNED_REVISION } from './import-integration-dashboards.mjs';
import { formula, metricPlan, preparePanelQuery, timeShifts } from '../web/src/api/integrations/evaluate.ts';
import { byteAxisInterval, formatMetricValue } from '../web/src/api/integrations/units.ts';
import { changeRows } from '../web/src/api/integrations/change.ts';
import { shades } from '../web/src/theme/shades.ts';
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
test('import preserves upstream requests, formulas and thresholds; linked images become our artwork', () => {
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
  assert.equal(d.widgets[1].definition.src, '/images/integrations/kubernetes-banner.svg');
  assert.equal(d.widgets[0].layout.w, 6);
});
test('all 12 bundled dashboards contain real upstream panels and no Datadog-hosted images', () => {
  const raw = readFileSync(new URL('../server/seeds/integrations/kubernetes/dashboards.json', import.meta.url), 'utf8');
  const docs = JSON.parse(raw);
  assert.equal(docs.length, 12);
  assert.equal(new Set(docs.map((d) => d.id)).size, 12);
  assert.ok(!raw.includes('/static/images') && !raw.includes('banner_img'));
  const images = [];
  const walk = (w) => (w.definition.type === 'image' && images.push(w.definition.src), (w.definition.widgets ?? []).forEach(walk));
  docs.forEach((d) => d.widgets.forEach(walk));
  assert.ok(images.length && images.every((src) => src.startsWith('data:image/') || src.startsWith('/images/integrations/')), 'only embedded or our own images');
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

test('metric units come from metadata.csv, quoted descriptions included', () => {
  const csv = [
    'metric_name,metric_type,interval,unit_name,per_unit_name,description,orientation,integration,short_name,curated_metric',
    'kubernetes.cpu.usage.total,gauge,,nanocore,,"The number of cores used by the host, in nanocores",0,kubelet,cpu usage,',
    'kubernetes.network.rx_bytes,gauge,,byte,second,The amount of bytes per second received,0,kubelet,network rx,',
    'kubernetes.pods.running,gauge,,,,The number of running pods,0,kubelet,running pods,',
  ].join('\n');
  assert.deepEqual(metricUnits(csv), [
    ['kubernetes.cpu.usage.total', 'nanocore'],
    ['kubernetes.network.rx_bytes', 'byte/second'],
  ]);
});

test('values read in their unit, compactly', () => {
  // Units as integrations-core's metadata.csv names them.
  assert.equal(formatMetricValue(1951856526.53, 'nanocore'), '1.95 cores');
  assert.equal(formatMetricValue(7e9, 'nanocore'), '7 cores');
  assert.equal(formatMetricValue(17503632530.29, 'byte'), '16.3 GiB');
  assert.equal(formatMetricValue(1234567, 'byte/second'), '1.18 MiB/s');
  assert.equal(formatMetricValue(42.5, 'percent'), '42.5%');
  assert.equal(formatMetricValue(0.85, 'fraction'), '85%');
  assert.equal(formatMetricValue(1500, 'millisecond'), '1.5 s');
  assert.equal(formatMetricValue(0.00025, 'second'), '250 µs');
  assert.equal(formatMetricValue(12.25, 'request/second'), '12.25 req/s');
  // No unit: large numbers are compacted, small ones kept.
  assert.equal(formatMetricValue(1234567), '1.23M');
  assert.equal(formatMetricValue(2.5e9), '2.5B');
  assert.equal(formatMetricValue(950), '950');
  assert.equal(formatMetricValue(0.125), '0.13');
});

test('element-wise formula functions', () => {
  const at = (values, step = 10_000) => [{ name: '*', points: values.map((v, i) => [i * step, v]) }];
  assert.deepEqual(formula('abs(a)', { a: at([-2, 3]) })[0].points, [
    [0, 2],
    [10_000, 3],
  ]);
  assert.deepEqual(formula('cumsum(a)', { a: at([1, 2, 3]) })[0].points, [
    [0, 1],
    [10_000, 3],
    [20_000, 6],
  ]);
  assert.deepEqual(formula('log10(a)', { a: at([100]) })[0].points, [[0, 2]]);
  assert.deepEqual(formula('clamp_max(a, 5)', { a: at([2, 9]) })[0].points, [
    [0, 2],
    [10_000, 5],
  ]);
  // A counter's change per second, minute and hour over 10s intervals.
  assert.deepEqual(formula('per_second(a)', { a: at([0, 50, 100]) })[0].points, [
    [10_000, 5],
    [20_000, 5],
  ]);
  assert.deepEqual(formula('per_minute(a)', { a: at([0, 50]) })[0].points, [[10_000, 300]]);
  assert.deepEqual(formula('per_hour(a)', { a: at([0, 1]) })[0].points, [[10_000, 360]]);
  // A reset counts from zero.
  assert.deepEqual(formula('monotonic_diff(a)', { a: at([5, 8, 2]) })[0].points, [
    [10_000, 3],
    [20_000, 2],
  ]);
});

test('time shifts become queries over earlier ranges', () => {
  assert.deepEqual(timeShifts('week_before(query1)'), { text: 'query1__shift604800', shifts: [{ name: 'query1__shift604800', query: 'query1', seconds: 604800 }] });
  assert.deepEqual(timeShifts('query1 - day_before(query1)'), { text: 'query1 - query1__shift86400', shifts: [{ name: 'query1__shift86400', query: 'query1', seconds: 86400 }] });
  assert.deepEqual(timeShifts('timeshift(query2, -3600) + month_before(query1)').shifts, [
    { name: 'query2__shift3600', query: 'query2', seconds: 3600 },
    { name: 'query1__shift2419200', query: 'query1', seconds: 2419200 },
  ]);
  assert.deepEqual(timeShifts('query1 / query2'), { text: 'query1 / query2', shifts: [] });
});

test('formula functions are only the known ones', () => {
  const a = [{ name: '*', points: [[0, 1]] }];
  assert.throws(() => formula('toString(a)', { a }), /not supported/);
  assert.throws(() => formula('constructor(a)', { a }), /not supported/);
  // A shift into the future isn't a comparison with the past.
  assert.deepEqual(timeShifts('timeshift(query1, 3600)').shifts, []);
});

test('change panels compare each group with its earlier value', () => {
  const v = (column, group, value) => ({ name: group, column, group, points: [[0, value]], display: 'line', dashed: false });
  const series = [
    v('query1__shift604800', 'kube_deployment:api', 4),
    v('query1', 'kube_deployment:api', 5),
    v('query1__shift604800', 'kube_deployment:web', 10),
    v('query1', 'kube_deployment:web', 8),
  ];
  assert.deepEqual(changeRows(series, { change_type: 'absolute', increase_good: true, order_by: 'change', order_dir: 'desc' }), [
    { group: 'kube_deployment:api', present: 5, past: 4, change: 1, good: true },
    { group: 'kube_deployment:web', present: 8, past: 10, change: -2, good: false },
  ]);
  const relative = changeRows(series, { change_type: 'relative', increase_good: false, order_by: 'name', order_dir: 'asc' });
  assert.deepEqual(
    relative.map((r) => [r.group, r.change, r.good]),
    [
      ['kube_deployment:api', 25, false],
      ['kube_deployment:web', -20, true],
    ],
  );
});

test('single-hue palettes are shades of one of our chart colors', () => {
  assert.deepEqual(shades('#3d7fe0', 3), ['#3d7fe0', '#77a5e9', '#b1ccf3']);
  assert.equal(shades('#3d7fe0', 1)[0], '#3d7fe0');
});

test('imported dashboards keep their notes, colors and embedded images; hosted artwork becomes ours', () => {
  const doc = {
    title: 'Kubernetes - Overview',
    layout_type: 'ordered',
    widgets: [
      {
        id: 1,
        definition: {
          type: 'group',
          title: 'Infrastructure',
          background_color: 'vivid_purple',
          banner_img: '/static/images/integration_dashboard/kubernetes_hero_2.jpeg',
          layout_type: 'ordered',
          widgets: [
            {
              id: 2,
              definition: { type: 'note', content: '## How to use\n\nPick a cluster.', background_color: 'vivid_blue', font_size: '24', text_align: 'center', show_tick: true },
              layout: { x: 0, y: 0, width: 6, height: 2 },
            },
          ],
        },
        layout: { x: 0, y: 0, width: 12, height: 4 },
      },
      { id: 3, definition: { type: 'image', url: 'data:image/png;base64,iVBORw0KGgo=', sizing: 'cover' }, layout: { x: 0, y: 4, width: 6, height: 3 } },
      { id: 4, definition: { type: 'image', url: '/static/images/integration_dashboard/kubernetes_hero_2.jpeg' }, layout: { x: 6, y: 4, width: 6, height: 3 } },
    ],
  };
  const d = convertDashboard(doc, 'kubernetes', 'assets/dashboards/kubernetes_dashboard.json', PINNED_REVISION);
  const group = d.widgets[0].definition;
  assert.equal(group.color, 'purple');
  assert.equal(group.compact, false);
  // The group's banner is our own artwork, above its widgets.
  assert.deepEqual(group.widgets[0].definition, { type: 'image', title: 'Kubernetes', src: '/images/integrations/kubernetes-banner.svg' });
  const note = group.widgets[1].definition;
  assert.deepEqual(note, { type: 'note', content: '## How to use\n\nPick a cluster.', background: 'blue', align: 'center', fontSize: 'lg', pointer: true });
  assert.ok(group.widgets[1].layout.y >= group.widgets[0].layout.y + group.widgets[0].layout.h, 'the note sits below the banner');
  assert.equal(d.widgets[1].definition.src, 'data:image/png;base64,iVBORw0KGgo=');
  assert.equal(d.widgets[2].definition.src, '/images/integrations/kubernetes-banner.svg');
});

test('note markdown: numbered steps are an ordered list, bullets an unordered one', async () => {
  const { markdownBlocks } = await import('../web/src/pages/dashboards/widgets/markdownBlocks.ts');
  assert.deepEqual(markdownBlocks('## Use\nIntro\n1. Select a timeframe\n2. Pick a cluster\n\n- one\n* two\n---'), [
    { type: 'heading', level: 2, text: 'Use' },
    { type: 'paragraph', text: 'Intro' },
    { type: 'list', ordered: true, items: ['Select a timeframe', 'Pick a cluster'] },
    { type: 'list', ordered: false, items: ['one', 'two'] },
    { type: 'rule' },
  ]);
});

test('one core is singular', () => {
  assert.equal(formatMetricValue(1e9, 'nanocore'), '1 core');
});

test('note markdown decodes the HTML entities integrations-core notes use, as text', async () => {
  const { markdownBlocks } = await import('../web/src/pages/dashboards/widgets/markdownBlocks.ts');
  assert.deepEqual(markdownBlocks('docs&nbsp;↗ &amp; &lt;b&gt;'), [{ type: 'paragraph', text: 'docs ↗ & <b>' }]);
});

test('kubernetes_state "cpu" is the same unit as cores; count units are plural except for one', () => {
  assert.equal(formatMetricValue(180, 'cpu'), '180 cores');
  assert.equal(formatMetricValue(14, 'node'), '14 nodes');
  assert.equal(formatMetricValue(1, 'node'), '1 node');
  assert.equal(formatMetricValue(0.4, 'packet/second'), '0.4 packets/s');
  assert.equal(formatMetricValue(3, 'bytes'), '3 B');
});

test('byte axes step in round binary units', () => {
  const GiB = 1024 ** 3;
  assert.equal(byteAxisInterval(651 * GiB), 200 * GiB);
  assert.equal(byteAxisInterval(48 * 1024 ** 2), 10 * 1024 ** 2);
  assert.equal(byteAxisInterval(0), undefined);
});

test('a group banner embedded in the upstream file is kept, and a hidden group title stays hidden', () => {
  const doc = {
    title: 'Kubernetes - Overview',
    layout_type: 'ordered',
    widgets: [
      {
        id: 1,
        definition: { type: 'group', title: 'Banner', show_title: false, banner_img: 'data:image/png;base64,iVBORw0KGgo=', layout_type: 'ordered', widgets: [] },
        layout: { x: 0, y: 0, width: 12, height: 2 },
      },
      { id: 2, definition: { type: 'group', title: 'Infrastructure', show_title: true, layout_type: 'ordered', widgets: [] }, layout: { x: 0, y: 4, width: 12, height: 4 } },
    ],
  };
  const d = convertDashboard(doc, 'kubernetes', 'assets/dashboards/kubernetes_dashboard.json', PINNED_REVISION);
  const banner = d.widgets[0].definition;
  assert.equal(banner.showTitle, false);
  assert.deepEqual(banner.widgets[0].definition, { type: 'image', title: 'Kubernetes', src: 'data:image/png;base64,iVBORw0KGgo=' });
  assert.equal(d.widgets[1].definition.showTitle, undefined);
  // No header: the group is as tall as its banner, not two rows taller.
  assert.equal(d.widgets[0].layout.h, 5);
});

test('a group without a header is only as tall as its widgets and padding', async () => {
  const { groupHeight } = await import('../web/src/pages/dashboards/tree.ts');
  const children = [{ layout: { x: 0, y: 0, w: 12, h: 5 } }];
  assert.equal(groupHeight(children), 8);
  assert.equal(groupHeight(children, false, false), 6);
});

test('infrastructure lists become Explorer searches: kind, template variables, alternatives, columns and sort', async () => {
  const { infrastructureList } = await import('../web/src/api/integrations/infrastructure.ts');
  const vars = { cluster: 'kube_cluster_name:aks-dev', namespace: '' };
  const resolveVar = (name) => vars[name] ?? '';
  const list = infrastructureList(
    {
      request_type: 'infrastructure_list',
      query: {
        data_source: 'infrastructure',
        resource_type: 'pod',
        query_string: '$cluster $namespace ((field#metadata.creationTimestamp:>=1hr AND NOT kube_condition_ready:true) OR pod_phase:failed)',
      },
      columns: [{ field: 'name' }, { field: 'status' }, { field: 'restarts' }, { field: 'cpu_usage_limits' }, { field: 'memory_usage_limits' }, { field: 'memory_usage' }],
      sort: { column: 'restarts', order: 'desc' },
    },
    resolveVar,
  );
  assert.equal(list.kind, 'pods');
  // (A AND NOT B) OR C, with the unsupported field# filter left out.
  assert.deepEqual(list.queries, ['kube_cluster_name:aks-dev -kube_condition_ready:true', 'kube_cluster_name:aks-dev pod_phase:failed']);
  assert.deepEqual(list.columns, ['name', 'status', 'restarts', 'cpu', 'memory']);
  assert.deepEqual(list.sort, { column: 'restarts', desc: true });

  const deployments = infrastructureList({ query: { data_source: 'infrastructure', resource_type: 'daemonSet', query_string: '$scope $cluster' }, columns: [] }, resolveVar);
  assert.equal(deployments.kind, 'daemonsets');
  assert.deepEqual(deployments.queries, ['kube_cluster_name:aks-dev']);
  assert.equal(deployments.columns, undefined, 'no columns: the kind’s default ones');
  assert.equal(deployments.sort, undefined);
});

test('infrastructure lists drop unresolved variables and match-all filters', async () => {
  const { infrastructureList } = await import('../web/src/api/integrations/infrastructure.ts');
  const list = infrastructureList({ query: { data_source: 'infrastructure', resource_type: 'pod', query_string: '$scope $cluster pod_phase:running' } }, (name) =>
    name === 'cluster' ? 'kube_cluster_name:*' : `$${name}`,
  );
  assert.deepEqual(list.queries, ['pod_phase:running']);
});

test('log queries count logs per facet value; throughput divides by the range', async () => {
  const { logRequest, logSeries, throughputs } = await import('../web/src/api/integrations/logs.ts');
  const query = {
    name: 'query1',
    data_source: 'logs',
    search: { query: 'kube_cluster_name:$kube_cluster_name.value AND source:(*kube* OR etcd)' },
    group_by: [{ facet: 'source', limit: 10 }],
    compute: { aggregation: 'count' },
  };
  const resolve = (q) => q.replace('$kube_cluster_name.value', 'aks-dev');
  assert.deepEqual(logRequest(query, resolve), { query: 'kube_cluster_name:aks-dev AND source:(*kube* OR etcd)', key: 'source', limit: 10 });
  assert.throws(() => logRequest({ ...query, compute: { aggregation: 'cardinality', metric: '@usr.id' } }, resolve), /count/);
  const range = { from: 0, to: 60_000 };
  assert.deepEqual(logSeries([{ value: 'etcd', count: 120 }], 'source', range), [{ name: 'source:etcd', points: [[60_000, 120]] }]);
  assert.deepEqual(throughputs('throughput(query1) * 2'), { text: 'query1__throughput * 2', names: ['query1'] });
});

test('a log pattern opens as a search for its service and longest fixed text', async () => {
  const { patternQuery } = await import('../web/src/pages/logs/patterns.ts');
  assert.equal(patternQuery({ pattern: 'GET /api/v1/orders/* * in *ms', service: 'order-service' }, 'env:prod'), 'env:prod service:order-service "GET /api/v1/orders/"');
  assert.equal(patternQuery({ pattern: '* *', service: 'billing' }, ''), 'service:billing');
  // Quotes can't be escaped in a search phrase: the longest piece between them is used.
  assert.equal(patternQuery({ pattern: 'say "hello" * now', service: '' }, ''), '"hello"');
});

test('a log search opens in a dashboard as a log stream widget', async () => {
  const { logStreamWidgets } = await import('../web/src/pages/dashboards/tree.ts');
  const [w] = logStreamWidgets('service:billing status:error');
  assert.equal(w.definition.type, 'log_stream');
  assert.equal(w.definition.query, 'service:billing status:error');
  assert.equal(w.definition.title, 'service:billing status:error');
  assert.deepEqual(w.layout, { x: 0, y: 0, w: 12, h: 8 });
  assert.equal(logStreamWidgets('')[0].definition.title, 'All logs');
});

test('log messages are shown without ANSI color codes', async () => {
  const { stripAnsi } = await import('../web/src/lib/format.ts');
  assert.equal(stripAnsi('INFO:\u001b[94m\u001b[1mGET\u001b[0m /metrics \u001b[32m200\u001b[0m'), 'INFO:GET /metrics 200');
  assert.equal(stripAnsi('plain [94m text'), 'plain [94m text');
});

test('view in context centers the time range on the log, never past now', async () => {
  const { centeredEnd } = await import('../web/src/lib/time.ts');
  const min = 60_000;
  assert.equal(centeredEnd(100 * min, 15 * min, 200 * min), 107.5 * min);
  assert.equal(centeredEnd(199 * min, 15 * min, 200 * min), 200 * min);
});

test('Datadog event text: the markdown between %%% markers', async () => {
  const { eventMarkdown } = await import('../web/src/lib/format.ts');
  assert.equal(eventMarkdown('%%% \n1 **Killing**: Stopping container build\n %%%'), '1 **Killing**: Stopping container build');
  assert.equal(eventMarkdown('plain text'), 'plain text');
});

test('template variables: first five, plus later ones in use; the rest behind +N', async () => {
  const { visibleVariables } = await import('../web/src/pages/dashboards/tree.ts');
  const vars = ['scope', 'cluster', 'namespace', 'deployment', 'daemonset', 'statefulset', 'replicaset', 'service', 'node'].map((name) => ({ name, prefix: name, default: '*' }));
  const folded = visibleVariables(vars, { node: 'aks-1' }, false);
  assert.deepEqual(
    folded.shown.map((v) => v.name),
    ['scope', 'cluster', 'namespace', 'deployment', 'daemonset', 'node'],
  );
  assert.equal(folded.hidden, 3);
  assert.equal(visibleVariables(vars, {}, true).shown.length, 9);
  assert.equal(visibleVariables(vars, {}, true).hidden, 0);
  assert.equal(visibleVariables(vars.slice(0, 6), {}, false).hidden, 0, 'one extra is shown rather than folded as +1');
});
