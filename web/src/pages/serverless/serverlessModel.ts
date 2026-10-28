// Pure helpers of the Serverless page: the "Filter groups with" toolbar, grouping the function table by
// service, the Issues & Insights cards and number formatting.
import type { LambdaFunction, LambdaInsightKind } from '../../api/serverless/client';

/** `any`, or only functions that have (`with`) / lack (`without`) the telemetry. */
export type Presence = 'any' | 'with' | 'without';

export interface SourceFilters {
  enhanced: Presence;
  traces: Presence;
  logs: Presence;
}

export const NO_SOURCE_FILTERS: SourceFilters = { enhanced: 'any', traces: 'any', logs: 'any' };

function keep(p: Presence, has: boolean): boolean {
  return p === 'any' || (p === 'with') === has;
}

export function applySourceFilters(functions: LambdaFunction[], f: SourceFilters): LambdaFunction[] {
  return functions.filter((fn) => keep(f.enhanced, fn.sources.includes('enhanced_metrics')) && keep(f.traces, fn.sources.includes('traces')) && keep(f.logs, fn.hasLogs));
}

export interface ServiceGroup {
  service: string;
  functions: LambdaFunction[];
  invocations: number;
  errors: number;
  costUsd: number;
}

export const NO_SERVICE = '(no service)';

/** Functions grouped by service: busiest group first, functions by invocations inside. */
export function groupByService(functions: LambdaFunction[]): ServiceGroup[] {
  const groups = new Map<string, LambdaFunction[]>();
  for (const f of functions) {
    const key = f.service || NO_SERVICE;
    groups.set(key, [...(groups.get(key) ?? []), f]);
  }
  const sum = (fs: LambdaFunction[], pick: (f: LambdaFunction) => number | null | undefined) => fs.reduce((a, f) => a + (pick(f) ?? 0), 0);
  return [...groups]
    .map(([service, fs]) => ({
      service,
      functions: [...fs].sort((a, b) => (b.invocations ?? 0) - (a.invocations ?? 0) || a.name.localeCompare(b.name)),
      invocations: sum(fs, (f) => f.invocations),
      errors: sum(fs, (f) => f.errors),
      costUsd: sum(fs, (f) => f.costUsd),
    }))
    .sort((a, b) => b.invocations - a.invocations || a.service.localeCompare(b.service));
}

export const INSIGHT_INFO: Record<LambdaInsightKind, { title: string; metric: string; severity: 'error' | 'warn' | 'info'; unit: 'pct' | 'count' }> = {
  high_errors: { title: 'High errors', metric: 'Error rate', severity: 'error', unit: 'pct' },
  out_of_memory: { title: 'Out of memory', metric: 'Out-of-memory errors', severity: 'error', unit: 'count' },
  timeouts: { title: 'Timeouts', metric: 'Timed-out invocations', severity: 'error', unit: 'count' },
  throttles: { title: 'Throttles', metric: 'Throttled invocations', severity: 'warn', unit: 'count' },
  cold_starts: { title: 'High cold starts', metric: 'Cold start rate', severity: 'warn', unit: 'pct' },
  high_memory: { title: 'High memory usage', metric: 'Peak memory of allocation', severity: 'warn', unit: 'pct' },
  over_provisioned: { title: 'Over-provisioned memory', metric: 'Peak memory of allocation', severity: 'info', unit: 'pct' },
};

const SEVERITY_ORDER = { error: 0, warn: 1, info: 2 };

export interface InsightCard {
  kind: LambdaInsightKind;
  functionId: string;
  functionName: string;
  value: number;
}

/** One card per function insight, most severe first, then the largest value. */
export function insightCards(functions: LambdaFunction[]): InsightCard[] {
  return functions
    .flatMap((f) => f.insights.map((i) => ({ kind: i.kind, functionId: f.id, functionName: f.name, value: i.value })))
    .sort((a, b) => SEVERITY_ORDER[INSIGHT_INFO[a.kind].severity] - SEVERITY_ORDER[INSIGHT_INFO[b.kind].severity] || b.value - a.value || a.functionName.localeCompare(b.functionName));
}

export function formatInsightValue(kind: LambdaInsightKind, value: number): string {
  return INSIGHT_INFO[kind].unit === 'pct' ? `${value.toFixed(1)} %` : value.toLocaleString('en-US');
}

export function formatCount(n: number | null | undefined): string {
  if (n == null) return '—';
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(n >= 10_000_000 ? 0 : 1)}M`;
  if (n >= 10_000) return `${(n / 1000).toFixed(n >= 100_000 ? 0 : 1)}k`;
  return Math.round(n).toLocaleString('en-US');
}

export function formatMs(ms: number | null | undefined): string {
  if (ms == null) return '—';
  if (ms >= 60_000) return `${(ms / 60_000).toFixed(1)} min`;
  if (ms >= 1000) return `${(ms / 1000).toFixed(ms >= 10_000 ? 1 : 2)} s`;
  return `${ms.toFixed(ms >= 100 ? 0 : 1)} ms`;
}

export function formatCost(usd: number | null | undefined): string {
  if (usd == null) return '—';
  if (usd === 0) return '$0';
  if (usd < 0.01) return '< $0.01';
  return `$${usd.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
}

export function formatMemory(mb: number | null | undefined): string {
  if (mb == null) return '—';
  return mb >= 1024 ? `${(mb / 1024).toFixed(mb % 1024 === 0 ? 0 : 1)} GB` : `${Math.round(mb)} MB`;
}
