// Infrastructure lists (list_stream widgets over Kubernetes resources) as Kubernetes Explorer searches.
import type { KubeKind } from '../kubernetes/client';

export interface InfrastructureRequest {
  query: { data_source: string; resource_type?: string; query_string?: string };
  columns?: { field: string }[];
  sort?: { column: string; order?: string };
}

export interface InfrastructureList {
  kind: KubeKind;
  /** Explorer queries, one per alternative of the upstream query (their results are combined). */
  queries: string[];
  /** Explorer column keys; undefined: the kind's default columns. */
  columns?: string[];
  sort?: { column: string; desc: boolean };
}

const KINDS: Record<string, KubeKind> = {
  pod: 'pods',
  container: 'containers',
  deployment: 'deployments',
  replicaSet: 'replicasets',
  daemonSet: 'daemonsets',
  statefulSet: 'statefulsets',
  job: 'jobs',
  cronJob: 'cronjobs',
  node: 'nodes',
  cluster: 'clusters',
  namespace: 'namespaces',
  service: 'services',
};

/** Upstream column fields → Explorer columns (usage, requests and limits share one column). */
const COLUMNS: Record<string, string> = {
  name: 'name',
  status: 'status',
  age: 'age',
  ready: 'ready',
  restarts: 'restarts',
  cpu_usage: 'cpu',
  cpu_usage_requests: 'cpu',
  cpu_usage_limits: 'cpu',
  memory_usage: 'memory',
  memory_usage_requests: 'memory',
  memory_usage_limits: 'memory',
};

/** A query in disjunctive normal form: alternatives of terms that must all match. */
type Dnf = string[][];

/** `a (b OR NOT c)` → [[a, b], [a, -c]]. Filters the Explorer can't evaluate (`field#…`) are left out. */
function alternatives(text: string): Dnf {
  const tokens = text.match(/\(|\)|[^\s()]+/g) ?? [];
  let i = 0;
  const and = (a: Dnf, b: Dnf): Dnf => a.flatMap((x) => b.map((y) => [...x, ...y]));
  function unary(): Dnf {
    const token = tokens[i++];
    if (token === '(') {
      const inner = or();
      i++; // ')'
      return inner;
    }
    if (token === 'NOT') {
      const inner = unary();
      // Only a single term can be negated in an Explorer query.
      return inner.length === 1 && inner[0].length === 1 ? [[`-${inner[0][0]}`]] : [[]];
    }
    // Unresolved variables and match-all filters (`key:*`) don't narrow anything.
    return token.includes('#') || token.startsWith('$') || token.endsWith(':*') ? [[]] : [[token]];
  }
  function conjunction(): Dnf {
    let value = unary();
    while (i < tokens.length && tokens[i] !== ')' && tokens[i] !== 'OR') {
      if (tokens[i] === 'AND') i++;
      value = and(value, unary());
    }
    return value;
  }
  function or(): Dnf {
    const value = conjunction();
    while (tokens[i] === 'OR') {
      i++;
      value.push(...conjunction());
    }
    return value;
  }
  return tokens.length ? or() : [[]];
}

/**
 * The Explorer searches for an upstream infrastructure list. `resolveVar` gives a template
 * variable's filter (`kube_cluster_name:aks-dev`), or '' when it matches everything.
 */
export function infrastructureList(request: InfrastructureRequest, resolveVar: (name: string) => string): InfrastructureList {
  const kind = KINDS[request.query.resource_type ?? ''];
  if (!kind) throw new Error(`Infrastructure list of ${request.query.resource_type ?? 'unknown resources'} is not supported yet`);
  const text = (request.query.query_string ?? '').replace(/\$(\w+)/g, (_, name: string) => resolveVar(name));
  const queries = [...new Set(alternatives(text).map((terms) => terms.join(' ')))];
  const columns = request.columns?.length ? [...new Set(request.columns.map((c) => COLUMNS[c.field]).filter(Boolean))] : undefined;
  const sortColumn = request.sort && COLUMNS[request.sort.column];
  return { kind, queries, columns, sort: sortColumn ? { column: sortColumn, desc: request.sort?.order !== 'asc' } : undefined };
}
