import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer, request, type Server } from 'node:http';
import { connect } from 'node:net';
import type { AddressInfo } from 'node:net';
import { BLOCKED_HEADER, startProxy } from '../src/proxy.ts';

function listen(server: Server): Promise<number> {
  return new Promise((done) => server.listen(0, '127.0.0.1', () => done((server.address() as AddressInfo).port)));
}

async function target(): Promise<{ port: number; close: () => void }> {
  // Answers with a fixed text per known path (never the request's own text).
  const paths = new Set(['/page', '/tunnelled']);
  const server = createServer((req, res) => res.end(paths.has(req.url ?? '') ? `hello ${req.url === '/page' ? '/page' : '/tunnelled'}` : 'hello'));
  const port = await listen(server);
  return { port, close: () => server.close() };
}

/** A plain HTTP request through the proxy (absolute-form URL). */
function viaProxy(proxyPort: number, url: string): Promise<{ status: number; body: string; blocked?: boolean }> {
  return new Promise((done, fail) => {
    const req = request({ host: '127.0.0.1', port: proxyPort, method: 'GET', path: url }, (res) => {
      let body = '';
      res.on('data', (c) => (body += c));
      res.on('end', () => done({ status: res.statusCode ?? 0, body, ...(res.headers[BLOCKED_HEADER] ? { blocked: true } : {}) }));
    });
    req.on('error', fail);
    req.end();
  });
}

/** A CONNECT through the proxy; resolves with the status line and, once tunnelled, a GET's answer. */
function tunnel(proxyPort: number, host: string, port: number): Promise<string> {
  return new Promise((done, fail) => {
    const socket = connect(proxyPort, '127.0.0.1', () => socket.write(`CONNECT ${host}:${port} HTTP/1.1\r\nHost: ${host}:${port}\r\n\r\n`));
    let seen = '';
    socket.on('data', (c) => {
      seen += c.toString();
      if (seen.startsWith('HTTP/1.1 200') && !seen.includes('hello')) socket.write(`GET /tunnelled HTTP/1.1\r\nHost: ${host}\r\nConnection: close\r\n\r\n`);
    });
    socket.on('end', () => done(seen));
    socket.on('error', fail);
  });
}

test('internal addresses are refused, plain and tunnelled', async () => {
  const t = await target();
  const blocked: string[] = [];
  const proxy = await startProxy({ allowPrivate: false, onBlocked: (url, reason) => blocked.push(`${url} ${reason}`) });
  try {
    const plain = await viaProxy(proxy.port, `http://127.0.0.1:${t.port}/secret`);
    assert.equal(plain.status, 403);
    assert.match(plain.body, /loopback/);
    assert.equal(plain.blocked, true);
    const tunnelled = await tunnel(proxy.port, '127.0.0.1', t.port);
    assert.match(tunnelled, /^HTTP\/1.1 403/);
    const named = await viaProxy(proxy.port, `http://localhost:${t.port}/`);
    assert.equal(named.status, 403);
    assert.equal(blocked.length, 3);
    assert.match(blocked[0], /^http:\/\/127\.0\.0\.1:\d+\/secret .*loopback/);
  } finally {
    await proxy.close();
    t.close();
  }
});

test('allowed targets are reached, plain and tunnelled', async () => {
  const t = await target();
  const proxy = await startProxy({ allowPrivate: true });
  try {
    const plain = await viaProxy(proxy.port, `http://127.0.0.1:${t.port}/page`);
    assert.deepEqual(plain, { status: 200, body: 'hello /page' });
    const tunnelled = await tunnel(proxy.port, 'localhost', t.port);
    assert.match(tunnelled, /^HTTP\/1.1 200/);
    assert.match(tunnelled, /hello \/tunnelled/);
  } finally {
    await proxy.close();
    t.close();
  }
});

test('the proxy listens on loopback only and answers bad requests', async () => {
  const proxy = await startProxy({ allowPrivate: true });
  try {
    assert.equal(proxy.host, '127.0.0.1');
    const bad = await viaProxy(proxy.port, '/not-absolute');
    assert.equal(bad.status, 400);
    const ftp = await viaProxy(proxy.port, 'ftp://example.com/file');
    assert.equal(ftp.status, 400);
  } finally {
    await proxy.close();
  }
});
