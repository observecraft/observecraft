// TanStack Query bindings for LLM Observability. Ranges are snapped to the minute so live time
// windows don't change the query key every 10 seconds.
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { getLlmTrace, llmEvaluations, llmFacets, llmOverview, searchLlmTraces, type LlmQuery } from '../../api/llm/client';
import { snapRange } from '../../lib/time';

const opts = { placeholderData: keepPreviousData, retry: 1 } as const;
const snapped = (q: LlmQuery): LlmQuery => ({ ...q, range: snapRange(q.range) });
const key = (name: string, ...rest: unknown[]) => ['llm', name, ...rest] as const;

export function useLlmOverview(q: LlmQuery, groupBy: 'model' | 'ml_app') {
  const s = snapped(q);
  return useQuery({ queryKey: key('overview', s, groupBy), queryFn: ({ signal }) => llmOverview(s, groupBy, signal), ...opts });
}

export function useLlmFacets(q: LlmQuery, dimensions: string[]) {
  const s = snapped(q);
  return useQuery({ queryKey: key('facets', s, dimensions), queryFn: ({ signal }) => llmFacets(s, dimensions, signal), ...opts });
}

export function useLlmTraces(q: LlmQuery, offset: number, limit: number) {
  const s = snapped(q);
  return useQuery({ queryKey: key('traces', s, offset, limit), queryFn: ({ signal }) => searchLlmTraces(s, offset, limit, signal), ...opts });
}

export function useLlmTrace(traceId: string | null) {
  return useQuery({ queryKey: key('trace', traceId), queryFn: ({ signal }) => getLlmTrace(traceId!, signal), enabled: !!traceId, staleTime: Infinity, retry: false });
}

export function useLlmEvaluations(q: LlmQuery, check: string | null, outcome: 'fail' | 'pass' | 'all') {
  const s = snapped(q);
  return useQuery({ queryKey: key('evaluations', s, check, outcome), queryFn: ({ signal }) => llmEvaluations(s, check, outcome, signal), ...opts });
}

const WEEK = 7 * 24 * 3_600_000;

/** Whether any LLM span arrived in the last 7 days (the onboarding shows otherwise). */
export function useHasLlmData() {
  const to = Math.floor(Date.now() / 600_000) * 600_000 + 600_000;
  return useQuery({
    queryKey: key('has-data', to),
    queryFn: async ({ signal }) => (await searchLlmTraces({ range: { from: to - WEEK, to }, query: '', filters: {} }, 0, 1, signal)).total > 0,
    staleTime: 60_000,
    retry: 1,
  });
}
