import assert from 'node:assert/strict';
import test from 'node:test';
import { registerHooks } from 'node:module';

// The web sources use extensionless relative imports (bundler resolution); resolve them to `.ts` here.
registerHooks({
  resolve(specifier, context, next) {
    try {
      return next(specifier, context);
    } catch (err) {
      if (specifier.startsWith('.')) return next(`${specifier}.ts`, context);
      throw err;
    }
  },
});
const model = await import('../web/src/pages/fleet/fleetModel.ts');
const mock = await import('../web/src/api/mock/fleet.ts');

const HOUR = 3_600_000;
const now = Date.now();
const range = { from: now - HOUR, to: now };

test('line diff marks added, removed and unchanged lines', () => {
  const a = 'agent:\n  logs: true\n  apm: true\nintegrations:\n- redisdb';
  const b = 'agent:\n  logs: false\n  apm: true\nintegrations:\n- redisdb\n- nginx';
  const diff = model.lineDiff(a, b);
  assert.deepEqual(
    diff.map((d) => `${d.kind} ${d.text}`),
    ['same agent:', 'del   logs: true', 'add   logs: false', 'same   apm: true', 'same integrations:', 'same - redisdb', 'add - nginx'],
  );
  // Line numbers on each side.
  assert.deepEqual(
    diff.map((d) => [d.left ?? null, d.right ?? null]),
    [
      [1, 1],
      [2, null],
      [null, 2],
      [3, 3],
      [4, 4],
      [5, 5],
      [null, 6],
    ],
  );
  assert.ok(model.lineDiff(a, a).every((d) => d.kind === 'same'));
  assert.equal(model.diffStats(diff).added, 2);
  assert.equal(model.diffStats(diff).removed, 1);
});

test('check names map to integration logos from our registry', () => {
  assert.equal(model.integrationLogo('redisdb'), 'redis');
  assert.equal(model.integrationLogo('postgres'), 'postgresql');
  assert.equal(model.integrationLogo('kubelet'), 'kubernetes');
  assert.equal(model.integrationLogo('nginx'), 'nginx');
  assert.equal(model.integrationLogo('system'), 'linux');
  assert.equal(model.integrationLogo('ibm_mq'), 'tile:ibm_mq', 'unknown checks get a lettered tile');
});

test('install methods pick their upgrade instructions', () => {
  assert.equal(model.installGuide('helm', 'datadog_agent'), 'helm');
  assert.equal(model.installGuide('datadog-operator', 'datadog_agent'), 'operator');
  assert.equal(model.installGuide('install_script_agent7', 'datadog_agent'), 'linux');
  assert.equal(model.installGuide('apt', 'datadog_agent'), 'linux');
  assert.equal(model.installGuide('docker', 'datadog_agent'), 'docker');
  assert.equal(model.installGuide('windows_msi', 'datadog_agent'), 'windows');
  assert.equal(model.installGuide('observecraft-agent', 'otel_collector'), 'observecraft-agent');
  assert.equal(model.installGuide(null, 'otel_collector'), 'otel');
  assert.equal(model.installGuide(null, 'datadog_agent'), 'unknown');
  for (const g of ['helm', 'operator', 'linux', 'docker', 'windows', 'observecraft-agent', 'otel', 'unknown']) {
    const guide = model.UPGRADE_GUIDES[g];
    assert.ok(guide.title && guide.steps.length > 0, `${g} has steps`);
  }
});

test('outdated agents are grouped by kind and version, oldest first', () => {
  const agents = [
    { kind: 'datadog_agent', version: '7.58.1', outdated: true, hostname: 'a' },
    { kind: 'datadog_agent', version: '7.60.0', outdated: true, hostname: 'b' },
    { kind: 'datadog_agent', version: '7.58.1', outdated: true, hostname: 'c' },
    { kind: 'datadog_agent', version: '7.62.2', outdated: false, hostname: 'd' },
    { kind: 'otel_collector', version: '0.160.0', outdated: true, hostname: 'e' },
  ];
  const groups = model.outdatedByVersion(agents);
  assert.deepEqual(
    groups.map((g) => [g.kind, g.version, g.agents.map((a) => a.hostname)]),
    [
      ['datadog_agent', '7.58.1', ['a', 'c']],
      ['datadog_agent', '7.60.0', ['b']],
      ['otel_collector', '0.160.0', ['e']],
    ],
  );
  assert.ok(model.compareVersions('7.62.10', '7.62.9') > 0);
  assert.ok(model.compareVersions('7.63.0-rc.1', '7.63.0') < 0);
});

test('the mock fleet has Datadog Agents and OTel Collectors on fictional hosts', async () => {
  const page = await mock.searchFleet({ range, query: '' });
  assert.equal(page.enabled, true);
  assert.equal(page.summary.agents, page.agents.length);
  assert.ok(page.agents.some((a) => a.kind === 'otel_collector'));
  assert.ok(page.agents.filter((a) => a.kind === 'datadog_agent').length >= 8);
  for (const a of page.agents) {
    assert.match(a.hostname, /^i-0a1b2c3d4e5f6\d{4}$/);
    assert.ok(!a.cluster || ['shop-prod-eu', 'shop-staging'].includes(a.cluster));
    assert.equal(a.id, `${a.kind}:${a.hostname}`);
  }
  assert.match(page.latest.datadogAgent, /^7\.6[0-2]\.\d+$/);
  const dd = page.agents.filter((a) => a.kind === 'datadog_agent');
  for (const a of dd) {
    assert.match(a.version, /^7\.(58|59|60|61|62)\.\d+$/);
    assert.equal(a.outdated, model.compareVersions(a.version, page.latest.datadogAgent) < 0, `${a.hostname} ${a.version}`);
  }
  assert.equal(page.summary.outdated, page.agents.filter((a) => a.outdated).length);
  assert.equal(page.summary.withIssues, page.agents.filter((a) => a.status === 'issues').length);
  assert.ok(page.summary.withIssues > 0 && page.summary.outdated > 0);
  assert.ok(page.summary.coverage.pct > 0 && page.summary.coverage.pct < 100);
  assert.equal(
    page.versions.reduce((n, v) => n + v.count, 0),
    page.agents.length,
  );
});

test('the mock filters by facets like the API', async () => {
  const all = await mock.searchFleet({ range, query: '' });
  const redis = await mock.searchFleet({ range, query: 'integration:redisdb' });
  assert.ok(redis.agents.length > 0 && redis.agents.length < all.agents.length);
  assert.ok(redis.agents.every((a) => a.integrations.includes('redisdb')));
  const collectors = await mock.searchFleet({ range, query: 'type:otel_collector' });
  assert.ok(collectors.agents.every((a) => a.kind === 'otel_collector'));
  const staging = await mock.searchFleet({ range, query: 'cluster:shop-staging status:issues' });
  assert.ok(staging.agents.every((a) => a.cluster === 'shop-staging' && a.status === 'issues'));
  // Summary and facets describe the whole fleet, whatever the query.
  assert.deepEqual(redis.summary, all.summary);
  assert.ok(all.facets.find((f) => f.key === 'integration').values.length > 3);
});

test('mock agent details carry checks and a configuration without secrets', async () => {
  const page = await mock.searchFleet({ range, query: 'status:issues type:datadog_agent' });
  const detail = await mock.getFleetAgent({ range, id: page.agents[0].id });
  assert.equal(detail.agent.id, page.agents[0].id);
  assert.ok(detail.checks.length > 0);
  assert.ok(detail.configYaml.includes('<redacted>'));
  assert.doesNotMatch(detail.configYaml, /api_key: [0-9a-f]{8}/);
  const collector = (await mock.searchFleet({ range, query: 'type:otel_collector' })).agents[0];
  assert.equal((await mock.getFleetAgent({ range, id: collector.id })).configYaml, null);
  await assert.rejects(mock.getFleetAgent({ range, id: 'datadog_agent:nope' }));
});

test('mock profiles cover every agent once', async () => {
  const page = await mock.searchFleet({ range, query: '' });
  const profiles = await mock.listFleetProfiles({ range });
  const ids = profiles.flatMap((p) => p.agents.map((a) => a.id)).sort();
  assert.deepEqual(ids, page.agents.map((a) => a.id).sort());
  assert.ok(profiles.length >= 3);
  assert.ok(profiles[0].agents.length >= profiles.at(-1).agents.length, 'largest first');
});
