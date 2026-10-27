import { readEventStream } from './eventStream';
// Typed HTTP client for the ObserveCraft API. Paths, request bodies and responses are checked
// against src/api/generated/schema.d.ts, which is generated from the Rust server (`make openapi`).
import type { paths } from './generated/schema';

/** "live" talks to the API; anything else uses the in-browser mocks. */
export const API_MODE: 'live' | 'mock' = import.meta.env.VITE_API_MODE === 'live' ? 'live' : 'mock';

type PostPath = { [P in keyof paths]: paths[P] extends { post: object } ? P : never }[keyof paths];
type PostOp<P extends PostPath> = paths[P] extends { post: infer O } ? O : never;
type Body<P extends PostPath> = PostOp<P> extends { requestBody: { content: { 'application/json': infer B } } } ? B : never;
/** The 200 JSON body, or void for endpoints that answer 204 No Content. */
type Ok<P extends PostPath> = PostOp<P> extends { responses: { 200: { content: { 'application/json': infer R } } } } ? R : void;

/** Fired on window when an API call answers 401. */
export const UNAUTHORIZED_EVENT = 'oc:unauthorized';

export class ApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
  }
}

type GetPath = { [P in keyof paths]: paths[P] extends { get: object } ? P : never }[keyof paths];
type GetOp<P extends GetPath> = paths[P] extends { get: infer O } ? O : never;
type GetOk<P extends GetPath> = GetOp<P> extends { responses: { 200: { content: { 'application/json': infer R } } } } ? R : never;
type PatchPath = { [P in keyof paths]: paths[P] extends { patch: object } ? P : never }[keyof paths];
type PatchOp<P extends PatchPath> = paths[P] extends { patch: infer O } ? O : never;
type PatchBody<P extends PatchPath> = PatchOp<P> extends { requestBody: { content: { 'application/json': infer B } } } ? B : never;
type PutPath = { [P in keyof paths]: paths[P] extends { put: object } ? P : never }[keyof paths];
type PutOp<P extends PutPath> = paths[P] extends { put: infer O } ? O : never;
type PutBody<P extends PutPath> = PutOp<P> extends { requestBody: { content: { 'application/json': infer B } } } ? B : never;
type PutOk<P extends PutPath> = PutOp<P> extends { responses: { 200: { content: { 'application/json': infer R } } } } ? R : void;
type DeletePath = { [P in keyof paths]: paths[P] extends { delete: object } ? P : never }[keyof paths];
type PatchOk<P extends PatchPath> = PatchOp<P> extends { responses: { 200: { content: { 'application/json': infer R } } } } ? R : void;
/** `{name}` segments of a templated path, e.g. "/traces/{trace_id}" → "trace_id". */
type PathParams<P extends string> = P extends `${string}{${infer K}}${infer Rest}` ? K | PathParams<Rest> : never;

async function checkedFetch(url: string, init: RequestInit): Promise<Response> {
  const res = await fetch(url, { ...init, credentials: 'same-origin' });
  if (!res.ok) {
    let message = `Request failed (${res.status})`;
    try {
      const err = (await res.json()) as { error?: string };
      if (err.error) message = err.error;
    } catch {
      /* non-JSON error body */
    }
    // An expired or revoked session anywhere sends the app back to sign-in (auth endpoints report their own errors).
    if (res.status === 401 && !url.startsWith('/api/v1/auth/')) window.dispatchEvent(new Event(UNAUTHORIZED_EVENT));
    throw new ApiError(message, res.status);
  }
  return res;
}

async function request<T>(url: string, init: RequestInit): Promise<T> {
  const res = await checkedFetch(url, init);
  if (res.status === 204) return undefined as T;
  return (await res.json()) as T;
}

export function post<P extends PostPath>(path: P, body: Body<P>, signal?: AbortSignal): Promise<Ok<P>> {
  return request(path, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body), signal });
}

function fill(path: string, params: Record<string, string>): string {
  return path.replace(/\{(\w+)\}/g, (_, k: string) => encodeURIComponent(params[k]));
}

/** GET with path parameters substituted (and URL-encoded) into the templated path, plus optional query. */
export function get<P extends GetPath>(path: P, params: Record<PathParams<P>, string>, signal?: AbortSignal, query?: Record<string, string | number>): Promise<GetOk<P>> {
  const qs = query ? `?${new URLSearchParams(Object.entries(query).map(([k, v]) => [k, String(v)]))}` : '';
  return request(fill(path, params) + qs, { method: 'GET', signal });
}

/** POST to a templated path (`/dashboards/{id}/clone`). */
export function postTo<P extends PostPath>(path: P, params: Record<PathParams<P>, string>, body: Body<P> | undefined, signal?: AbortSignal): Promise<Ok<P>> {
  const init: RequestInit = body === undefined ? { method: 'POST', signal } : { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body), signal };
  return request(fill(path, params), init);
}

export function patch<P extends PatchPath>(path: P, params: Record<PathParams<P>, string>, body: PatchBody<P>, signal?: AbortSignal): Promise<PatchOk<P>> {
  return request(fill(path, params), { method: 'PATCH', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body), signal });
}

export function put<P extends PutPath>(path: P, params: Record<PathParams<P>, string>, body: PutBody<P>, signal?: AbortSignal): Promise<PutOk<P>> {
  return request(fill(path, params), { method: 'PUT', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body), signal });
}

export function del<P extends DeletePath>(path: P, params: Record<PathParams<P>, string>, signal?: AbortSignal): Promise<void> {
  return request(fill(path, params), { method: 'DELETE', signal });
}

/** POST SSE keeps the query plan out of URLs and uses the same auth/error handling as JSON calls. */
type StreamOk<P extends PostPath> = PostOp<P> extends { responses: { 200: { content: { 'text/event-stream': infer R } } } } ? R : never;
export async function postStream<P extends PostPath>(path: P, body: Body<P>, onEvent: (event: string, data: StreamOk<P>) => void, signal: AbortSignal): Promise<void> {
  const res = await checkedFetch(path, { method: 'POST', headers: { 'content-type': 'application/json', accept: 'text/event-stream' }, body: JSON.stringify(body), signal });
  if (!res.body || !res.headers.get('content-type')?.includes('text/event-stream')) throw new Error('Expected a dashboard event stream');
  await readEventStream(res.body, (event, data) => onEvent(event, data as StreamOk<P>));
}
