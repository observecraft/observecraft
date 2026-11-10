// The topology_map widget of integrations' dashboards on Data Streams (`data_source: data_streams`, as the Kafka
// dashboards draw their topics): the Data Streams map narrowed to the query's messaging system and queue
// (`traffic_type:kafka queue:orders`) and its filters (`env:prod`), with the services producing to and consuming from it.
import { streamMap, type StreamEdge, type StreamNode } from '../dataStreams/client';
import type { TimeRange } from '../types';

export interface TopologyRequest {
  request_type?: string;
  query?: { data_source?: string; query_string?: string; filters?: string[]; service?: string };
}

export interface TopologyQuery {
  /** The messaging system (`traffic_type:`), any when undefined. */
  system?: string;
  /** The queue or topic (`queue:`), any when undefined. */
  queue?: string;
  env?: string;
  service?: string;
}

/** `key:value` terms of a search, without the `*` wildcards template variables leave. */
function terms(text: string): Map<string, string> {
  const out = new Map<string, string>();
  for (const term of text.split(/\s+/)) {
    const i = term.indexOf(':');
    const value = term.slice(i + 1).replace(/^"(.*)"$/, '$1');
    if (i > 0 && value && value !== '*') out.set(term.slice(0, i), value);
  }
  return out;
}

/** What a topology request shows, its template variables resolved by `resolve` (a search: `$env` → `env:prod`, or ''). */
export function topologyQuery(request: TopologyRequest | undefined, resolve: (query: string) => string): TopologyQuery {
  const q = request?.query ?? {};
  if (q.data_source !== 'data_streams') throw new Error(`Topology data source ${q.data_source ?? 'none'} is not supported yet`);
  const query = terms(resolve(q.query_string ?? ''));
  const filters = terms((q.filters ?? []).map(resolve).join(' '));
  return { system: query.get('traffic_type'), queue: query.get('queue'), env: filters.get('env'), service: q.service || filters.get('service') || undefined };
}

/** The map's topics matching the query and the edges and services around them. */
export function narrowTopology(map: { nodes: StreamNode[]; edges: StreamEdge[] }, q: TopologyQuery): { nodes: StreamNode[]; edges: StreamEdge[] } {
  const topics = new Set(map.nodes.filter((n) => n.kind === 'topic' && (!q.system || n.system === q.system) && (!q.queue || n.name === q.queue)).map((n) => n.id));
  const edges = map.edges.filter((e) => (topics.has(e.source) || topics.has(e.target)) && (!q.service || [e.source, e.target].includes(`service:${q.service}`)));
  const ids = new Set(edges.flatMap((e) => [e.source, e.target]));
  return { nodes: map.nodes.filter((n) => ids.has(n.id) || (topics.has(n.id) && !q.service)), edges };
}

/** A topology_map widget's graph over `range`. */
export async function queryTopology(request: TopologyRequest | undefined, resolve: (query: string) => string, range: TimeRange) {
  const q = topologyQuery(request, resolve);
  return narrowTopology(await streamMap({ range, env: q.env ?? null }), q);
}
