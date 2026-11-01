import assert from 'node:assert/strict';
import http from 'node:http';
import { once } from 'node:events';
import test from 'node:test';
import { forward } from './ingestion-benchmark/proxy.mjs';

test('benchmark proxy pins the upstream peer and rejects absolute request targets', async (t) => {
  let upstreamCalls = 0;
  let attackerCalls = 0;
  async function listen(handler) {
    const server = http.createServer(handler).listen(0, '127.0.0.1');
    await once(server, 'listening');
    t.after(() => new Promise((resolve) => server.close(resolve)));
    return `http://127.0.0.1:${server.address().port}`;
  }
  const upstream = await listen(async (req, res) => {
    upstreamCalls++;
    let body = '';
    for await (const part of req) body += part;
    res.end(JSON.stringify({ path: req.url, method: req.method, host: req.headers.host, body }));
  });
  const attacker = await listen((req, res) => {
    attackerCalls++;
    res.end('unexpected destination');
  });
  const proxy = new URL(await listen((req, res) => forward(req, res, upstream)));
  async function request(path, body = '') {
    return new Promise((resolve, reject) => {
      const req = http.request({ hostname: proxy.hostname, port: proxy.port, path, method: 'POST', headers: { host: 'untrusted.example.com' } }, async (res) => {
        let text = '';
        for await (const part of res) text += part;
        resolve({ status: res.statusCode, text });
      });
      req.on('error', reject);
      req.end(body);
    });
  }
  assert.equal((await request(`${attacker}/private`)).status, 400);
  assert.equal((await request(`//${new URL(attacker).host}/private`)).status, 400);
  assert.equal(attackerCalls, 0);
  assert.equal(upstreamCalls, 0);
  const result = await request('/?query=INSERT%20INTO%20logs', 'streamed rows');
  assert.equal(result.status, 200);
  assert.deepEqual(JSON.parse(result.text), { path: '/?query=INSERT%20INTO%20logs', method: 'POST', host: new URL(upstream).host, body: 'streamed rows' });
  assert.equal(upstreamCalls, 1);
  assert.equal(attackerCalls, 0);
});
