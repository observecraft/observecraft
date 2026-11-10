// The browser's only way out: a forward proxy on loopback that resolves every target itself, refuses internal
// addresses (net.ts) and connects to the address it checked, so a DNS answer can't change between the check and
// the connection. Plain HTTP is forwarded (absolute-form requests); HTTPS and WebSockets are CONNECT tunnels.
import { createServer, request as httpRequest, type IncomingMessage, type ServerResponse } from 'node:http';
import { connect, type AddressInfo, type Socket } from 'node:net';
import { resolve } from './net.ts';

export interface ProxyOptions {
  allowPrivate: boolean;
  /** Called with the URL (or host:port) and the reason of each refused request. */
  onBlocked?: (target: string, reason: string) => void;
}

export interface Proxy {
  host: string;
  port: number;
  close: () => Promise<void>;
}

const HOP_HEADERS = ['proxy-connection', 'proxy-authorization', 'connection', 'keep-alive', 'te', 'trailer', 'upgrade'];

/** Marks the proxy's own refusals, so a navigation to a refused address fails instead of showing the refusal page. */
export const BLOCKED_HEADER = 'x-observecraft-blocked';

function refuse(res: ServerResponse, status: number, message: string) {
  res.writeHead(status, { 'content-type': 'text/plain; charset=utf-8', connection: 'close', ...(status === 403 ? { [BLOCKED_HEADER]: '1' } : {}) });
  res.end(message);
}

export function startProxy(options: ProxyOptions): Promise<Proxy> {
  const blocked = (target: string, reason: string) => options.onBlocked?.(target, reason);

  const forward = async (req: IncomingMessage, res: ServerResponse) => {
    let url: URL;
    try {
      url = new URL(req.url ?? '');
    } catch {
      return refuse(res, 400, 'The proxy forwards absolute http URLs only');
    }
    if (url.protocol !== 'http:') return refuse(res, 400, 'The proxy forwards absolute http URLs only');
    let address: string;
    try {
      address = await resolve(url.hostname, options.allowPrivate);
    } catch (e) {
      blocked(url.href, (e as Error).message);
      return refuse(res, 403, (e as Error).message);
    }
    const headers = { ...req.headers };
    for (const h of HOP_HEADERS) delete headers[h];
    const upstream = httpRequest({ host: address, port: Number(url.port || 80), method: req.method, path: `${url.pathname}${url.search}`, headers, setHost: false }, (answer) => {
      res.writeHead(answer.statusCode ?? 502, answer.headers);
      answer.pipe(res);
    });
    upstream.on('error', (e) => (res.headersSent ? res.destroy() : refuse(res, 502, e.message)));
    req.pipe(upstream);
  };

  const tunnel = async (req: IncomingMessage, client: Socket, head: Buffer) => {
    const target = req.url ?? '';
    const match = /^\[?([^\]]+?)\]?:(\d+)$/.exec(target);
    if (!match) return client.end('HTTP/1.1 400 Bad Request\r\nconnection: close\r\n\r\n');
    const [, host, port] = match;
    let address: string;
    try {
      address = await resolve(host, options.allowPrivate);
    } catch (e) {
      blocked(target, (e as Error).message);
      return client.end(`HTTP/1.1 403 Forbidden\r\ncontent-type: text/plain\r\nconnection: close\r\n\r\n${(e as Error).message}`);
    }
    const upstream = connect(Number(port), address, () => {
      client.write('HTTP/1.1 200 Connection Established\r\n\r\n');
      if (head.length) upstream.write(head);
      upstream.pipe(client);
      client.pipe(upstream);
    });
    upstream.on('error', () => client.end('HTTP/1.1 502 Bad Gateway\r\nconnection: close\r\n\r\n'));
    client.on('error', () => upstream.destroy());
  };

  const server = createServer((req, res) => void forward(req, res));
  server.on('connect', (req, socket, head) => void tunnel(req, socket as Socket, head));
  server.on('clientError', (_e, socket) => socket.destroy());
  return new Promise((done) =>
    server.listen(0, '127.0.0.1', () => {
      const { port } = server.address() as AddressInfo;
      done({
        host: '127.0.0.1',
        port,
        close: () =>
          new Promise<void>((closed) => {
            server.closeAllConnections();
            server.close(() => closed());
          }),
      });
    }),
  );
}
