// Test-only streaming proxy. Delays/holds INSERTs, leaving reads available for observations.
import http from 'node:http';
import { pipeline } from 'node:stream';
import { pathToFileURL } from 'node:url';

let delayMs = 0;
let hold = false;
let active = 0;
let peakActive = 0;
let held = 0;
let inserts = 0;
let failures = 0;
const waiters = new Set();
const stats = () => ({ delayMs, hold, active, peakActive, held, inserts, failures });
export function forward(req, res, destination, delayed = false) {
  // Only the configured destination selects the peer. Never resolve an absolute request URL
  // against it: URL resolution lets a caller replace the destination's authority.
  if (!req.url?.startsWith('/') || req.url.startsWith('//')) {
    res.writeHead(400).end('Expected an origin-form request target');
    return;
  }
  const target = new URL(destination);
  active++;
  peakActive = Math.max(peakActive, active);
  const begin = () => {
    if (req.destroyed || res.destroyed) return;
    const upstream = http.request({ hostname: target.hostname, port: target.port, path: req.url, method: req.method, headers: { ...req.headers, host: target.host } }, (reply) => {
      res.writeHead(reply.statusCode, reply.headers);
      pipeline(reply, res, () => {});
    });
    upstream.on('error', () => {
      failures++;
      if (!res.headersSent) res.writeHead(503);
      res.end();
    });
    res.on('close', () => upstream.destroy());
    pipeline(req, upstream, () => {});
  };
  const schedule = () => {
    if (delayed && delayMs) setTimeout(begin, delayMs);
    else begin();
  };
  let waiting;
  res.on('close', () => {
    active--;
    if (waiting) waiters.delete(waiting);
  });
  if (delayed && hold) {
    held++;
    waiting = schedule;
    waiters.add(waiting);
  } else schedule();
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const apis = process.env.BENCH_APIS.split(',');
  let next = 0;
  http
    .createServer((req, res) => {
      const query = new URL(req.url, 'http://proxy').searchParams.get('query') ?? '';
      const insert = /^\s*INSERT\s/i.test(query);
      if (insert) inserts++;
      forward(req, res, 'http://clickhouse:8123', insert);
    })
    .listen(8123, '0.0.0.0');
  http
    .createServer(async (req, res) => {
      if (req.url === '/indexer-metrics') {
        try {
          const metrics = await Promise.all(
            apis.map(async (api) => {
              const r = await fetch(`http://${api}:8090/metrics`, { headers: { authorization: 'Bearer disposable-benchmark-token' }, signal: AbortSignal.timeout(5000) });
              if (!r.ok) throw new Error(`Indexer metrics: ${r.status}`);
              return [api, await r.text()];
            }),
          );
          res.setHeader('content-type', 'application/json');
          res.end(JSON.stringify(Object.fromEntries(metrics)));
        } catch (e) {
          res.writeHead(503).end(String(e));
        }
        return;
      }
      if (req.url === '/query' && req.method === 'POST') {
        try {
          let body = '';
          for await (const part of req) body += part;
          const url = new URL('http://clickhouse:8123/');
          for (const [key, value] of Object.entries(JSON.parse(body))) url.searchParams.set(key, value);
          const r = await fetch(url, { signal: AbortSignal.timeout(30000) });
          res.writeHead(r.status, { 'content-type': 'text/plain' }).end(await r.text());
        } catch (e) {
          res.writeHead(503).end(String(e));
        }
        return;
      }
      if (req.url === '/ready') {
        const targets = [...apis.map((api) => `http://${api}:8080/readyz`), 'http://intake:4318/healthz'];
        const ready = await Promise.all(
          targets.map(async (url) => {
            try {
              const r = await fetch(url, { signal: AbortSignal.timeout(2000) });
              await r.text();
              return r.ok;
            } catch {
              return false;
            }
          }),
        );
        res.writeHead(ready.every(Boolean) ? 200 : 503).end(JSON.stringify(ready));
        return;
      }
      if (req.url === '/ingest/logs') {
        forward(req, res, `http://${apis[next++ % apis.length]}:8090`);
        return;
      }
      if (req.url === '/control' && req.method === 'POST') {
        let body = '';
        for await (const part of req) body += part;
        const config = JSON.parse(body);
        if ('delayMs' in config) delayMs = config.delayMs;
        if ('hold' in config) hold = config.hold;
        if (!hold) {
          for (const callback of waiters) callback();
          waiters.clear();
        }
        if (config.reset) {
          peakActive = active;
          held = inserts = failures = 0;
        }
      }
      res.setHeader('content-type', 'application/json');
      res.end(JSON.stringify(stats()));
    })
    .listen(8080, '0.0.0.0');
}
