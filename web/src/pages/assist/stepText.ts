// What a run's step did, in words: the kind of data it looked at, a short title, the query or object, and
// the time window. Used by the run timeline (RunView).

export type StepKind = 'logs' | 'metrics' | 'apm' | 'monitors' | 'incidents' | 'errors' | 'kubernetes' | 'security' | 'code' | 'conclude' | 'other';

export interface StepText {
  kind: StepKind;
  title: string;
  /** The query, object or question the step was about. */
  what?: string;
  /** The time window, e.g. "last 1h". */
  when?: string;
}

type Input = Record<string, unknown>;

const str = (v: unknown): string | undefined => (typeof v === 'string' && v.trim() ? v.trim() : typeof v === 'number' ? String(v) : undefined);

function window(input: Input, fallback?: string): string | undefined {
  const from = str(input.from) ?? fallback;
  if (!from) return undefined;
  const to = str(input.to);
  if (/^\d+[mhdw]$/.test(from)) return to && to !== 'now' ? `${from} to ${to} ago` : `last ${from}`;
  return to ? `${from} – ${to}` : `since ${from}`;
}

const KIND_BY_TOOL: Record<string, StepKind> = {
  search_logs: 'logs',
  query_metrics: 'metrics',
  list_services: 'apm',
  search_traces: 'apm',
  search_monitors: 'monitors',
  get_monitor: 'monitors',
  create_monitor: 'monitors',
  search_incidents: 'incidents',
  get_incident: 'incidents',
  declare_incident: 'incidents',
  search_error_issues: 'errors',
  search_kubernetes: 'kubernetes',
  search_security_signals: 'security',
  get_security_signal: 'security',
  list_repo_files: 'code',
  search_repo_code: 'code',
  read_repo_file: 'code',
  propose_change: 'code',
  conclude: 'conclude',
};

export function describeStep(name: string, input: Input = {}): StepText {
  const kind = KIND_BY_TOOL[name] ?? 'other';
  const q = str(input.query);
  switch (name) {
    case 'search_logs':
      return { kind, title: 'Searched logs', what: q ?? 'all logs', when: window(input, '1h') };
    case 'query_metrics': {
      const queries = Array.isArray(input.queries) ? input.queries.map(String) : [];
      return { kind, title: queries.length === 1 ? 'Queried a metric' : `Queried ${queries.length} metrics`, what: queries.join(', '), when: window(input, '1h') };
    }
    case 'list_services':
      return { kind, title: 'Checked service health', what: q ?? str(input.env) ?? 'all services', when: window(input, '1h') };
    case 'search_traces':
      return { kind, title: 'Searched traces', what: q ?? 'all traces', when: window(input, '1h') };
    case 'search_monitors':
      return { kind, title: 'Searched monitors', what: q ?? (Array.isArray(input.statuses) && input.statuses.length ? `status ${input.statuses.join(', ')}` : 'all monitors') };
    case 'get_monitor':
      return { kind, title: 'Read a monitor', what: `#${str(input.id) ?? '?'}` };
    case 'create_monitor':
      return { kind, title: 'Created a monitor', what: str(input.name) };
    case 'search_incidents':
      return { kind, title: 'Searched incidents', what: q ?? 'all incidents', when: window(input, '30d') };
    case 'get_incident':
      return { kind, title: 'Read an incident', what: `IR-${str(input.id) ?? '?'}` };
    case 'declare_incident':
      return { kind, title: 'Declared an incident', what: str(input.title) };
    case 'search_error_issues':
      return { kind, title: 'Searched Error Tracking', what: q ?? 'all issues', when: window(input, '1d') };
    case 'search_kubernetes':
      return { kind, title: `Looked at Kubernetes ${str(input.kind) ?? 'resources'}`, what: q };
    case 'search_security_signals':
      return { kind, title: 'Searched security signals', what: q ?? 'all signals', when: window(input, '1d') };
    case 'get_security_signal':
      return { kind, title: 'Read a security signal', what: str(input.id) };
    case 'list_repo_files':
      return { kind, title: 'Listed files', what: [str(input.repo), str(input.path)].filter(Boolean).join(' / ') };
    case 'search_repo_code':
      return { kind, title: 'Searched the code', what: `${q ?? ''} in ${str(input.repo) ?? 'the repository'}`.trim() };
    case 'read_repo_file':
      return { kind, title: 'Read a file', what: str(input.path) };
    case 'propose_change':
      return { kind, title: 'Proposed a change', what: str(input.title) };
    case 'conclude':
      return { kind, title: 'Concluded' };
    default:
      return { kind, title: `Ran ${name.replace(/_/g, ' ')}` };
  }
}

/** Tool steps per kind, most first. */
export function stepCounts(steps: { type: string; name?: string }[]): { kind: StepKind; count: number }[] {
  const counts = new Map<StepKind, number>();
  for (const s of steps) {
    if (s.type !== 'tool' || !s.name) continue;
    const k = describeStep(s.name).kind;
    if (k === 'conclude') continue;
    counts.set(k, (counts.get(k) ?? 0) + 1);
  }
  return [...counts.entries()].map(([kind, count]) => ({ kind, count })).sort((a, b) => b.count - a.count);
}
