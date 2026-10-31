// A controllable native ClickHouse endpoint, used only by kafka-durability.mjs.
import http from 'node:http';
import net from 'node:net';

let enabled = false;
let blocked = 0;
let indexed = 0;
let indexerBlocked = 0;
function proxy(port) {
  net
    .createServer((client) => {
      if (!enabled) {
        blocked++;
        client.destroy();
        return;
      }
      const upstream = net.connect(port, 'clickhouse');
      client.pipe(upstream).pipe(client);
      client.on('error', () => upstream.destroy());
      upstream.on('error', () => client.destroy());
      client.on('close', () => upstream.destroy());
      upstream.on('close', () => client.destroy());
    })
    .listen(port, '0.0.0.0');
}
proxy(9000);
proxy(8123);
http
  .createServer((req, res) => {
    if (req.url === '/ingest/logs') {
      if (!enabled) {
        indexerBlocked++;
        res.writeHead(503).end();
        return;
      }
      let body = '';
      req.on('data', (chunk) => {
        body += chunk;
      });
      req.on('end', () => {
        indexed += body.trim().split('\n').filter(Boolean).length;
        res.end();
      });
      return;
    }
    if (req.method === 'POST' && req.url === '/enable') enabled = true;
    res.setHeader('content-type', 'application/json');
    res.end(JSON.stringify({ enabled, blocked, indexed, indexerBlocked }));
  })
  .listen(8080, '0.0.0.0');
