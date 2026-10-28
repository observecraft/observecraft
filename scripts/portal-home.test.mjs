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
const logic = await import('../web/src/api/portal/logic.ts');
const mock = await import('../web/src/api/mock/portal.ts');

const at = (h) => new Date(2026, 9, 28, h, 30);

test('greeting follows the local time of day', () => {
  assert.equal(logic.greeting(at(4)), 'Good evening');
  assert.equal(logic.greeting(at(5)), 'Good morning');
  assert.equal(logic.greeting(at(11)), 'Good morning');
  assert.equal(logic.greeting(at(12)), 'Good afternoon');
  assert.equal(logic.greeting(at(17)), 'Good afternoon');
  assert.equal(logic.greeting(at(18)), 'Good evening');
  assert.equal(logic.greeting(at(23)), 'Good evening');
  assert.equal(logic.dateLine(at(9)), 'Oct 28, 2026');
});

test('first names come from the display name, else the email', () => {
  assert.equal(logic.firstName('Alex Morgan'), 'Alex');
  assert.equal(logic.firstName('  Priya  '), 'Priya');
  assert.equal(logic.firstName('', 'sam.lee@example.com'), 'Sam');
  assert.equal(logic.firstName('', ''), '');
});

test('pull requests are grouped in display order, newest first', () => {
  const pr = (group, number, updatedAt) => ({
    group,
    number,
    updatedAt,
    title: '',
    repo: 'example-org/storefront',
    url: `u${number}`,
    author: { login: 'a', avatarUrl: '' },
    comments: 0,
    reviewers: [],
    additions: 1,
    deletions: 1,
  });
  const groups = logic.groupPullRequests([pr('merged', 1, 5), pr('needs_review', 2, 1), pr('needs_review', 3, 9), pr('drafts', 4, 2)]);
  assert.deepEqual(
    groups.map((g) => [g.key, g.label, g.items.map((i) => i.number)]),
    [
      ['needs_review', 'Needs your review', [3, 2]],
      ['returned', 'Returned to you', []],
      ['approved', 'Approved', []],
      ['waiting', 'Waiting for reviewers', []],
      ['drafts', 'Drafts', [4]],
      ['merged', 'Recently merged', [1]],
    ],
  );
});

test('entity tabs filter recents, teams and favorites', () => {
  const e = (name, team, lastSeen, favorite = false) => ({
    name,
    team,
    lastSeen,
    favorite,
    kind: 'service',
    lastDeploy: null,
    monitors: 0,
    issues: 0,
    dashboards: 0,
    scorecardLevel: null,
    onCall: null,
  });
  const all = [e('order-service', 'checkout', 3), e('billing', 'payments', 2, true), e('legacy-batch', 'checkout', null)];
  assert.deepEqual(
    logic.entitiesOf(all, 'recents', []).map((x) => x.name),
    ['order-service', 'billing'],
  );
  assert.deepEqual(
    logic.entitiesOf(all, 'teams', ['checkout']).map((x) => x.name),
    ['order-service', 'legacy-batch'],
  );
  assert.deepEqual(
    logic.entitiesOf(all, 'favorites', []).map((x) => x.name),
    ['billing'],
  );
  assert.deepEqual(
    logic.entitiesOf(all, 'teams', ['checkout'], 'legacy').map((x) => x.name),
    ['legacy-batch'],
  );
});

test('ages and logo guesses', () => {
  const now = 1_800_000_000_000;
  assert.equal(logic.age(null, now), '—');
  assert.equal(logic.age(now - 30_000, now), 'just now');
  assert.equal(logic.age(now - 5 * 60_000, now), '5m ago');
  assert.equal(logic.age(now - 3 * 3_600_000, now), '3h ago');
  assert.equal(logic.age(now - 2 * 86_400_000, now), '2d ago');
  assert.equal(logic.guessLogo('https://github.com/example-org'), 'github');
  assert.equal(logic.guessLogo('https://example-shop.atlassian.net/jira'), 'jira');
  assert.equal(logic.guessLogo('https://example.service-now.com'), '');
  assert.equal(logic.guessLogo('https://example.servicenow.com'), 'tile:servicenow');
  assert.equal(logic.guessLogo('not a url'), '');
});

test('mock: GitHub and Jira are connected, GitLab and Linear show the connect overlay', async () => {
  mock.resetPortalMock();
  const gh = await mock.getPullRequests('github');
  assert.equal(gh.status, 'ok');
  assert.ok(gh.pullRequests.length > 5);
  assert.ok(gh.pullRequests.every((p) => p.repo.startsWith('example-org/')));
  assert.equal((await mock.getPullRequests('gitlab')).status, 'not_connected');
  assert.equal((await mock.getTickets('jira')).status, 'ok');
  assert.equal((await mock.getTickets('linear')).status, 'not_connected');
  assert.ok((await mock.getTickets('work_items')).tickets.every((t) => t.key.startsWith('IR-')));
});

test('mock: connectors validate and never hand back the token', async () => {
  mock.resetPortalMock();
  await assert.rejects(mock.saveConnector('gitlab', { baseUrl: 'http://gitlab.example.com', account: '', token: 'glpat-x' }), /https/);
  await assert.rejects(mock.saveConnector('gitlab', { baseUrl: '', account: '', token: '' }), /needs a token/);
  await assert.rejects(mock.saveConnector('jira', { baseUrl: 'https://example.atlassian.net', account: 'nope', token: 't' }), /email/);
  const saved = await mock.saveConnector('gitlab', { baseUrl: 'https://gitlab.example.com/', account: '', token: 'glpat-secret' });
  assert.equal(saved.baseUrl, 'https://gitlab.example.com');
  assert.ok(!JSON.stringify(await mock.getConnectors()).includes('glpat-secret'));
  assert.equal((await mock.getPullRequests('gitlab')).status, 'identity_required');
  await mock.saveIdentities({ identities: [{ provider: 'gitlab', handle: '@alex.morgan' }] });
  assert.equal((await mock.getPullRequests('gitlab')).status, 'ok');
  await assert.rejects(mock.saveIdentities({ identities: [{ provider: 'github', handle: 'alex morgan' }] }), /not a valid GitHub username/);
});

test('mock: apps and favorites', async () => {
  mock.resetPortalMock();
  await assert.rejects(mock.createApp({ name: 'x', url: 'javascript:alert(1)', logo: '' }), /http/);
  const apps = await mock.createApp({ name: 'Runbooks', url: 'https://wiki.example.com/runbooks', logo: 'confluence' });
  assert.equal(apps.length, 1);
  await mock.deleteApp(apps[0].id);
  assert.equal((await mock.getApps()).length, 0);
  await mock.setFavorite('billing', true);
  const entities = await mock.getEntities();
  assert.ok(entities.find((e) => e.name === 'billing').favorite);
  assert.ok(entities.find((e) => e.name === 'order-service').monitors > 0);
});
