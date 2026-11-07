// Integrations → Community: integrations-extras' community and partner integrations, from the same catalog as the
// Integration Catalog (filterCatalog with `repository`, used by useCatalog in mock and live mode).
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { filterCatalog } from '../web/src/api/integrations/catalogFilter.ts';

const catalog = JSON.parse(readFileSync(new URL('../web/src/api/integrations/catalog.generated.json', import.meta.url), 'utf8')).integrations;
const summaries = catalog.map((c) => ({
  id: c.id,
  name: c.name,
  publisher: c.publisher,
  logo: c.id,
  status: 'available',
  categories: c.categories,
  description: c.description,
  repository: c.repository,
}));
const all = { query: '', status: 'all', category: null };

test('the community catalog is integrations-extras’ integrations, by their publishers', () => {
  const community = filterCatalog(summaries, { ...all, repository: 'integrations-extras' });
  assert.ok(community.length >= 110, `${community.length}`);
  const ids = new Set(community.map((i) => i.id));
  for (const id of ['bind9', 'unbound', 'gitea', 'redis-enterprise']) assert.ok(ids.has(id), id);
  for (const id of ['postgres', 'traefik-mesh']) assert.ok(!ids.has(id), id);
  assert.equal(community.find((i) => i.id === 'unbound').publisher, 'Community');
});

test('search, category and status filter, sorted by name; the Integration Catalog keeps every repository', () => {
  assert.deepEqual(
    filterCatalog(summaries, { ...all, query: 'bind' }).map((i) => i.id),
    ['bind9'],
  );
  const network = filterCatalog(summaries, { ...all, category: 'Network', repository: 'integrations-extras' });
  assert.ok(network.length > 0 && network.every((i) => i.categories.includes('Network')));
  const names = network.map((i) => i.name);
  assert.deepEqual(
    names,
    [...names].sort((a, b) => a.localeCompare(b)),
  );
  assert.deepEqual(filterCatalog([{ ...summaries[0], status: 'warning' }], { ...all, status: 'installed' }).length, 1, 'warnings count as installed');
});

test('no invented marketplace apps remain', () => {
  const mock = readFileSync(new URL('../web/src/api/mock/integrations.ts', import.meta.url), 'utf8');
  assert.doesNotMatch(mock, /MarketplaceApp|searchMarketplace/);
});

test('“Build an integration” leads to our integrations guide, which points to the Agent’s own guides', () => {
  const page = readFileSync(new URL('../web/src/pages/integrations/CommunityPage.tsx', import.meta.url), 'utf8');
  assert.match(page, /docs\/integrations\.md/);
  const guide = readFileSync(new URL('../docs/integrations.md', import.meta.url), 'utf8');
  for (const link of ['use-community-integrations', 'custom_checks/write_agent_check', 'integrations/agent_integration']) assert.ok(guide.includes(link), link);
  assert.match(guide, /datadog-agent integration install -t datadog-/);
});
