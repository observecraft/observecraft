import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
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
  load(url, context, next) {
    if (url.endsWith('.json')) return { format: 'module', source: `export default ${readFileSync(new URL(url), 'utf8')};`, shortCircuit: true };
    return next(url, context);
  },
});
const logic = await import('../web/src/api/portalCatalog/logic.ts');
const reports = await import('../web/src/api/portalCatalog/reports.ts');
const model = await import('../web/src/pages/portal/catalog/catalogModel.ts');
const mock = await import('../web/src/api/mock/portalCatalog.ts');

const DAY = 86_400_000;
const instrumented = new Set(['order-service', 'billing', 'storefront-api']);

test('peers are inferred like the server: datastores by database, queues by destination, externals by peer', () => {
  const call = (service, attrs, hits = 10) => ({ service, hits, errors: hits / 10, p95Ms: hits, ...attrs });
  assert.deepEqual(logic.inferPeer(call('order-service', { dbSystem: 'postgresql', dbName: 'orders-db', peerService: 'postgres' }), instrumented), {
    kind: 'datastore',
    name: 'orders-db',
    technology: 'postgresql',
  });
  assert.equal(logic.inferPeer(call('storefront-api', { dbSystem: 'redis', dbName: '0', peerService: 'cart-cache' }), instrumented).name, 'cart-cache');
  assert.equal(logic.inferPeer(call('order-service', { messagingSystem: 'kafka', destination: 'order-events', peerService: 'kafka' }), instrumented).name, 'order-events');
  assert.equal(logic.inferPeer(call('billing', { peerService: 'payments-provider' }), instrumented).kind, 'external_provider');
  assert.equal(logic.inferPeer(call('storefront-api', { peerService: 'order-service' }), instrumented), null);
  const out = logic.inferEntities([call('order-service', { dbSystem: 'postgresql', dbName: 'orders-db' }, 10), call('billing', { dbSystem: 'postgresql', dbName: 'orders-db' }, 30)], instrumented);
  assert.deepEqual(
    out.map((e) => [e.name, e.requests, e.errors, e.p95Ms, e.callers]),
    [['orders-db', 40, 4, 30, ['billing', 'order-service']]],
  );
});

test('definitions are validated and normalized like the server', () => {
  const def = logic.validateDefinition({
    apiVersion: 'v3',
    kind: 'Service',
    metadata: { name: 'Order-Service', owner: '@checkout', tags: ['tier:1'], links: [{ type: 'repo', url: 'https://git.example.com/example-org/order-service' }] },
    spec: { tier: 1, lifecycle: 'Production', dependsOn: ['billing', 'datastore:orders-db'], componentOf: ['checkout'] },
  });
  assert.equal(def.kind, 'service');
  assert.equal(def.metadata.name, 'order-service');
  assert.equal(def.metadata.owner, 'checkout');
  assert.equal(def.spec.tier, '1');
  assert.equal(def.spec.lifecycle, 'production');
  assert.deepEqual(def.spec.dependsOn, ['service:billing', 'datastore:orders-db']);
  assert.deepEqual(def.spec.componentOf, ['system:checkout']);
  assert.equal(def.metadata.links[0].name, 'https://git.example.com/example-org/order-service');
  assert.throws(() => logic.validateDefinition({ apiVersion: 'v2.2', kind: 'service', metadata: { name: 'x' } }), /apiVersion 'v2.2'/);
  assert.throws(() => logic.validateDefinition({ apiVersion: 'v3', kind: 'lambda', metadata: { name: 'x' } }), /kind 'lambda'/);
  assert.throws(() => logic.validateDefinition({ apiVersion: 'v3', kind: 'service', metadata: { name: 'has space' } }), /may only contain/);
  assert.throws(() => logic.validateDefinition({ apiVersion: 'v3', kind: 'service', metadata: { name: 'x', links: [{ type: 'repo', url: 'javascript:alert(1)' }] } }), /http\(s\) URL/);
  assert.throws(() => logic.validateDefinition({ apiVersion: 'v3', kind: 'service', metadata: { name: 'x', contacts: [{ type: 'email', contact: 'nobody' }] } }), /valid email/);
  assert.throws(() => logic.parseJsonDocuments('apiVersion: v3'), /JSON documents/);
  const twice = JSON.stringify([
    { apiVersion: 'v3', kind: 'queue', metadata: { name: 'q' } },
    { apiVersion: 'v3', kind: 'queue', metadata: { name: 'Q' } },
  ]);
  assert.throws(() => logic.parseJsonDocuments(twice), /Document 2: queue:q is defined twice/);
});

test('repository paths ignore hosts and suffixes', () => {
  assert.equal(logic.repoPath('https://git.example.com/example-org/order-service.git'), 'example-org/order-service');
  assert.equal(logic.repoPath('git@git.example.com:example-org/billing.git'), 'example-org/billing');
  assert.equal(logic.repoPath('Example-Org/Billing/'), 'example-org/billing');
});

test('mock catalog merges telemetry, metadata and definitions', async () => {
  mock.resetPortalCatalogMock();
  const now = Date.now();
  const cat = await mock.getPortalCatalog({ range: { from: now - 3600_000, to: now } });
  const byId = new Map(cat.entities.map((e) => [e.id, e]));
  const order = byId.get('service:order-service');
  assert.ok(order, 'order-service is listed');
  assert.ok(order.sources.includes('apm') && order.sources.includes('metadata'));
  assert.equal(order.owner, 'checkout');
  assert.ok(order.reqPerSec > 0);
  const db = byId.get('datastore:orders-db');
  assert.equal(db.technology, 'postgresql');
  assert.deepEqual(db.related, ['billing', 'order-service']);
  assert.equal(db.owner, 'checkout');
  assert.ok(!db.inferred);
  assert.ok(byId.get('queue:order-events'));
  assert.ok(byId.get('external_provider:payments-provider').inferred);
  assert.ok(byId.get('service:cart-sync-job').inferred);
  assert.deepEqual(byId.get('system:checkout').related.slice(0, 2), ['datastore:orders-db', 'queue:order-events']);
  assert.ok(cat.entities.some((e) => e.kind === 'endpoint' && e.service === 'order-service'));
  assert.ok(cat.entities.some((e) => e.kind === 'repository'));
  assert.ok(cat.teams.find((t) => t.handle === 'checkout').entities >= 3);
});

test('mock definitions: create, edit, import and delete; services write their metadata', async () => {
  mock.resetPortalCatalogMock();
  const created = await mock.upsertEntityDefinition({
    apiVersion: 'v3',
    kind: 'service',
    metadata: { name: 'search-api', owner: 'search', links: [{ name: 'Code', type: 'repo', url: 'https://git.example.com/example-org/search-api' }] },
    spec: { lifecycle: 'experimental' },
  });
  assert.equal(created.source, 'ui');
  const now = Date.now();
  let cat = await mock.getPortalCatalog({ range: { from: now - 3600_000, to: now } });
  const svc = cat.entities.find((e) => e.id === 'service:search-api');
  assert.equal(svc.owner, 'search');
  assert.equal(svc.repoUrl, 'https://git.example.com/example-org/search-api');
  assert.equal(svc.lifecycle, 'experimental');
  await assert.rejects(mock.updateEntityDefinition(created.id, { apiVersion: 'v3', kind: 'datastore', metadata: { name: 'orders-db' } }), /already defined/);
  const res = await mock.importEntityDefinitions(
    JSON.stringify([
      { apiVersion: 'v3', kind: 'api', metadata: { name: 'search-public-api', owner: 'search' } },
      { apiVersion: 'v3', kind: 'service', metadata: { name: 'search-api', owner: 'search' }, spec: { tier: 2 } },
    ]),
  );
  assert.deepEqual([res.created, res.updated], [1, 1]);
  await mock.deleteEntityDefinition(created.id);
  cat = await mock.getPortalCatalog({ range: { from: now - 3600_000, to: now } });
  // The service keeps its metadata (like the server), so it stays listed.
  assert.ok(cat.entities.find((e) => e.id === 'service:search-api').sources.includes('metadata'));
});

test('teams report counts incidents per bucket and MTTR like the server', () => {
  const start = Math.floor(1_800_000_000_000 / DAY) * DAY;
  const range = { from: start, to: start + 3 * DAY - 1 };
  const cov = {
    owners: new Map([
      ['order-service', 'checkout'],
      ['billing', 'payments'],
    ]),
    sloTeams: [['checkout']],
    monitorTeams: [['payments']],
    scheduleTeams: ['checkout'],
  };
  const incidents = [
    { declaredAt: start + 3600_000, resolvedAt: start + 7200_000, open: false, teams: ['checkout'], services: ['order-service'] },
    { declaredAt: start + DAY + 10, resolvedAt: start + DAY + 10 + 30 * 60_000, open: false, teams: [], services: ['billing', 'order-service'] },
    { declaredAt: start - 5 * DAY, resolvedAt: null, open: true, teams: ['payments'], services: [] },
  ];
  const r = reports.teamsReport(
    range,
    [
      { handle: 'checkout', name: 'Checkout', members: 3 },
      { handle: 'payments', name: 'Payments', members: 2 },
    ],
    incidents,
    cov,
  );
  assert.equal(r.bucketMs, DAY);
  assert.deepEqual(r.teams[0].incidents, [1, 1, 0]);
  assert.equal(r.teams[0].mttrMinutes, 45);
  assert.deepEqual([r.teams[1].incidentCount, r.teams[1].openIncidents, r.teams[1].mttrMinutes], [1, 1, 30]);
  assert.equal(reports.bucketSize({ from: 0, to: 90 * DAY }), 7 * DAY);
  const trend = reports.scorecardTrend(
    range,
    [
      [start, 'a', 'checkout', 1],
      [start, 'b', 'checkout', 2],
      [start + 2 * DAY, 'a', 'checkout', 3],
    ],
    [],
  );
  assert.deepEqual(trend.teams[0].levels, [1.5, null, 3]);
  assert.deepEqual(trend.teams[0].current, [0, 0, 0, 1]);
});

test('catalog health and reliability by team', () => {
  const e = (over) => ({ kind: 'service', owner: '', docsUrl: '', repoUrl: '', links: [], contacts: [], definitionId: null, ...over });
  const health = reports.catalogHealth(
    [
      e({ owner: 'checkout', docsUrl: 'https://docs.example.com', contacts: [{ type: 'slack', contact: '#c' }] }),
      e({ repoUrl: 'https://git.example.com/x' }),
      e({ kind: 'endpoint' }),
      e({ kind: 'datastore', definitionId: 'd', owner: 'platform' }),
    ],
    new Set(['checkout']),
  );
  assert.deepEqual(
    health.rows.map((r) => [r.key, r.covered, r.total]),
    [
      ['owner', 2, 3],
      ['docs', 1, 3],
      ['repo', 1, 3],
      ['contacts', 1, 3],
      ['onCall', 1, 3],
    ],
  );
  assert.deepEqual(reports.servicesNamed(['avg:trace.errors{service:order-service,env:prod}', 'service:billing']), ['billing', 'order-service']);
  const slo = (teams, tags, state, budget) => ({ teams, tags, goodQuery: null, totalQuery: null, sliceQuery: null, status: { state, budgetRemaining: budget } });
  const rows = reports.reliabilityByTeam(
    [slo([], ['service:order-service'], 'breached', -5), slo(['payments'], [], 'ok', 80)],
    [
      { teams: [], tags: [], query: 'avg:x{service:order-service}', status: 'alert' },
      { teams: [], tags: [], query: 'avg:y{*}', status: 'ok' },
    ],
    new Map([['order-service', 'checkout']]),
  );
  assert.deepEqual(
    rows.map((r) => [r.team, r.slos, r.breached, r.minBudget, r.monitors, r.alerting]),
    [
      ['checkout', 1, 1, -5, 1, 1],
      ['payments', 1, 0, 80, 0, 0],
      ['', 0, 0, null, 1, 0],
    ],
  );
});

test('catalog navigation counts, views and search', () => {
  const ent = (kind, name, over = {}) => ({ id: `${kind}:${name}`, kind, name, owner: '', tags: [], displayName: null, inferred: false, ...over });
  const entities = [
    ent('service', 'order-service', { owner: 'checkout', tags: ['tier:1'] }),
    ent('service', 'billing', { owner: 'payments' }),
    ent('datastore', 'orders-db', { owner: 'checkout' }),
    ent('endpoint', 'POST /orders', { owner: 'checkout' }),
    ent('service', 'cart-sync-job', { inferred: true }),
  ];
  const counts = model.navCounts(entities, ['checkout']);
  assert.equal(counts.software, 5);
  assert.equal(counts.mine, 3);
  assert.equal(counts.service, 3);
  assert.equal(counts.inferred, 1);
  assert.deepEqual(
    model.filterEntities(entities, { section: 'service', search: 'tier:1', teams: [], myTeams: [] }).map((e) => e.name),
    ['order-service'],
  );
  assert.deepEqual(
    model.filterEntities(entities, { section: 'mine', search: '', teams: [], myTeams: ['checkout'] }).map((e) => e.name),
    ['order-service', 'orders-db', 'POST /orders'],
  );
  assert.deepEqual(
    model.filterEntities(entities, { section: 'inferred', search: '', teams: [], myTeams: [] }).map((e) => e.name),
    ['cart-sync-job'],
  );
  assert.deepEqual(
    model.filterEntities(entities, { section: 'all', search: 'ORDER', teams: ['checkout'], myTeams: [] }).map((e) => e.name),
    ['order-service', 'orders-db', 'POST /orders'],
  );
  for (const view of model.VIEWS) assert.ok(model.columnsFor(view.id).length >= 3, view.id);
  assert.equal(model.columnsFor('ownership')[0], 'name');
});

test('the entity form round-trips through a definition', () => {
  const form = model.emptyForm();
  Object.assign(form, { kind: 'queue', name: 'Order-Events', owner: '@platform', system: 'checkout', description: 'Events', lifecycle: 'production', tier: '1', tags: 'tier:1, env:prod' });
  form.links = [{ name: 'Runbook', type: 'runbook', url: 'https://docs.example.com/runbooks/order-events' }];
  const def = logic.validateDefinition(model.definitionFromForm(form));
  assert.equal(def.metadata.name, 'order-events');
  assert.deepEqual(def.metadata.tags, ['tier:1', 'env:prod']);
  assert.deepEqual(def.spec.componentOf, ['system:checkout']);
  const back = model.formFromDefinition(def);
  assert.equal(back.owner, 'platform');
  assert.equal(back.system, 'checkout');
  assert.equal(back.tags, 'tier:1, env:prod');
});

test('definitions are written as YAML for repositories', () => {
  const yaml = model.definitionYaml({
    apiVersion: 'v3',
    kind: 'queue',
    metadata: { name: 'order-events', owner: 'platform', tags: ['tier:1'], links: [{ name: 'Runbook', type: 'runbook', url: 'https://docs.example.com/r' }], contacts: [] },
    spec: { tier: '1', componentOf: ['system:checkout'], lifecycle: null },
  });
  assert.equal(
    yaml,
    [
      'apiVersion: v3',
      'kind: queue',
      'metadata:',
      '  name: order-events',
      '  owner: platform',
      '  tags:',
      '    - tier:1',
      '  links:',
      '    - name: Runbook',
      '      type: runbook',
      '      url: https://docs.example.com/r',
      'spec:',
      '  tier: "1"',
      '  componentOf:',
      '    - system:checkout',
      '',
    ].join('\n'),
  );
});
