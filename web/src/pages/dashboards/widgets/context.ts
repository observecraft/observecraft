import type { DashboardExecution } from '../execution';
import { createContext, useContext } from 'react';
import type { Widget } from '../../../api/dashboards/types';
import type { TimeRange } from '../../../api/types';

/** Everything a widget needs from its dashboard: the time range, query resolution and edit callbacks. */
export interface DashboardContextValue {
  range: TimeRange;
  metrics?: DashboardExecution;
  /** Resolves template variables in a metric query. */
  resolve: (query: string) => string;
  /** Applies the dashboard's `$var` values to a log/monitor search query. */
  resolveSearch: (query: string) => string;
  canEdit: boolean;
  onEdit: (w: Widget) => void;
  onClone: (id: string) => void;
  onDelete: (id: string) => void;
  onFullscreen: (w: Widget) => void;
  /** Child layout changes inside a group. */
  onGroupLayout: (groupId: string, layouts: { i: string; x: number; y: number; w: number; h: number }[]) => void;
}

export const DashboardContext = createContext<DashboardContextValue | null>(null);

export function useDashboardContext(): DashboardContextValue {
  const v = useContext(DashboardContext);
  if (!v) throw new Error('Widgets must render inside a DashboardContext');
  return v;
}

/** 3000 → "3k", 1500000 → "1.5M"; small values keep `precision` decimals. */
export function formatValue(v: number, precision = 0): string {
  const a = Math.abs(v);
  const trim = (s: string) => s.replace(/\.0+$/, '').replace(/(\.\d*[1-9])0+$/, '$1');
  if (a >= 1e9) return `${trim((v / 1e9).toFixed(1))}B`;
  if (a >= 1e6) return `${trim((v / 1e6).toFixed(1))}M`;
  if (a >= 1e4) return `${trim((v / 1e3).toFixed(a >= 1e5 ? 0 : 1))}k`;
  if (a >= 1e3) return `${trim((v / 1e3).toFixed(1))}k`;
  return trim(v.toFixed(precision));
}
