// integrations-core's recommended monitors of PostgreSQL and Redis in the monitor template gallery
// (web/src/pages/monitors/<check>Templates.generated.ts, scripts/import-integration-dashboards.mjs --monitors).
import assert from 'node:assert/strict';
import test from 'node:test';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const require = createRequire(new URL('../web/package.json', import.meta.url));
const { outputFiles } = require('esbuild').buildSync({
  stdin: {
    contents: "export { TEMPLATES } from './pages/monitors/templates';\nexport { parseMetricQuery, formulaQueries } from './lib/metricQuery';",
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
const { TEMPLATES, formulaQueries } = await import(`data:text/javascript;base64,${Buffer.from(outputFiles[0].text).toString('base64')}`);

test('PostgreSQL’s connection saturation monitor, as integrations-core defines it', () => {
  const t = TEMPLATES.find((x) => x.id === 'postgres-connection-pool-is-reaching-saturation-point');
  assert.ok(t);
  assert.equal(t.query, 'avg:postgresql.percent_usage_connections{*}');
  assert.deepEqual([t.options.windowMinutes, t.options.comparator, t.options.thresholds.critical], [15, 'above', 0.9]);
  assert.match(t.name, /\[Postgres\]/);
});

test('the anomaly monitor (replication delay) is left out: anomaly detection is not supported', () => {
  assert.equal(
    TEMPLATES.some((x) => /replication-delay/.test(x.id)),
    false,
  );
});
