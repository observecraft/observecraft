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
const inc = await import('../web/src/api/mock/incidents.ts');
const id = await import('../web/src/api/mock/identity.ts');

const DAY = 86_400_000;
const now = Date.now();
const lastYear = { from: now - 365 * DAY, to: now + DAY };
const search = (extra = {}) => inc.searchIncidents({ query: '', statuses: [], severities: [], mine: false, teams: [], range: lastYear, offset: 0, limit: 200, ...extra });
const tally = (list, key) => {
  const out = {};
  for (const i of list) for (const v of [i[key]].flat()) out[v] = (out[v] ?? 0) + 1;
  return out;
};

test('incident search lists open incidents first, then newest, with consistent facet counts', async () => {
  const page = await search();
  assert.ok(page.total >= 15, 'a realistic number of incidents');
  assert.equal(page.incidents.length, page.total);
  const order = page.incidents.map((i) => [i.status === 'resolved' ? 1 : 0, -i.declaredAt]);
  assert.deepEqual(
    order,
    [...order].sort((a, b) => a[0] - b[0] || a[1] - b[1]),
  );
  assert.equal(page.counts.active + page.counts.stable + page.counts.resolved, page.total);
  assert.ok(page.counts.active > 0 && page.counts.stable > 0 && page.counts.resolved > 0);
  assert.deepEqual(page.severityCounts, tally(page.incidents, 'severity'));
  assert.deepEqual(page.teamCounts, tally(page.incidents, 'teams'));
  for (const i of page.incidents) {
    assert.ok(Number.isInteger(i.id) && i.id > 0);
    assert.ok(i.detectedAt <= i.declaredAt);
    assert.equal(i.resolvedAt != null, i.status === 'resolved');
  }
  assert.equal(new Set(page.incidents.map((i) => i.id)).size, page.total, 'IR numbers are unique');
});

test('status, severity and team filters; each facet ignores only its own filter', async () => {
  const all = await search();
  const resolved = await search({ statuses: ['resolved'] });
  assert.ok(resolved.incidents.every((i) => i.status === 'resolved'));
  assert.equal(resolved.total, all.counts.resolved);
  assert.deepEqual(resolved.counts, all.counts, 'status counts ignore the status filter');
  assert.deepEqual(resolved.severityCounts, tally(resolved.incidents, 'severity'), 'severity counts apply the status filter');

  const sev = all.incidents[0].severity;
  const bySev = await search({ severities: [sev] });
  assert.ok(bySev.total > 0 && bySev.incidents.every((i) => i.severity === sev));
  assert.deepEqual(bySev.severityCounts, all.severityCounts, 'severity counts ignore the severity filter');
  assert.deepEqual(bySev.teamCounts, tally(bySev.incidents, 'teams'), 'team counts apply the severity filter');

  const team = all.incidents[0].teams[0];
  const byTeam = await search({ teams: [team] });
  assert.ok(byTeam.total > 0 && byTeam.incidents.every((i) => i.teams.includes(team)));
  assert.deepEqual(byTeam.teamCounts, all.teamCounts, 'team counts ignore the team filter');
  assert.deepEqual(byTeam.severityCounts, tally(byTeam.incidents, 'severity'));
});

test('text search matches title, IR number, services and teams; range and paging apply', async () => {
  const all = await search();
  const target = all.incidents[3];
  const byNumber = await search({ query: `IR-${target.id}` });
  assert.ok(byNumber.incidents.some((i) => i.id === target.id));
  const billing = await search({ query: 'billing' });
  assert.ok(billing.total > 0);
  for (const i of billing.incidents) assert.match(`${i.title} ${i.services.join(' ')} ${i.teams.join(' ')}`.toLowerCase(), /billing/);

  const recent = await search({ range: { from: now - 10 * DAY, to: now + DAY } });
  assert.ok(recent.total > 0 && recent.total < all.total);
  assert.ok(recent.incidents.every((i) => i.declaredAt >= now - 10 * DAY));

  const second = await search({ offset: 5, limit: 5 });
  assert.equal(second.total, all.total);
  assert.deepEqual(
    second.incidents.map((i) => i.id),
    all.incidents.slice(5, 10).map((i) => i.id),
  );
});

test('"mine" keeps incidents the current user commands or responds to', async () => {
  const me = await id.getProfile();
  const all = await search();
  const mine = await search({ mine: true });
  assert.ok(mine.total > 0 && mine.total < all.total);
  for (const s of mine.incidents) {
    const d = await inc.getIncident(s.id);
    assert.ok(d.commander?.id === me.id || d.responders.some((r) => r.id === me.id), `IR-${s.id} involves me`);
  }
});

test('declaring an incident takes the next IR number, appears in search and starts the timeline', async () => {
  const me = await id.getProfile();
  const before = await search();
  const next = Math.max(...before.incidents.map((i) => i.id)) + 1;
  const d = await inc.declareIncident({ title: '  [Checkout] Cart totals off by one cent  ', severity: 'SEV-2', services: ['order-service'], teams: ['checkout'] });
  assert.equal(d.id, next);
  assert.equal(d.title, '[Checkout] Cart totals off by one cent');
  assert.equal(d.status, 'active');
  assert.equal(d.commander?.id, me.id, 'the declaring user commands by default');
  assert.equal(d.declaredBy, me.name);
  assert.equal(d.incidentType, 'Production Incident');
  assert.equal(d.detectionMethod, 'employee');

  const after = await search();
  assert.equal(after.total, before.total + 1);
  assert.equal(after.incidents[0].id, next, 'newest open incident first');
  assert.equal(after.counts.active, before.counts.active + 1);

  const timeline = await inc.getTimeline(next);
  assert.equal(timeline[0].kind, 'declared');
  assert.equal(timeline[0].body, 'Incident declared with severity SEV-2');
  assert.equal(timeline[0].authorId, me.id);

  await assert.rejects(inc.declareIncident({ title: 'x', severity: 'SEV-9' }), /severity/);
  await assert.rejects(inc.declareIncident({ title: '   ', severity: 'SEV-1' }), /title/);
  await assert.rejects(inc.getIncident(99_999), /IR-99999 not found/);
});

test('status, severity and commander changes are written to the timeline', async () => {
  const d = await inc.declareIncident({ title: '[Catalog] Search facets missing', severity: 'SEV-3', teams: ['catalog'] });
  const users = await id.searchUsers({ query: '', statuses: ['active'], offset: 0, limit: 200 });
  const other = users.users.find((u) => u.id !== d.commander?.id);

  const stable = await inc.updateIncident(d.id, { status: 'stable', severity: 'SEV-2', commanderId: other.id });
  assert.equal(stable.status, 'stable');
  assert.equal(stable.severity, 'SEV-2');
  assert.equal(stable.commander?.id, other.id);
  assert.ok(stable.stableAt && !stable.resolvedAt && stable.acknowledgedAt);

  const resolved = await inc.updateIncident(d.id, { status: 'resolved' });
  assert.ok(resolved.resolvedAt);
  await inc.updateIncident(d.id, { status: 'resolved', severity: 'SEV-2' }); // no change, no entry

  const bodies = (await inc.getTimeline(d.id)).map((e) => `${e.kind}: ${e.body}`);
  assert.deepEqual(bodies.slice(-4), [
    'status: Status changed from Active to Stable',
    'severity: Severity changed from SEV-3 to SEV-2',
    `commander: ${other.name} is now the incident commander`,
    'status: Status changed from Stable to Resolved',
  ]);

  await inc.addNote(d.id, 'Rolled back catalog-service 2.8.1');
  const last = (await inc.getTimeline(d.id)).at(-1);
  assert.equal(last.kind, 'note');
  await inc.setImportant(d.id, last.id, true);
  assert.equal((await inc.getTimeline(d.id)).at(-1).important, true);
});

test('response team, pages, documents, follow-ups and the postmortem', async () => {
  const d = await inc.declareIncident({ title: '[Billing] Refunds stuck in pending', severity: 'SEV-3', teams: ['billing'] });
  const users = await id.searchUsers({ query: '', statuses: ['active'], offset: 0, limit: 200 });
  const helper = users.users.find((u) => u.id !== d.commander?.id);
  await inc.addResponder(d.id, helper.id, 'scribe');
  const team = await inc.listResponders(d.id);
  assert.equal(team[0].role, 'incident_commander');
  assert.ok(team.some((m) => m.userId === helper.id && m.role === 'scribe'));
  assert.ok((await inc.getIncident(d.id)).responders.some((r) => r.id === helper.id));
  await inc.removeResponder(d.id, helper.id);
  assert.ok(!(await inc.listResponders(d.id)).some((m) => m.userId === helper.id));

  const page = await inc.createPage(d.id, 'billing', 'Refund worker down');
  assert.equal(page.status, 'triggered');
  await inc.updatePage(d.id, page.id, 'acknowledged');
  assert.equal((await inc.listPages(d.id))[0].status, 'acknowledged');
  await assert.rejects(inc.createPage(d.id, 'no-such-team', 'x'), /unknown team/);

  await inc.addDocument(d.id, 'Runbook', 'https://wiki.example.com/billing/refunds');
  await assert.rejects(inc.addDocument(d.id, 'Bad', 'javascript:alert(1)'), /http/);
  const docs = await inc.listDocuments(d.id);
  assert.equal(docs.length, 1);
  await inc.addFollowup(d.id, 'Alert on refund queue age');
  const fu = (await inc.getIncident(d.id)).followups[0];
  await inc.setFollowupDone(d.id, fu.id, true);
  assert.equal((await inc.getIncident(d.id)).followups[0].done, true);

  await inc.createNotification(d.id, ['#billing-alerts'], 'Refunds delayed', 'Investigating');
  assert.equal((await inc.listNotifications(d.id))[0].subject, 'Refunds delayed');

  assert.equal(await inc.getPostmortem(d.id), null);
  const pm = await inc.generatePostmortem(d.id, 'standard');
  assert.match(pm.content, new RegExp(`^# Postmortem IR-${d.id}: \\[Billing\\] Refunds stuck in pending`));
  assert.match(pm.content, /- \[x\] Alert on refund queue age/);
  assert.match(pm.content, /\[Runbook\]\(https:\/\/wiki\.example\.com\/billing\/refunds\)/);
  await inc.savePostmortem(d.id, '# Edited');
  assert.equal((await inc.getPostmortem(d.id)).content, '# Edited');
});

test('API keys and application keys show the secret only at creation', async () => {
  const created = await id.createIngestKey('prod-collector');
  assert.match(created.key, /^ocik_[A-Za-z0-9_-]{40,}$/);
  assert.equal(created.prefix, created.key.slice(0, 10));
  const listed = await id.listIngestKeys();
  assert.equal(listed[0].id, created.id, 'newest first');
  assert.ok(!JSON.stringify(listed).includes(created.key), 'the secret is not listed');
  assert.ok(listed.every((k) => !('key' in k)));
  await id.revokeIngestKey(created.id);
  // A revoked key stays listed, marked revoked (the page's Status filter shows it).
  assert.ok((await id.listIngestKeys()).find((k) => k.id === created.id)?.revokedAt);
  await assert.rejects(id.revokeIngestKey(created.id), /not found/);
  await assert.rejects(id.createIngestKey('  '), /name/);

  const app = await id.createAppKey('terraform');
  assert.match(app.key, /^ocak_/);
  const apps = await id.listAppKeys();
  assert.ok(apps.some((k) => k.id === app.id && k.prefix === app.key.slice(0, 10)));
  assert.ok(!JSON.stringify(apps).includes(app.key));
});

test('invited users appear as pending with their roles; duplicates are skipped', async () => {
  const roles = await id.listRoles();
  const standard = roles.find((r) => r.id.endsWith('-standard'));
  const before = standard.userCount;
  const sent = await id.inviteUsers([' New.Person@Example.com '], [standard.id]);
  assert.equal(sent.length, 1);
  assert.equal(sent[0].email, 'new.person@example.com');
  assert.match(sent[0].url, /\/login\/link\//);
  const found = await id.searchUsers({ query: 'new.person', offset: 0, limit: 50 });
  assert.equal(found.total, 1);
  assert.equal(found.users[0].status, 'pending');
  assert.equal(found.users[0].name, 'new person');
  assert.deepEqual(
    found.users[0].roles.map((r) => r.id),
    [standard.id],
  );
  assert.equal((await id.listRoles()).find((r) => r.id === standard.id).userCount, before + 1);
  const pending = await id.searchUsers({ query: '', statuses: ['pending'], offset: 0, limit: 200 });
  assert.ok(pending.users.some((u) => u.email === 'new.person@example.com'));
  assert.deepEqual(await id.inviteUsers(['new.person@example.com'], []), [], 'already a member');
  await assert.rejects(id.inviteUsers(['not-an-email'], []), /valid email/);
});

test('profile, users and teams share one directory', async () => {
  const me = await id.getProfile();
  assert.match(me.email, /@example\.com$/);
  assert.ok(
    me.roles.some((r) => r.id === `${me.org.id}-admin`),
    'the mock user is an admin',
  );
  const names = (await id.searchUsers({ query: '', offset: 0, limit: 200 })).users.map((u) => u.name);
  for (const n of ['Alex Morgan', 'Priya Shah', 'Daniel Kim', 'Sara Levi', 'Omar Haddad']) assert.ok(names.includes(n), n);
  assert.deepEqual(
    names,
    [...names].sort((a, b) => a.toLowerCase().localeCompare(b.toLowerCase())),
  );
  const updated = await id.updateProfile({ title: 'Staff Engineer' });
  assert.equal(updated.title, 'Staff Engineer');
  await assert.rejects(id.updateUser(me.id, { status: 'deactivated' }), /yourself/);

  await id.createTeam({ handle: 'Growth', name: 'Growth', description: 'Experiments' });
  const teams = await id.searchTeams({ query: 'growth', sort: 'name', offset: 0, limit: 50 });
  assert.equal(teams.teams[0].handle, 'growth');
  assert.equal(teams.teams[0].memberCount, 1);
  assert.equal(teams.teams[0].members[0].id, me.id);
  await assert.rejects(id.createTeam({ handle: 'growth', name: 'Growth' }), /already exists/);
  const mine = await id.searchTeams({ member: 'me', sort: 'name', offset: 0, limit: 50 });
  assert.ok(mine.teams.some((t) => t.handle === 'growth'));
  assert.ok((await id.getProfile()).teams.some((t) => t.handle === 'growth'));
  for (const t of ['payments', 'catalog', 'platform'])
    assert.ok(
      (await id.searchTeams({ query: t, sort: 'name', offset: 0, limit: 50 })).teams.some((x) => x.handle === t),
      t,
    );
});

test('data retention and log indexes (Organization Settings → Data Retention)', async () => {
  const ix = await import('../web/src/api/mock/logIndexes.ts');
  const s = await ix.dataSettings();
  assert.ok(s.spanRetentionDays <= s.maxSpanRetentionDays);
  assert.ok(s.metricRetentionTiers.includes(s.metrics.raw));
  const changed = await ix.updateDataSettings({ spanRetentionDays: 7 });
  assert.equal(changed.spanRetentionDays, 7);
  await assert.rejects(ix.updateDataSettings({ spanRetentionDays: s.maxSpanRetentionDays + 1 }), /Span retention/);
  await assert.rejects(ix.updateDataSettings({ metrics: { raw: 30, rollup5m: 15, rollup1h: 90 } }), /finer resolution/);

  const before = await ix.listIndexes();
  assert.ok(before.indexes.length > 0 && before.maxIndexes >= before.indexes.length);
  const made = await ix.createIndex({ name: ' Audit ', filter: 'service:billing', retentionDays: 30 });
  assert.equal(made.name, 'audit');
  assert.equal(made.position, before.indexes.length);
  await assert.rejects(ix.createIndex({ name: 'audit', retentionDays: 30 }), /already exists/);
  await assert.rejects(ix.createIndex({ name: 'x', retentionDays: before.maxRetentionDays + 1 }), /Retention/);
  const ids = (await ix.listIndexes()).indexes.map((i) => i.id).reverse();
  await ix.reorderIndexes(ids);
  assert.deepEqual(
    (await ix.listIndexes()).indexes.map((i) => i.id),
    ids,
  );
  await ix.deleteIndex(made.id);
  assert.ok(!(await ix.listIndexes()).indexes.some((i) => i.id === made.id));
});
