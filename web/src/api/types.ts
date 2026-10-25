// Wire contract between the web app and the ObserveCraft query API.
// Keep these shapes stable: the backend (Vector → storage → query service) will implement them.

export type LogStatus = 'error' | 'warn' | 'info' | 'debug';

export type JsonValue = string | number | boolean | null | JsonValue[] | { [key: string]: JsonValue };

export interface LogEvent {
  id: string;
  timestamp: number; // epoch ms
  status: LogStatus;
  service: string;
  host: string;
  source: string;
  message: string;
  /** Indexed infrastructure tags (env, kube_namespace, pod_name, …) — queried as `key:value`. */
  tags: Record<string, string>;
  /** Structured event attributes parsed from the log — queried as `@path.to.attr:value`. */
  attributes: Record<string, JsonValue>;
  /** Trace the log was written in (links to APM), queried as `trace_id:…`. */
  traceId?: string | null;
  spanId?: string | null;
}

export interface TimeRange {
  from: number; // epoch ms
  to: number;
}

export interface LogQuery {
  range: TimeRange;
  /** Search syntax, e.g. `service:(a OR b) env:prod -status:info "bad response" @http.status_code:5*` */
  query: string;
}

export interface FacetValue {
  value: string;
  count: number;
}

export interface Facet {
  key: string; // query key, e.g. "service", "kube_namespace"
  label: string; // display name, e.g. "Service"
  group: string; // e.g. "CORE", "KUBERNETES"
  values: FacetValue[];
}

export interface HistogramBucket {
  t: number;
  counts: Record<LogStatus, number>;
}

export interface LogInsight {
  kind: 'error_outlier';
  attribute: string; // query key including "@" for attributes
  value: string;
  shareOfErrors: number; // 0..1
  shareOfLogs: number;
}

export interface LogSearchResult {
  logs: LogEvent[];
  total: number;
  facets: Facet[];
  histogram: HistogramBucket[];
  insights: LogInsight[];
}

export interface QuerySuggestion {
  /** Text inserted into the query bar (e.g. "service:" or "service:order-service"). */
  text: string;
  count: number;
}

/** One push from the live-tail stream (SSE/WebSocket in the real backend). */
export interface LiveTailBatch {
  events: LogEvent[]; // events to display (already sampled), newest last
  matchedPerSec: number; // rate of events matching the query
  displayedRatio: number; // displayed / matched, 0..1
}

export interface MetricSeries {
  name: string;
  points: [number, number][];
}
