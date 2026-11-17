// The dashboard list's search syntax (web/src/api/dashboards/search.ts), as the server's `parse_search` reads it.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { addRecentSearch, parseDashboardSearch } from '../web/src/api/dashboards/search.ts';

test('search text is split into filters and free text', () => {
  assert.deepEqual(parseDashboardSearch('postgres author:"Jane Doe" team:billing favorites:true widgets.title:latency overview'), {
    text: 'postgres overview',
    authors: ['Jane Doe'],
    teams: ['billing'],
    favorites: true,
    widgetTitles: ['latency'],
  });
  const t = parseDashboardSearch('Team:checkout service:api author: Favorites:false');
  assert.equal(t.text, 'service:api');
  assert.deepEqual(t.teams, ['checkout']);
  assert.equal(t.favorites, false);
  assert.deepEqual(t.authors, []);
  assert.deepEqual(parseDashboardSearch('  '), { text: '', authors: [], teams: [], favorites: null, widgetTitles: [] });
});

test('recent searches keep the newest five, without repeats or blanks', () => {
  let list = [];
  for (const [q, at] of [
    ['postgres', 1],
    ['aws billing', 2],
    ['  ', 3],
    ['Postgres ', 4],
    ['a', 5],
    ['b', 6],
    ['c', 7],
    ['d', 8],
  ])
    list = addRecentSearch(list, q, at);
  assert.deepEqual(
    list.map((r) => r.query),
    ['d', 'c', 'b', 'a', 'Postgres'],
  );
  assert.equal(list.find((r) => r.query === 'Postgres').at, 4);
});
