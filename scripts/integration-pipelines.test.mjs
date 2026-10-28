import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { convertPipeline, PIPELINE_SOURCES } from './import-integration-pipelines.mjs';

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

test('every listed integration pipeline is imported with its license', () => {
  for (const integration of Object.keys(PIPELINE_SOURCES)) {
    const dir = new URL(`../server/seeds/integrations/${integration}/`, import.meta.url);
    const p = JSON.parse(readFileSync(new URL('pipeline.json', dir), 'utf8'));
    assert.equal(p.id, integration);
    assert.ok(p.filter.startsWith('source:'), `${integration} filters on its source`);
    assert.ok(p.processors.some((x) => x.type === 'grok-parser'));
    assert.ok(!JSON.stringify(p).includes('"isOcsf":true'));
    assert.match(readFileSync(new URL('LICENSE', dir), 'utf8'), /Redistribution and use in source and binary forms/);
  }
});
