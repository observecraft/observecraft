import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { convertPipeline, MOCK_PIPELINES } from './import-integration-pipelines.mjs';

const REV = 'c7dbc72cac1946294acda77f6491ecd081d9b498';

test('an integrations-core pipeline keeps its filter and processors, without OCSF sub-pipelines', () => {
  const doc = {
    id: 'nginx',
    facets: [{ name: 'Method', path: 'http.method' }],
    pipeline: {
      type: 'pipeline',
      name: 'Nginx',
      enabled: true,
      filter: { query: 'source:nginx' },
      processors: [
        { type: 'grok-parser', name: 'Parsing', source: 'message', samples: ['GET / 200'], grok: { supportRules: '', matchRules: 'r %{word:http.method} %{data}' } },
        { type: 'pipeline', name: 'OCSF', ocsf: { isOcsf: true }, filter: { query: '*' }, processors: [{ type: 'schema-processor' }] },
        { type: 'pipeline', name: 'Errors', filter: { query: 'status:error' }, processors: [{ type: 'status-remapper', sources: ['level'] }] },
      ],
    },
  };
  const p = convertPipeline(doc, 'nginx', 'assets/logs/nginx.yaml', REV);
  assert.equal(p.id, 'nginx');
  assert.equal(p.name, 'Nginx');
  assert.equal(p.filter, 'source:nginx');
  assert.deepEqual(
    p.processors.map((x) => x.name),
    ['Parsing', 'Errors'],
  );
  assert.deepEqual(p.processors[0].grok, doc.pipeline.processors[0].grok);
  assert.deepEqual(p.source, { repository: 'DataDog/integrations-core', revision: REV, path: 'nginx/assets/logs/nginx.yaml', license: 'BSD-3-Clause' });
  assert.throws(() => convertPipeline({ id: 'x' }, 'x', 'assets/logs/x.yaml', REV));
});

test('every integration’s pipelines are imported with their license, and the mock carries a few', () => {
  const dir = new URL('../server/seeds/integrations/core/', import.meta.url);
  const all = JSON.parse(readFileSync(new URL('pipelines.json', dir), 'utf8'));
  assert.ok(all.length >= 240, `${all.length} pipelines`);
  assert.equal(new Set(all.map((p) => p.id)).size, all.length, 'ids are unique');
  for (const p of all) {
    assert.ok(!JSON.stringify(p).includes('"isOcsf":true'), p.id);
    assert.equal(p.source.license, 'BSD-3-Clause');
  }
  assert.match(readFileSync(new URL('LICENSE', dir), 'utf8'), /Redistribution and use in source and binary forms/);
  const mock = JSON.parse(readFileSync(new URL('../web/src/api/pipelines/library.generated.json', import.meta.url), 'utf8'));
  assert.deepEqual(mock.map((p) => p.id).sort(), [...MOCK_PIPELINES].sort());
  for (const p of mock) {
    assert.ok(p.filter.startsWith('source:'), `${p.id} filters on its source`);
    assert.ok(p.processors.some((x) => x.type === 'grok-parser'));
  }
});

test('lookup tables written as one block of lines become the list Datadog’s API stores', () => {
  const doc = {
    pipeline: {
      type: 'pipeline',
      name: 'X',
      filter: { query: 'source:x' },
      processors: [
        { type: 'lookup-processor', source: 'severity', target: 'status', lookupTable: 'low, info\nhigh, error\n' },
        { type: 'pipeline', name: 'Nested', processors: [{ type: 'lookup-processor', source: 'a', target: 'b', lookupTable: ['1,one'] }] },
      ],
    },
  };
  const p = convertPipeline(doc, 'x', 'assets/logs/x.yaml', 'sha');
  assert.deepEqual(p.processors[0].lookupTable, ['low, info', 'high, error']);
  assert.deepEqual(p.processors[1].processors[0].lookupTable, ['1,one']);
});
