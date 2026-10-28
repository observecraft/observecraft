import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { registerHooks } from 'node:module';

// The web sources use extensionless relative imports (bundler resolution); resolve them to `.ts` here.
// They import JSON without import attributes, as the bundler allows: load it as a module.
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
const status = await import('../web/src/api/statusPages/status.ts');
const mock = await import('../web/src/api/mock/statusPages.ts');
const types = await import('../web/src/api/statusPages/types.ts');

const DAY = 86_400_000;
const HOUR = 3_600_000;
// Noon UTC, 2026-10-28.
const NOW = Date.UTC(2026, 9, 28, 12);
const TODAY = Date.UTC(2026, 9, 28);

const update = (createdAt, status, impacts) => ({ id: createdAt, status, message: '', impacts, createdBy: null, createdAt });
const impact = (componentId, impact) => ({ componentId, impact });

test('statuses rank from operational to major outage', () => {
  assert.equal(status.worstStatus([]), 'operational');
  assert.equal(status.worstStatus(['degraded_performance', 'major_outage', 'partial_outage']), 'major_outage');
  assert.equal(status.worstStatus(['operational', 'degraded_performance']), 'degraded_performance');
});

test('open notices make components worse, never better; resolved ones are ignored', () => {
  const components = [
    { id: 'checkout', name: 'Checkout', group: null, status: 'operational' },
    { id: 'search', name: 'Search', group: null, status: 'partial_outage' },
  ];
  const notices = [
    { resolvedAt: null, impacts: [impact('checkout', 'degraded_performance'), impact('search', 'degraded_performance')] },
    { resolvedAt: NOW, impacts: [impact('checkout', 'major_outage')] },
  ];
  const current = status.currentComponents(components, notices);
  assert.deepEqual(
    current.map((c) => [c.id, c.status, c.currentStatus]),
    [
      ['checkout', 'operational', 'degraded_performance'],
      ['search', 'partial_outage', 'partial_outage'],
    ],
  );
  assert.equal(status.overallStatus(current), 'partial_outage');
  assert.equal(status.overallStatus([]), 'operational');
});

test('slugs: lowercase letters, digits and inner dashes; names suggest one', () => {
  for (const ok of ['status', 'shop-status-2', 'a']) assert.equal(status.checkSlug(ok), null, ok);
  for (const bad of ['', 'Status', '-status', 'status-', 'with space', 'x'.repeat(65)]) assert.match(status.checkSlug(bad), /lowercase letters, digits and dashes/, bad);
  assert.equal(status.slugify('  Storefront Status — EU!  '), 'storefront-status-eu');
  assert.equal(status.slugify('***'), '');
});

test('logos are empty or http(s) URLs', () => {
  for (const ok of ['', 'https://cdn.example.com/logo.png', 'http://example.com/a.svg']) assert.equal(status.checkLogoUrl(ok), null, ok);
  for (const bad of ['javascript:alert(1)', 'data:image/png;base64,AA', 'logo.png', 'ftp://example.com/x.png']) assert.equal(status.checkLogoUrl(bad), 'The logo must be an http or https URL', bad);
});

test('components are grouped in order of first appearance', () => {
  const c = (name, group) => ({ id: name, name, group, status: 'operational', currentStatus: 'operational' });
  const groups = status.groupComponents([c('Checkout', 'Storefront'), c('Billing', null), c('Search', 'Storefront'), c('API', 'Platform')]);
  assert.deepEqual(
    groups.map((g) => [g.group, g.components.map((x) => x.name)]),
    [
      ['Storefront', ['Checkout', 'Search']],
      [null, ['Billing']],
      ['Platform', ['API']],
    ],
  );
});

test('uptime: 60 UTC days, the last one today, operational without notices', () => {
  const days = status.uptimeDays('checkout', [], NOW);
  assert.equal(days.length, 60);
  assert.equal(days[59].start, TODAY);
  assert.equal(days[0].start, TODAY - 59 * DAY);
  assert.ok(days.every((d) => d.status === 'operational' && d.downMs === 0));
  assert.equal(status.uptimePercent(days, NOW), 100);
});

test('uptime replays the updates: each one sets the impacts until the next', () => {
  const start = TODAY - 2 * DAY + 22 * HOUR; // two days ago, 22:00
  const notice = {
    startedAt: start,
    resolvedAt: start + 6 * HOUR,
    impacts: [],
    updates: [
      update(start + 6 * HOUR, 'resolved', []),
      update(start + 3 * HOUR, 'monitoring', [impact('checkout', 'degraded_performance')]),
      update(start, 'investigating', [impact('checkout', 'major_outage'), impact('search', 'partial_outage')]),
    ],
  };
  const days = status.uptimeDays('checkout', [notice], NOW);
  // 22:00–24:00 major outage two days ago; 00:00–01:00 major, then degraded until 04:00 yesterday.
  assert.deepEqual(days[57], { start: TODAY - 2 * DAY, status: 'major_outage', downMs: 2 * HOUR, noData: false });
  assert.deepEqual(days[58], { start: TODAY - DAY, status: 'major_outage', downMs: HOUR, noData: false });
  assert.deepEqual(days[59], { start: TODAY, status: 'operational', downMs: 0, noData: false });
  const total = 59 * DAY + 12 * HOUR;
  assert.equal(status.uptimePercent(days, NOW), 100 * (1 - (3 * HOUR) / total));
  // Search: partial outage for the first 3 hours only.
  const search = status.uptimeDays('search', [notice], NOW);
  assert.equal(search[57].status, 'partial_outage');
  assert.equal(search[58].downMs, HOUR);
  assert.equal(
    status.uptimeDays('cart', [notice], NOW).some((d) => d.status !== 'operational'),
    false,
  );
});

test('uptime: open notices last until now, overlapping outages count once, degraded is not downtime', () => {
  const a = { startedAt: TODAY + HOUR, resolvedAt: null, impacts: [], updates: [update(TODAY + HOUR, 'investigating', [impact('checkout', 'partial_outage')])] };
  const b = { startedAt: TODAY + 2 * HOUR, resolvedAt: TODAY + 4 * HOUR, impacts: [], updates: [update(TODAY + 2 * HOUR, 'identified', [impact('checkout', 'major_outage')])] };
  const slow = { startedAt: TODAY - DAY, resolvedAt: TODAY - DAY + 5 * HOUR, impacts: [], updates: [update(TODAY - DAY, 'identified', [impact('checkout', 'degraded_performance')])] };
  const days = status.uptimeDays('checkout', [a, b, slow], NOW);
  assert.deepEqual(days[59], { start: TODAY, status: 'major_outage', downMs: 11 * HOUR, noData: false });
  assert.deepEqual(days[58], { start: TODAY - DAY, status: 'degraded_performance', downMs: 0, noData: false });
});

test('uptime: a notice without updates uses its own impacts over its lifetime', () => {
  const n = { startedAt: TODAY + HOUR, resolvedAt: TODAY + 3 * HOUR, impacts: [impact('checkout', 'major_outage')], updates: [] };
  assert.deepEqual(status.uptimeDays('checkout', [n], NOW)[59], { start: TODAY, status: 'major_outage', downMs: 2 * HOUR, noData: false });
});

test('mock: create, edit with kept ids, notices, resolve and the public view', async () => {
  const page = await mock.createStatusPage({
    name: 'Shop',
    slug: 'shop-test',
    visibility: 'internal',
    emailSubscriptions: true,
    components: [{ name: 'Checkout', group: 'Storefront' }, { name: 'Search' }],
  });
  assert.equal(page.emailSubscriptions, true);
  assert.equal(page.slackSubscriptions, false);
  assert.equal(page.logoUrl, '');
  await assert.rejects(mock.createStatusPage({ name: 'Logo', slug: 'logo-test', visibility: 'public', logoUrl: 'javascript:alert(1)' }), /http or https/);
  const [checkout, search] = page.components;
  assert.match(checkout.id, /^c_[a-z0-9]{12}$/);
  await assert.rejects(mock.createStatusPage({ name: 'Again', slug: 'shop-test', visibility: 'public', components: [] }), /already taken/);
  await assert.rejects(mock.createStatusPage({ name: 'Bad', slug: 'Bad', visibility: 'public', components: [] }), /dashes/);

  let d = await mock.createStatusPageNotice(page.id, { title: 'Checkout errors', message: 'Investigating', status: 'investigating', impacts: [impact(checkout.id, 'major_outage')] });
  assert.equal(d.overallStatus, 'major_outage');
  const notice = d.notices[0];
  await assert.rejects(mock.createStatusPageNotice(page.id, { title: 'x', message: 'y', status: 'investigating', impacts: [impact('c_unknown', 'major_outage')] }), /not on this status page/);
  d = await mock.updateStatusPageNotice(page.id, notice.id, { message: 'Fixed', status: 'resolved' });
  assert.equal(d.overallStatus, 'operational');
  assert.equal(d.notices[0].updates.length, 2);
  assert.ok(d.notices[0].resolvedAt);
  await assert.rejects(mock.updateStatusPageNotice(page.id, notice.id, { message: 'again', status: 'monitoring' }), /resolved/);

  d = await mock.updateStatusPage(page.id, { name: 'Shop', slug: 'shop-test', visibility: 'public', components: [{ id: search.id, name: 'Product search' }, { name: 'Cart' }] });
  assert.equal(d.components[0].id, search.id);
  assert.notEqual(d.components[1].id, checkout.id);

  const pub = await mock.getPublicStatusPage('shop-test');
  assert.equal(pub.name, 'Shop');
  assert.ok(pub.notices.every((n) => n.createdBy === null && n.incidentId === null));
  assert.ok((await mock.listStatusPages()).some((p) => p.slug === 'shop-test' && p.componentCount === 2));
  await mock.deleteStatusPage(page.id);
  await assert.rejects(mock.getStatusPage(page.id), /not found/);
});

test('mock seed uses the demo domain and has history for the uptime bars', async () => {
  const pages = await mock.listStatusPages();
  assert.ok(pages.length >= 2);
  assert.ok(pages.some((p) => p.visibility === 'public') && pages.some((p) => p.visibility === 'internal'));
  const detail = await mock.getStatusPage(pages.find((p) => p.visibility === 'public').id);
  assert.ok(detail.notices.some((n) => n.resolvedAt) && detail.notices.some((n) => !n.resolvedAt));
  const text = JSON.stringify(detail).toLowerCase();
  assert.ok(['checkout', 'storefront', 'billing'].some((w) => text.includes(w)));
});

test('days before a component existed have no data and do not count toward uptime', () => {
  // Added yesterday at 18:00 UTC; a one-hour outage today.
  const since = TODAY - 6 * HOUR;
  const n = { startedAt: TODAY + HOUR, resolvedAt: TODAY + 2 * HOUR, impacts: [], updates: [update(TODAY + HOUR, 'investigating', [impact('checkout', 'major_outage')])] };
  const days = status.uptimeDays('checkout', [n], NOW, status.UPTIME_DAYS, since);
  assert.equal(days.filter((d) => d.noData).length, 58);
  assert.deepEqual(
    days.slice(58).map((d) => d.noData),
    [false, false],
  );
  // Covered time: 6 h yesterday and 12 h today; one hour down.
  assert.equal(status.uptimePercent(days, NOW, since), 100 * (1 - HOUR / (18 * HOUR)));
  // Without a creation time every day counts, as before.
  assert.equal(status.uptimeDays('checkout', [n], NOW).filter((d) => d.noData).length, 0);
});

test('visualization labels', () => {
  
  assert.deepEqual(Object.keys(types.VISUALIZATION_LABEL), ['bars_and_uptime', 'bars', 'names']);
});
