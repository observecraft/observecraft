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
const engine = await import('../web/src/api/mock/pipelineEngine.ts');
const mock = await import('../web/src/api/mock/pipelines.ts');

const sample = (message, attributes = {}, extra = {}) => ({
  message,
  status: 'info',
  service: 'api-gateway',
  host: 'i-0a1b2c3d4e5f60001',
  source: 'nginx',
  tags: {},
  attributes,
  date: null,
  traceId: '',
  ...extra,
});

test('grok preview: matchers, filters, nested attributes and rule order', () => {
  const out = engine.parseGrok({
    supportRules: '_method %{word:http.method}',
    matchRules: 'access %{ip:network.client.ip} %{_method} %{notSpace:http.url} %{integer:http.status_code} %{number:duration:scale(1000)}\nany %{data:msg}',
    samples: ['10.0.0.12 GET /cart 503 0.25', 'not an access log'],
  });
  assert.equal(out.error, null);
  assert.deepEqual(out.samples[0], { matched: true, rule: 'access', attributes: { network: { client: { ip: '10.0.0.12' } }, http: { method: 'GET', url: '/cart', status_code: 503 }, duration: 250 } });
  assert.deepEqual(out.samples[1], { matched: true, rule: 'any', attributes: { msg: 'not an access log' } });
  const kv = engine.parseGrok({ supportRules: '', matchRules: 'r %{date("yyyy-MM-dd HH:mm:ss"):ts} %{data::keyvalue}', samples: ['2026-10-25 13:00:00 user=jane order="A 1"', 'x'] });
  assert.deepEqual(kv.samples[0].attributes, { ts: Date.UTC(2026, 9, 25, 13), user: 'jane', order: 'A 1' });
  assert.equal(kv.samples[1].matched, false);
  const bad = engine.parseGrok({ supportRules: '', matchRules: 'r %{nope:x}', samples: ['x'] });
  assert.match(bad.error, /nope/);
});

test('test endpoint: processors run on the sample when the filter matches', () => {
  const processors = [
    { type: 'grok-parser', grok: { matchRules: 'r %{word:level} %{integer:http.status_code} %{data:msg}' } },
    { type: 'category-processor', target: 'http.status_category', categories: [{ filter: { query: '@http.status_code:[500 TO 599]' }, name: 'error' }] },
    { type: 'status-remapper', sources: ['level'] },
    { type: 'message-remapper', sources: ['msg'] },
    { type: 'attribute-remapper', sources: ['http.status_code'], target: 'code', targetFormat: 'string', preserveSource: true },
    { type: 'string-builder-processor', template: '%{level}/%{code}', target: 'summary' },
    { type: 'arithmetic-processor', expression: 'http.status_code * 2 - 6', target: 'math' },
    { type: 'lookup-processor', source: 'code', target: 'meaning', lookupTable: ['503,Service Unavailable'] },
    { type: 'user-agent-parser', sources: ['ua'] },
  ];
  const out = mock.testPipeline({ filter: 'source:nginx', processors, log: sample('WARN 503 upstream down') });
  assert.equal(out.matched, true);
  assert.equal(out.log.status, 'warn');
  assert.equal(out.log.message, 'upstream down');
  assert.equal(out.log.attributes.http.status_category, 'error');
  assert.equal(out.log.attributes.code, '503');
  assert.equal(out.log.attributes.summary, 'WARN/503');
  assert.equal(out.log.attributes.math, 1000);
  assert.equal(out.log.attributes.meaning, 'Service Unavailable');
  const miss = mock.testPipeline({ filter: 'source:redis', processors, log: sample('WARN 503 x') });
  assert.equal(miss.matched, false);
  assert.equal(miss.log.message, 'WARN 503 x');
});

test('test endpoint: a parsed message stays an attribute until remapped; GeoIP and array processors, as the server', () => {
  const processors = [
    { type: 'grok-parser', grok: { matchRules: 'r %{ip:network.client.ip} %{data:message}' } },
    { type: 'geo-ip-parser', sources: ['network.client.ip'], target: 'network.client.geoip' },
    { type: 'geo-ip-parser', sources: ['peer'], target: 'peer_geoip' },
    { type: 'array-processor', operation: { type: 'select', source: 'entries', target: 'outcome', filter: 'name:login', valueToExtract: 'outcome' } },
    { type: 'array-processor', operation: { type: 'length', source: 'entries', target: 'count' } },
  ];
  const attributes = {
    peer: 'unknown',
    entries: [
      { name: 'check', outcome: 'skip' },
      { name: 'login', outcome: 'ok' },
    ],
  };
  const out = mock.testPipeline({ processors, log: sample('10.0.0.12 card declined', attributes) });
  assert.equal(out.log.message, '10.0.0.12 card declined');
  assert.equal(out.log.attributes.message, 'card declined');
  assert.deepEqual(out.log.attributes.network.client.geoip, {});
  assert.deepEqual(out.log.attributes.peer_geoip, { invalidAddress: 'unknown' });
  assert.equal(out.log.attributes.outcome, 'ok');
  assert.equal(out.log.attributes.count, 2);
  const remapped = mock.testPipeline({ processors: [...processors, { type: 'message-remapper', sources: ['message'] }], log: sample('10.0.0.12 card declined') });
  assert.equal(remapped.log.message, 'card declined');
  assert.equal(remapped.log.attributes.message, undefined);
  const url = mock.testPipeline({ processors: [{ type: 'url-parser', sources: ['u'], target: 'd' }], log: sample('m', { u: 'https://Shop.Example.com:8443/cart?q=red%20shoes#top' }) });
  assert.deepEqual(url.log.attributes.d, { scheme: 'https', host: 'shop.example.com', port: 8443, path: '/cart', queryString: { q: 'red%20shoes' }, hash: 'top' });
});

test('mock pipelines: deterministic demo seed, CRUD, reorder and read-only integration pipelines', async () => {
  const first = await mock.listPipelines();
  assert.ok(first.length >= 4);
  assert.deepEqual(
    first.map((p) => p.position),
    first.map((_, i) => i),
  );
  assert.ok(first.some((p) => p.readOnly && p.source === 'integration:nginx'));
  for (const p of first) assert.match(p.filter, /service:|source:/);

  const created = await mock.createPipeline({ name: '  Checkout errors ', filter: 'service:order-service status:error', enabled: true, processors: [{ type: 'status-remapper', sources: ['level'] }] });
  assert.equal(created.name, 'Checkout errors');
  assert.equal(created.position, first.length);
  await assert.rejects(mock.createPipeline({ name: 'bad', filter: '', enabled: true, processors: [{ type: 'grok-parser', name: 'broken', grok: { matchRules: 'r %{nope}' } }] }), /broken/);
  await assert.rejects(mock.createPipeline({ name: ' ', filter: '', enabled: true, processors: [] }), /1–100/);

  const updated = await mock.updatePipeline(created.id, { ...created, name: 'Checkout', enabled: false });
  assert.equal(updated.name, 'Checkout');
  assert.equal((await mock.setPipelineEnabled(created.id, true)).enabled, true);
  const integration = first.find((p) => p.readOnly);
  await assert.rejects(mock.updatePipeline(integration.id, { name: 'x', filter: '', enabled: true, processors: [] }), /read-only/);
  assert.equal((await mock.setPipelineEnabled(integration.id, false)).enabled, false);

  const ids = (await mock.listPipelines()).map((p) => p.id).reverse();
  await mock.reorderPipelines(ids);
  assert.deepEqual(
    (await mock.listPipelines()).map((p) => p.id),
    ids,
  );
  await mock.deletePipeline(created.id);
  assert.ok(!(await mock.listPipelines()).some((p) => p.id === created.id));
});

test('mock integration library: installed flags and installing once', async () => {
  const lib = await mock.listLibrary();
  assert.ok(lib.length >= 5);
  assert.equal(lib.find((p) => p.id === 'nginx').installed, true);
  const redis = lib.find((p) => p.id === 'redisdb');
  assert.equal(redis.installed, false);
  assert.equal(redis.upstream.license, 'BSD-3-Clause');
  const installed = await mock.installLibraryPipeline('redisdb');
  assert.equal(installed.readOnly, true);
  assert.equal(installed.filter, 'source:redis');
  await assert.rejects(mock.installLibraryPipeline('redisdb'), /already installed/);
  assert.equal((await mock.listLibrary()).find((p) => p.id === 'redisdb').installed, true);
});

test('the mock engine compiles every integration pipeline and parses its grok samples', async () => {
  const groks = (list) => list.flatMap((p) => (p.type === 'grok-parser' ? [p] : p.type === 'pipeline' ? groks(p.processors) : []));
  for (const lib of await mock.listLibrary()) {
    assert.deepEqual(engine.processorErrors(lib.filter, lib.processors), [], lib.id);
    for (const g of groks(lib.processors)) {
      const out = engine.parseGrok({ supportRules: g.grok.supportRules ?? '', matchRules: g.grok.matchRules ?? '', samples: g.samples ?? [] });
      assert.equal(out.error, null, `${lib.id} / ${g.name}`);
      out.samples.forEach((s, i) => assert.ok(s.matched, `${lib.id} / ${g.name}: ${g.samples[i]}`));
    }
  }
});

test('the mock runs the imported nginx pipeline on an access log', () => {
  const nginx = mock.libraryPipeline('nginx');
  const out = mock.testPipeline({ filter: nginx.filter, processors: nginx.processors, log: sample('10.0.0.12 - - [25/Oct/2026:12:59:58 +0000] "GET /cart?id=7 HTTP/1.1" 503 612 "-" "curl/8.4.0"') });
  assert.equal(out.matched, true);
  assert.equal(out.log.attributes.http.status_code, 503);
  assert.equal(out.log.attributes.http.url_details.path, '/cart');
  assert.equal(out.log.status, 'error');
  assert.equal(out.log.date, '2026-10-25T12:59:58.000Z');
});

test('a parsed `message` is an attribute; a message remapper makes it the message (as Datadog’s engine)', () => {
  const redis = mock.libraryPipeline('redisdb');
  const line = '1:M 25 Oct 2026 12:59:58.000 # WARNING overcommit_memory is set to 0';
  const out = mock.testPipeline({ filter: redis.filter, processors: redis.processors, log: sample(line, {}, { source: 'redis' }) });
  assert.equal(out.log.message, line);
  assert.equal(out.log.attributes.message, 'WARNING overcommit_memory is set to 0');
  assert.equal(out.log.status, 'warn');
  const etcd = mock.libraryPipeline('etcd');
  const parsed = mock.testPipeline({ filter: etcd.filter, processors: etcd.processors, log: sample('2026-10-25 12:59:58.123456 E | rafthttp: failed to dial peer', {}, { source: 'etcd' }) });
  assert.equal(parsed.log.message, 'failed to dial peer');
  assert.equal(parsed.log.attributes.message, undefined);
});

test('grok: `.` matches line breaks and a final line break ends the log, as in Datadog’s parser (and the server’s)', () => {
  const stack = 'java.net.UnknownHostException: zookeeper: Name or service not known\n\tat java.net.Inet4AddressImpl.lookupAllHostAddr(Native Method)\n\tat Main.run(Main.java:1)';
  const rules = engine.compileGrok('', 'error_rule %{notSpace:error.kind}: %{data:error.message}(\\n|\\t).*');
  assert.deepEqual(engine.grokParse(rules, stack, 0)?.values, [
    ['error.kind', 'java.net.UnknownHostException'],
    ['error.message', 'zookeeper: Name or service not known'],
  ]);
  assert.deepEqual(engine.grokParse(engine.compileGrok('', 'rule %{word:a} %{data:b}'), 'x tail\n', 0)?.values, [
    ['a', 'x'],
    ['b', 'tail'],
  ]);
});
