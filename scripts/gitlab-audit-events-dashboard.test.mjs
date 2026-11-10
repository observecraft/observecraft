// The GitLab Audit Events Overview dashboard (scripts/gitlab-audit-events-dashboard.mjs) over the integration's events.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { registerHooks } from 'node:module';
import test from 'node:test';
import { buildDashboard } from './gitlab-audit-events-dashboard.mjs';

// The web sources use extensionless imports and JSON without attributes (bundler resolution).
registerHooks({
  resolve(specifier, context, next) {
    try {
      return next(specifier, context);
    } catch (err) {
      if (specifier.startsWith('.')) return next(`${specifier}.ts`, context);
      throw err;
    }
  },
  load(url, context, next) {
    if (url.endsWith('.json')) return { format: 'module', source: `export default ${readFileSync(new URL(url), 'utf8')};`, shortCircuit: true };
    return next(url, context);
  },
});
const { mockAggregateLogs, mockSearchLogs } = await import('../web/src/api/mock/index.ts');
const { logRequest } = await import('../web/src/api/integrations/logs.ts');
const { searchSignals } = await import('../web/src/api/mock/security.ts');

const dashboard = buildDashboard();
const all = (widgets) => widgets.flatMap((w) => [w, ...(w.definition.type === 'group' ? all(w.definition.widgets) : [])]);
const panels = all(dashboard.widgets).filter((w) => w.definition.type === 'integration');
const streams = all(dashboard.widgets).filter((w) => w.definition.type === 'log_stream');
const queries = (source) => panels.flatMap((w) => w.definition.original.requests.flatMap((r) => r.queries.filter((q) => q.data_source === source).map((q) => [w.definition.title, q])));

test('the seed is the script output', () => {
  const seed = readFileSync(new URL('../server/seeds/integrations/gitlab-audit-events/dashboards.json', import.meta.url), 'utf8');
  assert.equal(seed, JSON.stringify([dashboard], null, 2) + '\n', 'run node scripts/gitlab-audit-events-dashboard.mjs');
});

test("one dashboard with our logo and Datadog's sections, over the GitLab audit events only", () => {
  assert.equal(dashboard.id, 'gitlab-audit-events-overview');
  assert.equal(dashboard.logo, 'gitlab-audit-events');
  assert.ok(all(dashboard.widgets).some((w) => w.definition.type === 'image' && w.definition.logo === 'gitlab-audit-events'));
  const groups = dashboard.widgets.map((w) => w.definition.title).filter(Boolean);
  for (const section of ['Overview', 'User insights', 'Authorization granted', 'Merge requests', 'Project activity', 'Token management', 'Cloud SIEM signals'])
    assert.ok(groups.includes(section), section);
  assert.ok(panels.length >= 35 && streams.length >= 6, `${panels.length} panels, ${streams.length} streams`);
  for (const w of panels) for (const r of w.definition.original.requests) for (const q of r.queries) assert.ok(['logs', 'security_signals'].includes(q.data_source), w.definition.title);
  assert.ok(queries('security_signals').length >= 4);
  for (const q of [...[...queries('logs'), ...queries('security_signals')].map(([, q]) => q.search.query), ...streams.map((w) => w.definition.query)]) {
    assert.match(q, /^source:gitlab-audit-events( |$)/, q);
    assert.doesNotMatch(q, /\(@|\) OR \(/, `the search syntax has no groups across attributes: ${q}`);
  }
});

test('every panel and stream shows data in mock mode', async () => {
  const to = Date.now();
  const range = { from: to - 7 * 86_400_000, to };
  for (const [title, q] of queries('logs')) {
    const buckets = mockAggregateLogs(logRequest(q, (s) => s, range, false)).data.buckets;
    assert.ok(buckets.length > 0 && buckets.some((b) => b.computes.c0 > 0), `${title}: ${q.search.query} ${JSON.stringify(q.group_by)}`);
  }
  for (const [title, q] of queries('security_signals')) {
    const page = await searchSignals({ query: q.search.query, range, facets: ['@workflow.rule.name'] });
    assert.ok(page.total > 0, `${title}: ${q.search.query}`);
  }
  for (const w of streams) {
    assert.ok(mockSearchLogs({ range, query: w.definition.query }).total > 0, `${w.definition.title}: ${w.definition.query}`);
  }
});
