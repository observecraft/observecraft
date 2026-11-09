import { after, before, test } from 'node:test';
import assert from 'node:assert/strict';
import type { AddressInfo } from 'node:net';
import type { Server } from 'node:http';
import { createApp } from '../src/server.ts';
import type { RunRequest, RunResult, Runner } from '../src/run.ts';
import type { Sessions } from '../src/session.ts';

// The HTTP layer with a scripted runner: the browser itself is covered by run.test.ts.
let server: Server;
let base = '';
let release: (() => void) | null = null;
const seen: RunRequest[] = [];

const runner: Runner = {
  async run(request) {
    seen.push(request);
    if (request.startUrl.includes('slow')) await new Promise<void>((done) => (release = done));
    return {
      passed: true,
      durationMs: 1,
      startUrl: request.startUrl,
      browserVersion: '158',
      device: { id: 'chrome.laptop_large', width: 1440, height: 1100, isMobile: false },
      steps: [],
    } satisfies RunResult;
  },
  close: async () => {},
};

const acted: unknown[] = [];
const sessions: Sessions = {
  async open(req) {
    if (req.startUrl.includes('busy')) throw new Error('Every browser session is busy');
    return { id: 's1', ok: true, urlBefore: '', url: req.startUrl, title: 'Shop', snapshot: 'URL: x', requests: [] };
  },
  async act(id, action, variables) {
    if (id !== 's1') throw new Error(`Unknown or expired session ${id}`);
    acted.push({ action, variables });
    return { ok: true, urlBefore: 'u', url: 'u', title: 't', snapshot: 's', requests: [] };
  },
  async end() {},
  async close() {},
};

before(async () => {
  server = createApp({ runner, sessions, token: 'secret-token', concurrency: 1 });
  await new Promise<void>((done) => server.listen(0, '127.0.0.1', () => done()));
  base = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
});

after(() => server.close());

const run = (body: unknown, token = 'secret-token') =>
  fetch(`${base}/v1/runs`, { method: 'POST', headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' }, body: JSON.stringify(body) });

test('runs need the token', async () => {
  assert.equal((await run({ startUrl: 'https://shop.example.com', steps: [] }, 'wrong')).status, 401);
  const noHeader = await fetch(`${base}/v1/runs`, { method: 'POST', body: '{}' });
  assert.equal(noHeader.status, 401);
});

test('a run answers the runner result', async () => {
  const res = await run({ startUrl: 'https://shop.example.com', steps: [{ name: 'Reload', type: 'refresh' }], variables: { A: 'b' } });
  assert.equal(res.status, 200);
  const body = (await res.json()) as RunResult;
  assert.equal(body.passed, true);
  assert.deepEqual(seen.at(-1)?.variables, { A: 'b' });
});

test('bad requests are refused', async () => {
  for (const body of [
    {},
    { startUrl: 'file:///etc/passwd', steps: [] },
    { startUrl: 'https://shop.example.com', steps: 'x' },
    { startUrl: 'https://shop.example.com', steps: new Array(101).fill({ type: 'refresh' }) },
  ]) {
    assert.equal((await run(body)).status, 400, JSON.stringify(body).slice(0, 80));
  }
  const notJson = await fetch(`${base}/v1/runs`, { method: 'POST', headers: { authorization: 'Bearer secret-token' }, body: 'nope' });
  assert.equal(notJson.status, 400);
  assert.equal((await fetch(`${base}/v1/nothing`, { headers: { authorization: 'Bearer secret-token' } })).status, 404);
});

test('a busy runner answers 503 with Retry-After', async () => {
  const slow = run({ startUrl: 'https://slow.example.com', steps: [] });
  while (!release) await new Promise((r) => setTimeout(r, 10));
  const busy = await run({ startUrl: 'https://shop.example.com', steps: [] });
  assert.equal(busy.status, 503);
  assert.ok(busy.headers.get('retry-after'));
  release();
  assert.equal((await slow).status, 200);
});

test('health needs no token', async () => {
  const res = await fetch(`${base}/healthz`);
  assert.equal(res.status, 200);
});

test('agent sessions open, act and end', async () => {
  const call = (method: string, path: string, body?: unknown) =>
    fetch(`${base}${path}`, { method, headers: { authorization: 'Bearer secret-token', 'content-type': 'application/json' }, body: body === undefined ? undefined : JSON.stringify(body) });
  const opened = await call('POST', '/v1/sessions', { startUrl: 'https://shop.example.com' });
  assert.equal(opened.status, 200);
  assert.equal(((await opened.json()) as { id: string }).id, 's1');
  assert.equal((await call('POST', '/v1/sessions', { startUrl: 'https://busy.example.com' })).status, 503);
  assert.equal((await call('POST', '/v1/sessions', { startUrl: 'file:///etc/passwd' })).status, 400);
  const act = await call('POST', '/v1/sessions/s1/actions', { action: { type: 'click', ref: 'e3' }, variables: { USER: 'qa' } });
  assert.equal(act.status, 200);
  assert.deepEqual(acted.at(-1), { action: { type: 'click', ref: 'e3' }, variables: { USER: 'qa' } });
  assert.equal((await call('POST', '/v1/sessions/nope/actions', { action: { type: 'snapshot' } })).status, 404);
  assert.equal((await call('POST', '/v1/sessions/s1/actions', { action: { type: 'teleport' } })).status, 400);
  assert.equal((await call('DELETE', '/v1/sessions/s1')).status, 204);
  assert.equal((await fetch(`${base}/v1/sessions`, { method: 'POST', body: '{}' })).status, 401);
});
