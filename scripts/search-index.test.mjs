import assert from 'node:assert/strict';
import test from 'node:test';
import { registerHooks } from 'node:module';

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

const { NAV_SECTIONS, NAV_FOOTER } = await import('../web/src/components/shell/navMenus.ts');
const { buildPageIndex, rankEntries, entriesForChip } = await import('../web/src/components/shell/searchIndex.ts');

const INDEX = buildPageIndex(NAV_SECTIONS, NAV_FOOTER);

test('every sidebar item, menu group and link becomes a page entry with a category and path', () => {
  const byPath = new Map(INDEX.map((e) => [e.path, e]));
  assert.equal(byPath.get('/monitors')?.name, 'Monitors');
  assert.equal(byPath.get('/check/summary')?.category, 'Monitoring');
  assert.ok(INDEX.every((e) => e.name && e.category && e.path.startsWith('/')));
});

test('paths appear once; the first (most specific title) wins', () => {
  const paths = INDEX.map((e) => e.path);
  assert.equal(new Set(paths).size, paths.length);
});

test('create links are actions', () => {
  const create = INDEX.filter((e) => e.kind === 'action');
  assert.ok(create.length > 0);
  assert.ok(create.some((e) => e.path === '/automation/workflows/new'));
});

test('ranking puts exact and prefix name matches before substring and category matches', () => {
  const entries = [
    { kind: 'page', name: 'Service Catalog', category: 'APM', path: '/catalog' },
    { kind: 'page', name: 'Monitors', category: 'Monitoring', path: '/monitors' },
    { kind: 'page', name: 'Monitor Quality', category: 'Monitoring', path: '/monitors/quality' },
    { kind: 'page', name: 'Synthetic Monitoring', category: 'Digital Experience', path: '/synthetics' },
    { kind: 'page', name: 'SLOs', category: 'Monitoring', path: '/slo' },
  ];
  assert.deepEqual(
    rankEntries(entries, 'monitor').map((e) => e.name),
    ['Monitor Quality', 'Monitors', 'Synthetic Monitoring', 'SLOs'],
  );
});

test('every query word must match the name, category or path', () => {
  const entries = [
    { kind: 'page', name: 'Pods', category: 'Kubernetes', path: '/orchestration/explorer/pod' },
    { kind: 'page', name: 'Nodes', category: 'Kubernetes', path: '/orchestration/explorer/node' },
  ];
  assert.deepEqual(
    rankEntries(entries, 'kube pod').map((e) => e.name),
    ['Pods'],
  );
});

test('an empty query keeps the original order, up to the limit', () => {
  assert.deepEqual(
    rankEntries(INDEX, '', 3).map((e) => e.path),
    INDEX.slice(0, 3).map((e) => e.path),
  );
});

test('chips narrow the page index: actions are create links, settings are settings pages', () => {
  assert.ok(entriesForChip(INDEX, 'actions').every((e) => e.kind === 'action'));
  const settings = entriesForChip(INDEX, 'settings');
  assert.ok(settings.length > 0);
  assert.ok(settings.every((e) => /settings/i.test(e.name) || /settings/.test(e.path)));
  assert.equal(entriesForChip(INDEX, null).length, INDEX.length);
});

test('menu links keep their label (one generic word gets the group title) and take the group as category', () => {
  const byPath = new Map(INDEX.map((e) => [e.path, e]));
  assert.equal(byPath.get('/monitors/triggered')?.name, 'Monitors Triggered');
  assert.equal(byPath.get('/monitors/triggered')?.category, 'Monitors');
  const explorer = INDEX.find((e) => e.name === 'Kubernetes Explorer');
  assert.ok(explorer, 'Kubernetes Explorer is listed under its own name');
  assert.equal(explorer.category, 'Containers');
});

test('on an equal match, pages and actions rank before entities', () => {
  const entries = [
    { kind: 'dashboard', name: 'Kubernetes - Overview', category: 'Dashboard', path: '/dashboards/k8s' },
    { kind: 'page', name: 'Kubernetes Overview', category: 'Containers', path: '/kubernetes' },
  ];
  assert.deepEqual(
    rankEntries(entries, 'kube').map((e) => e.path),
    ['/kubernetes', '/dashboards/k8s'],
  );
});
