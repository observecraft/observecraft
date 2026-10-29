import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import { forCluster, parseWindow, statements } from './storage-report.mjs';

const sql = readFileSync(new URL('./storage-report.sql', import.meta.url), 'utf8');

test('windows are read as seconds', () => {
  assert.equal(parseWindow('1h'), 3600);
  assert.equal(parseWindow('20m'), 1200);
  assert.equal(parseWindow('90s'), 90);
  assert.equal(parseWindow('2d'), 172800);
  assert.equal(parseWindow('600'), 600);
  assert.throws(() => parseWindow('soon'), /window/);
  assert.throws(() => parseWindow('0m'), /window/);
});

test('the report has one titled statement per section', () => {
  const parts = statements(sql);
  assert.deepEqual(
    parts.map((s) => s.title),
    ['Rows and bytes written (new parts) per table', 'Merges per table', 'CPU by query kind and merges', 'Top 10 SELECT shapes by CPU'],
  );
  for (const s of parts) {
    assert.ok(!s.sql.trimEnd().endsWith(';'), 'sent without the terminator');
    assert.match(s.sql, /\{window:UInt32\}/, `${s.title} is bounded by the window`);
    // ClickHouse 26.8 rejects `sum(rows) AS rows` when the column is used inside: aliases must differ.
    for (const [, column] of s.sql.matchAll(/\b(?:sum|avg|count|max|min|any)\((\w+)\)\s+AS\s+(\w+)/g))
      assert.ok(!new RegExp(`\\(${column}\\)\\s+AS\\s+${column}\\b`).test(s.sql), `${s.title}: alias ${column}`);
    assert.ok(!/\bAS\s+(rows|query|table|database|event_type|query_kind)\b/.test(s.sql), `${s.title}: no alias shadows a column`);
  }
});

test('a cluster reads the system tables of every replica', () => {
  const [first] = statements(sql);
  const single = forCluster(first.sql, '');
  assert.equal(single, first.sql);
  const clustered = forCluster(first.sql, 'oc');
  assert.match(clustered, /clusterAllReplicas\('oc', system\.part_log\)/);
  assert.doesNotMatch(clustered.replace(/clusterAllReplicas\('oc', system\.\w+\)/g, ''), /system\.(part_log|query_log)/);
  assert.match(forCluster(statements(sql)[3].sql, 'oc'), /clusterAllReplicas\('oc', system\.query_log\)/);
  assert.throws(() => forCluster(first.sql, "oc'); DROP TABLE x; --"), /cluster name/);
});
