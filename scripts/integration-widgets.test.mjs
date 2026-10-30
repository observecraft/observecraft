import assert from 'node:assert/strict';
import test from 'node:test';
import { convertDashboard } from './import-integration-dashboards.mjs';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { fills, formatStyle, matchFormat } from '../web/src/pages/dashboards/widgets/conditionalFormats.ts';

// widgetQueries.ts imports without extensions, as the web build does: bundle it first.
const require = createRequire(new URL('../web/package.json', import.meta.url));
const { outputFiles } = require('esbuild').buildSync({
  entryPoints: [fileURLToPath(new URL('../web/src/pages/dashboards/widgets/widgetQueries.ts', import.meta.url))],
  bundle: true,
  platform: 'node',
  format: 'esm',
  write: false,
});
const { monitorQuery, widgetMetrics, widgetQueries } = await import(`data:text/javascript;base64,${Buffer.from(outputFiles[0].text).toString('base64')}`);

test('conditional formats apply the first matching condition, as upstream widgets do', () => {
  const formats = [
    { comparator: '<', value: 0, palette: 'black_on_light_green' },
    { comparator: '>=', value: 40, palette: 'white_on_yellow' },
    { comparator: '>=', value: 0, palette: 'black_on_light_yellow' },
  ];
  assert.equal(matchFormat(-3, formats)?.palette, 'black_on_light_green');
  assert.equal(matchFormat(52, formats)?.palette, 'white_on_yellow');
  assert.equal(matchFormat(12, formats)?.palette, 'black_on_light_yellow');
  assert.equal(matchFormat(undefined, formats), undefined);
});

test('palettes fill the cell or only color the text', () => {
  assert.deepEqual(formatStyle('white_on_red'), { background: 'var(--cf-red)', color: 'var(--text-inverse)' });
  assert.deepEqual(formatStyle('black_on_light_yellow'), { background: 'var(--cf-light-yellow)', color: 'var(--text)' });
  assert.deepEqual(formatStyle('green_on_white'), { color: 'var(--cf-green)' });
  assert.equal(formatStyle('custom_text'), undefined);
  assert.ok(fills('white_on_green') && !fills('red_on_white') && !fills('custom_bg'));
});

test('plain group colors import as pale headers, vivid ones as saturated', () => {
  const group = (background_color) => ({ definition: { type: 'group', title: 'G', background_color, layout_type: 'ordered', widgets: [] }, layout: { x: 0, y: 0, width: 12, height: 4 } });
  const doc = { title: 'T', layout_type: 'ordered', widgets: [group('purple'), group('vivid_blue'), group(undefined)] };
  const [pale, vivid, none] = convertDashboard(doc, 'kubernetes', 'assets/dashboards/t.json', 'x').widgets.map((w) => w.definition);
  assert.equal(pale.color, 'purple');
  assert.equal(pale.light, true);
  assert.equal(vivid.color, 'blue');
  assert.equal(vivid.light, undefined);
  assert.equal(none.light, undefined);
});

test('a widget becomes a monitor query: plain arithmetic between its queries, else its first query', () => {
  const resolve = (q) => q.replace('$cluster', 'kube_cluster_name:shop-prod');
  const integration = (formula) => ({
    type: 'integration',
    original: {
      type: 'query_value',
      requests: [
        {
          queries: [
            { name: 'query1', data_source: 'metrics', query: 'sum:kubernetes.cpu.requests{$cluster}' },
            { name: 'query2', data_source: 'metrics', query: 'sum:kubernetes.cpu.limits{$cluster}' },
          ],
          formulas: [{ formula }],
        },
      ],
    },
  });
  assert.equal(
    monitorQuery(widgetQueries(integration('query1 / query2 * 100')), resolve),
    'sum:kubernetes.cpu.requests{kube_cluster_name:shop-prod} / sum:kubernetes.cpu.limits{kube_cluster_name:shop-prod} * 100',
  );
  assert.equal(monitorQuery(widgetQueries(integration('exclude_null(query1)')), resolve), 'sum:kubernetes.cpu.requests{kube_cluster_name:shop-prod}');
  assert.deepEqual(widgetMetrics(widgetQueries(integration('query1'))), ['kubernetes.cpu.requests', 'kubernetes.cpu.limits']);
  assert.equal(monitorQuery(widgetQueries({ type: 'note', content: '', background: 'white', align: 'left', fontSize: 'md' }), resolve), undefined);
  assert.equal(monitorQuery(widgetQueries({ type: 'toplist', request: { query: 'avg:system.load.1{*} by {host}' }, aggregator: 'avg', limit: 10 }), resolve), 'avg:system.load.1{*} by {host}');
});

const catalogModule = require('esbuild').buildSync({
  entryPoints: [fileURLToPath(new URL('../web/src/pages/metrics/catalog.ts', import.meta.url))],
  bundle: true,
  platform: 'node',
  format: 'esm',
  write: false,
});
const { metricOrigin, typeGroup, queryableTags } = await import(`data:text/javascript;base64,${Buffer.from(catalogModule.outputFiles[0].text).toString('base64')}`);

test('a metric reads as coming from the product that generated it', () => {
  const logs = new Set(['checkout.errors']);
  assert.equal(metricOrigin({ name: 'system.cpu.user', kind: 'gauge' }, logs), 'Agent');
  assert.equal(metricOrigin({ name: 'kubernetes.cpu.usage.total', kind: 'gauge' }, logs), 'Agent integrations');
  assert.equal(metricOrigin({ name: 'http.server.request.duration.p95', kind: 'histogram' }, logs), 'OpenTelemetry');
  assert.equal(metricOrigin({ name: 'trace.requests', kind: 'span' }, logs), 'APM');
  assert.equal(metricOrigin({ name: 'checkout.errors.p95', kind: 'histogram' }, logs), 'Logs');
  assert.equal(metricOrigin({ name: 'checkout.error_ratio', kind: 'derived' }, logs), 'Derived');
  assert.equal(typeGroup('exponential_histogram'), 'Distributions');
  const rules = [{ enabled: true, kind: 'indexing', metricPattern: 'kubernetes.*', tags: ['env'], action: 'allow_tags', id: '1', name: 'k8s', metricsImpacted: 0, updatedAt: 0, updatedBy: '' }];
  assert.deepEqual(queryableTags('kubernetes.cpu.limits', ['env', 'pod_name'], rules), ['env']);
  assert.deepEqual(queryableTags('system.load.1', ['env', 'host'], rules), ['env', 'host']);
});
