// The Google Workspace Overview dashboard (scripts/google-workspace-dashboard.mjs) over the integration's activities.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { buildDashboard } from './google-workspace-dashboard.mjs';

const dashboard = buildDashboard();
const all = (widgets) => widgets.flatMap((w) => [w, ...(w.definition.type === 'group' ? all(w.definition.widgets) : [])]);
const panels = all(dashboard.widgets).filter((w) => w.definition.type === 'integration');
const streams = all(dashboard.widgets).filter((w) => w.definition.type === 'log_stream');
const queries = (source) => panels.flatMap((w) => w.definition.original.requests.flatMap((r) => r.queries.filter((q) => q.data_source === source).map((q) => [w.definition.title, q])));

test('the seed is the script output', () => {
  const seed = readFileSync(new URL('../server/seeds/integrations/google-workspace/dashboards.json', import.meta.url), 'utf8');
  assert.equal(seed, JSON.stringify([dashboard], null, 2) + '\n', 'run node scripts/google-workspace-dashboard.mjs');
});

test('one dashboard with our logo and its sections, over the Google Workspace activities only', () => {
  assert.equal(dashboard.id, 'google-workspace-overview');
  assert.equal(dashboard.logo, 'google-workspace');
  assert.ok(all(dashboard.widgets).some((w) => w.definition.type === 'image' && w.definition.logo === 'google-workspace'));
  const groups = dashboard.widgets.map((w) => w.definition.title).filter(Boolean);
  for (const section of ['Overview', 'Logins', 'Admin activity', 'Drive sharing', 'OAuth apps', 'Groups, users and devices', 'Cloud SIEM signals']) assert.ok(groups.includes(section), section);
  assert.ok(panels.length >= 40 && streams.length >= 7, `${panels.length} panels, ${streams.length} streams`);
  for (const w of panels) for (const r of w.definition.original.requests) for (const q of r.queries) assert.ok(['logs', 'security_signals'].includes(q.data_source), w.definition.title);
  assert.ok(queries('security_signals').length >= 4);
  for (const q of [...[...queries('logs'), ...queries('security_signals')].map(([, q]) => q.search.query), ...streams.map((w) => w.definition.query)]) {
    assert.match(q, /^source:gsuite( |$)/, q);
    assert.doesNotMatch(q, /\(@|\) OR \(/, `the search syntax has no groups across attributes: ${q}`);
  }
});
