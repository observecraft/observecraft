// Every imported integration dashboard (server/seeds/integrations/core/dashboards.ndjson.gz) through the panel engine:
// each widget's queries, formulas and functions run against stub data (logs and security signals through the
// in-browser mocks), and widget types the renderer can't draw are counted. Prints the gaps by count; fails when more
// widgets than `ALLOWED` can't be drawn. `OC_DASHBOARD_REPORT=<file>` writes every failing widget there.
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { gunzipSync } from 'node:zlib';

const require = createRequire(new URL('../web/package.json', import.meta.url));
const { outputFiles } = require('esbuild').buildSync({
  stdin: {
    contents: `export { queryIntegrationPanel } from './api/integrations/panels'; export { isEventPanel, panelSupport } from './api/integrations/panelSupport';`,
    resolveDir: fileURLToPath(new URL('../web/src', import.meta.url)),
    loader: 'ts',
  },
  bundle: true,
  platform: 'node',
  format: 'esm',
  write: false,
  loader: { '.json': 'json' },
  // Mock mode: logs and signals come from the in-browser mocks.
  define: { 'import.meta.env': '{}' },
  logLevel: 'silent',
});
const { isEventPanel, queryIntegrationPanel, panelSupport } = await import(`data:text/javascript;base64,${Buffer.from(outputFiles[0].text).toString('base64')}`);

/** Widgets that can't be drawn today: the test fails when there are more (lower it as the engine catches up). */
const ALLOWED = 125;

const range = { from: 1_792_929_600_000, to: 1_792_933_200_000 };
// Two series per query, a point a minute: formulas, functions and aggregations run on real numbers.
const source = {
  catalog: async () => [],
  timeseries: async ({ queries, points }) =>
    queries.flatMap(() => ['host:a', 'host:b'].map((name, k) => ({ name, points: Array.from({ length: Math.min(points ?? 60, 60) }, (_, i) => [range.from + i * 60_000, 10 + i + k]) }))),
};
/** A template variable at its default (`*`), as a dashboard opened without a selection. */
const resolve = (query) => query.replace(/\$[\w-]+(?:\.value)?/g, '*');

const dashboards = gunzipSync(readFileSync(new URL('../server/seeds/integrations/core/dashboards.ndjson.gz', import.meta.url)))
  .toString('utf8')
  .trim()
  .split('\n')
  .flatMap((line) => JSON.parse(line).dashboards);

/** An error message as a gap: the same problem with different names counts once. */
const gap = (message) =>
  message
    .replace(/: .*$/s, (rest) => (/^: (Unsupported|.* is not supported)/.test(rest) ? rest : ''))
    .replace(/\b\d+(\.\d+)?\b/g, 'N')
    .slice(0, 120);

test('every integration dashboard widget draws with the panel engine', async () => {
  const gaps = new Map();
  const failing = [];
  let widgets = 0;
  const walk = async (ws, dashboard) => {
    for (const w of ws) {
      const d = w.definition;
      if (d.widgets) await walk(d.widgets, dashboard);
      if (d.type !== 'integration') continue;
      widgets++;
      const o = d.original;
      // Lists and event panels read other APIs; metric panels run their queries.
      const metric = !panelSupport(o) && !isEventPanel(o) && !['free_text', 'hostmap', 'list_stream', 'topology_map'].includes(o.type);
      const problem =
        panelSupport(o) ??
        (metric
          ? await queryIntegrationPanel(o, resolve, range, source).then(
              () => null,
              (e) => e.message,
            )
          : null);
      if (problem) {
        gaps.set(gap(problem), (gaps.get(gap(problem)) ?? 0) + 1);
        failing.push(`${dashboard.id} · ${o.title ?? o.type}: ${problem}`);
      }
    }
  };
  await Promise.all(dashboards.map((dashboard) => walk(dashboard.widgets, dashboard)));
  const ranked = [...gaps].sort((a, b) => b[1] - a[1]);
  const summary = `${widgets - failing.length} of ${widgets} integration widgets draw; ${failing.length} don't:\n${ranked.map(([g, n]) => `  ${n} ${g}`).join('\n')}`;
  console.log(summary);
  if (process.env.OC_DASHBOARD_REPORT) writeFileSync(process.env.OC_DASHBOARD_REPORT, `${summary}\n\n${failing.join('\n')}\n`);
  assert.ok(failing.length <= ALLOWED, summary);
});
