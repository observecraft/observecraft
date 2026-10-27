import { useEffect, useMemo, useRef, useState } from 'react';
import { queryDashboard, streamDashboard } from '../../api/dashboards/client';
import { collectDashboardQueries, mergeDashboardFrame, metricQueryKey, type DashboardQueryFrame } from '../../api/dashboards/execution';
import type { PanelDataSource } from '../../api/integrations/panelQueries';
import type { TimeRange } from '../../api/types';
import type { Widget } from '../../api/dashboards/types';
import { ApiError } from '../../api/http';

export interface DashboardExecution {
  ready: boolean;
  error: Error | null;
  connectionError: Error | null;
  version: number;
  source: PanelDataSource;
  has: (query: string, points: number) => boolean;
}

/** One dashboard subscription. Clock ticks update display bounds without reconnecting the stream. */
export function useDashboardExecution(widgets: Widget[], resolve: (q: string) => string, range: TimeRange, live: boolean, refreshVersion = 0): DashboardExecution {
  const duration = range.to - range.from;
  const queries = useMemo(() => collectDashboardQueries(widgets, resolve, { from: 0, to: duration }), [widgets, resolve, duration]);
  const key = JSON.stringify([queries, live, duration, live ? null : range, refreshVersion]);
  const currentRange = useRef(range);
  currentRange.current = range;
  const [state, setState] = useState<{ key: string; frame?: DashboardQueryFrame; error: Error | null; version: number }>({ key: '', error: null, version: 0 });
  const connection = useRef<{ key: string; abort: AbortController; dispose?: ReturnType<typeof setTimeout> }>();
  useEffect(() => {
    if (connection.current?.key === key && !connection.current.abort.signal.aborted) {
      clearTimeout(connection.current.dispose);
    } else {
      clearTimeout(connection.current?.dispose);
      connection.current?.abort.abort();
      const abort = new AbortController();
      connection.current = { key, abort };
      const onFrame = (frame: DashboardQueryFrame, snapshot: boolean) => {
        if (abort.signal.aborted) return;
        if (frame.error) {
          setState((old) => ({ ...old, key, frame: old.key === key ? old.frame : undefined, error: new Error(frame.error!) }));
          return;
        }
        setState((old) => ({
          key,
          frame: mergeDashboardFrame(old.key === key ? old.frame : undefined, frame, snapshot),
          error: frame.error ? new Error(frame.error) : null,
          version: old.version + 1,
        }));
      };
      if (queries.length)
        void (async () => {
          let failures = 0;
          while (!abort.signal.aborted) {
            try {
              if (!live) {
                onFrame(await queryDashboard(queries, currentRange.current, abort.signal), true);
                return;
              }
              await streamDashboard(queries, currentRange.current, onFrame, abort.signal);
              if (!abort.signal.aborted) throw new Error('Live updates disconnected');
            } catch (error) {
              if (abort.signal.aborted) return;
              setState((old) => ({ ...old, key, frame: old.key === key ? old.frame : undefined, error: error instanceof Error ? error : new Error('Dashboard query failed') }));
              if (!live || (error instanceof ApiError && error.status >= 400 && error.status < 500 && error.status !== 429)) return;
            }
            const delay = Math.min(30000, 1000 * 2 ** failures++);
            await new Promise<void>((done) => {
              const finish = () => {
                clearTimeout(timer);
                abort.signal.removeEventListener('abort', finish);
                done();
              };
              const timer = setTimeout(finish, delay);
              abort.signal.addEventListener('abort', finish, { once: true });
            });
          }
        })();
    }
    // React StrictMode replays effects; reuse the subscription instead of opening a second HTTP request.
    return () => {
      const old = connection.current;
      if (old?.key === key) old.dispose = setTimeout(() => old.abort.abort(), 0);
    };
    // key contains the complete plan/selection; query objects and live clock ticks must not reconnect it.
  }, [key]);

  const frame = state.key === key ? state.frame : undefined;
  const error = state.key === key ? state.error : null;
  return useMemo(() => {
    const byId = new Map(frame?.results.map((r) => [r.id, r]));
    const byQuery = new Map(queries.map((q) => [metricQueryKey(q.query, q.points), byId.get(q.id)]));
    const source: PanelDataSource = {
      catalog: async () =>
        queries.map((q) => ({ name: /:([^{}]+)\{/.exec(q.query)?.[1] ?? '', kind: byId.get(q.id)?.kind ?? 'unknown', unit: byId.get(q.id)?.unit ?? undefined, tags: [], description: '' })),
      timeseries: async (request) => {
        if (error && !frame) throw error;
        return request.queries.flatMap((query) => {
          const result = byQuery.get(metricQueryKey(query, request.points ?? 120));
          if (!result) throw new Error('Metric query is absent from the dashboard execution plan');
          if (result.error) throw new Error(result.error);
          return result.series;
        });
      },
    };
    return {
      ready: !queries.length || !!frame,
      error: frame ? null : error,
      connectionError: frame ? error : null,
      version: state.version,
      source,
      has: (q: string, points: number) => byQuery.has(metricQueryKey(q, points)),
    };
  }, [frame, error, queries, state.version]);
}
