// oc-browser's HTTP API, for the ObserveCraft server only (Bearer OC_BROWSER_TOKEN on an internal network):
//   POST /v1/runs   run a browser test (RunRequest → RunResult); 503 with Retry-After when every slot is busy
//   POST   /v1/sessions               open an agent session (OpenRequest → ActionResult & {id}); 503 when all are busy
//   POST   /v1/sessions/{id}/actions  one action ({action, variables} → ActionResult); 404 for an unknown session
//   DELETE /v1/sessions/{id}          end it
//   GET  /healthz   liveness, without the token
// The server validates steps (browser/steps.rs); this layer only checks the request's shape.
import { createServer, type IncomingMessage, type Server, type ServerResponse } from 'node:http';
import { timingSafeEqual } from 'node:crypto';
import type { RunRequest, Runner } from './run.ts';
import type { SessionAction, Sessions } from './session.ts';

const MAX_BODY_BYTES = 2 * 1024 * 1024;
const MAX_STEPS = 100;

export interface AppOptions {
  runner: Runner;
  sessions: Sessions;
  token: string;
  /** Runs at once; more are refused with 503. */
  concurrency: number;
}

function send(res: ServerResponse, status: number, body: unknown, headers: Record<string, string> = {}) {
  res.writeHead(status, { 'content-type': 'application/json', ...headers });
  res.end(JSON.stringify(body));
}

function authorized(req: IncomingMessage, token: string): boolean {
  const given = Buffer.from(/^Bearer (.+)$/.exec(req.headers.authorization ?? '')?.[1] ?? '');
  const wanted = Buffer.from(token);
  return given.length === wanted.length && timingSafeEqual(given, wanted);
}

async function readJson(req: IncomingMessage): Promise<unknown> {
  let size = 0;
  const chunks: Buffer[] = [];
  for await (const chunk of req) {
    size += chunk.length;
    if (size > MAX_BODY_BYTES) throw new Error('The request is larger than 2 MiB');
    chunks.push(chunk);
  }
  try {
    return JSON.parse(Buffer.concat(chunks).toString('utf8'));
  } catch {
    throw new Error('The request is not JSON');
  }
}

/** The run request, or why it's malformed. */
export function parseRun(body: unknown): RunRequest | string {
  const b = body as Partial<RunRequest> | null;
  if (!b || typeof b !== 'object') return 'Send a JSON object';
  if (typeof b.startUrl !== 'string' || !/^https?:\/\//i.test(b.startUrl)) return 'startUrl must be an http or https URL';
  if (!Array.isArray(b.steps) || b.steps.length > MAX_STEPS) return `steps must be a list of at most ${MAX_STEPS} steps`;
  if (b.variables !== undefined && (typeof b.variables !== 'object' || Object.values(b.variables).some((v) => typeof v !== 'string'))) return 'variables must map names to strings';
  return b as RunRequest;
}

const ACTIONS = ['snapshot', 'goto', 'click', 'type', 'select', 'press', 'back', 'scroll', 'assert'];

async function sessionRoute(req: IncomingMessage, res: ServerResponse, path: string, sessions: Sessions) {
  const m = /^\/v1\/sessions(?:\/([\w-]+)(\/actions)?)?$/.exec(path);
  if (!m) return send(res, 404, { error: 'Not found' });
  const [, id, actions] = m;
  if (req.method === 'DELETE' && id && !actions) {
    await sessions.end(id);
    res.writeHead(204).end();
    return;
  }
  if (req.method !== 'POST') return send(res, 404, { error: 'Not found' });
  let body: Record<string, unknown>;
  try {
    body = ((await readJson(req)) ?? {}) as Record<string, unknown>;
  } catch (e) {
    return send(res, 400, { error: (e as Error).message });
  }
  if (!id) {
    if (typeof body.startUrl !== 'string' || !/^https?:\/\//i.test(body.startUrl)) return send(res, 400, { error: 'startUrl must be an http or https URL' });
    try {
      return send(res, 200, await sessions.open(body as never));
    } catch (e) {
      const message = (e as Error).message;
      return send(res, /busy/.test(message) ? 503 : 422, { error: message }, /busy/.test(message) ? { 'retry-after': '10' } : {});
    }
  }
  if (!actions) return send(res, 404, { error: 'Not found' });
  const action = body.action as SessionAction | undefined;
  if (!action || !ACTIONS.includes(action.type)) return send(res, 400, { error: `action.type is one of ${ACTIONS.join(', ')}` });
  const variables = (body.variables ?? {}) as Record<string, string>;
  try {
    return send(res, 200, await sessions.act(id, action, variables));
  } catch (e) {
    const message = (e as Error).message;
    return send(res, /session/.test(message) ? 404 : 500, { error: message });
  }
}

export function createApp({ runner, sessions, token, concurrency }: AppOptions): Server {
  let running = 0;
  return createServer(async (req, res) => {
    const path = new URL(req.url ?? '/', 'http://oc-browser').pathname;
    if (req.method === 'GET' && path === '/healthz') return send(res, 200, { ok: true });
    if (!authorized(req, token)) return send(res, 401, { error: 'Missing or wrong token' });
    if (path.startsWith('/v1/sessions')) return sessionRoute(req, res, path, sessions);
    if (req.method !== 'POST' || path !== '/v1/runs') return send(res, 404, { error: 'Not found' });
    let request: RunRequest | string;
    try {
      request = parseRun(await readJson(req));
    } catch (e) {
      return send(res, 400, { error: (e as Error).message });
    }
    if (typeof request === 'string') return send(res, 400, { error: request });
    if (running >= concurrency) return send(res, 503, { error: 'Every browser slot is busy' }, { 'retry-after': '5' });
    running++;
    try {
      send(res, 200, await runner.run(request));
    } catch (e) {
      send(res, 500, { error: `The run failed: ${(e as Error).message}` });
    } finally {
      running--;
    }
  });
}
