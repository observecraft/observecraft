import { Activity, Gauge, Timer } from 'lucide-react';
import type { CorrectionCategory, SloItem, SloState, SloStatus, SloType, SloWindow } from '../../api/slos/client';

export const STATE_LABEL: Record<SloState, string> = { breached: 'Breached', warning: 'Warning', ok: 'OK', no_data: 'No Data' };
export const STATE_COLOR: Record<SloState, string> = { breached: 'var(--mon-alert)', warning: 'var(--mon-warn)', ok: 'var(--mon-ok)', no_data: 'var(--mon-nodata)' };
export const TYPE_LABEL: Record<SloType, string> = { metric: 'Metric', monitor: 'Monitor', time_slice: 'Time Slice' };
export const WINDOW_LABEL: Record<SloWindow, string> = { '1d': '1 day', '7d': '7 days', '30d': '30 days', '90d': '90 days' };
export const WINDOWS: SloWindow[] = ['1d', '7d', '30d', '90d'];
export const CATEGORY_LABEL: Record<CorrectionCategory, string> = {
  scheduled_maintenance: 'Scheduled maintenance',
  outside_business_hours: 'Outside business hours',
  deployment: 'Deployment',
  other: 'Other',
};
export const BURN_LABEL: Record<string, string> = { high: 'High (≥ 10×)', elevated: 'Elevated (1–10×)', normal: 'Normal (< 1×)', unknown: 'No recent data' };

export function StateBadge({ state }: { state: SloState }) {
  return <span className={`slo-state slo-state-${state}`}>{STATE_LABEL[state]}</span>;
}

export function TypeIcon({ type, size = 15 }: { type: SloType; size?: number }) {
  const Icon = type === 'monitor' ? Gauge : type === 'time_slice' ? Timer : Activity;
  return (
    <span className="slo-type-icon" title={`${TYPE_LABEL[type]}-based SLO`}>
      <Icon size={size} />
    </span>
  );
}

/** "99.95%", with enough decimals to tell targets apart. */
export function fmtPct(v: number | null | undefined, digits = 3): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return '—';
  if (v === 100) return '100%';
  return `${v.toFixed(digits).replace(/0+$/, '').replace(/\.$/, '')}%`;
}

/** "1.2K", "340", "2.5M". */
export function fmtAmount(v: number): string {
  const a = Math.abs(v);
  if (a >= 1e6) return `${(v / 1e6).toFixed(1).replace(/\.0$/, '')}M`;
  if (a >= 1e4) return `${(v / 1e3).toFixed(1).replace(/\.0$/, '')}K`;
  return a >= 100 ? v.toFixed(0) : v.toFixed(1).replace(/\.0$/, '');
}

/** Minutes as "2h 13m" / "45m" / "3d 4h". */
export function fmtMinutes(m: number): string {
  const rounded = Math.round(Math.abs(m));
  const sign = m < 0 ? '-' : '';
  if (rounded < 60) return `${sign}${rounded}m`;
  const h = Math.floor(rounded / 60);
  if (h < 24) return `${sign}${h}h${rounded % 60 ? ` ${rounded % 60}m` : ''}`;
  return `${sign}${Math.floor(h / 24)}d${h % 24 ? ` ${h % 24}h` : ''}`;
}

/** What is left of the budget in its unit: "38 events left", "12m left", "overspent by 1h 5m". */
export function budgetLeft(s: SloStatus): string {
  if (s.budgetAllowed === null || s.budgetAllowed === undefined || s.budgetUsed === null || s.budgetUsed === undefined) return '';
  const left = s.budgetAllowed - s.budgetUsed;
  const amount = s.unit === 'minutes' ? fmtMinutes(Math.abs(left)) : `${fmtAmount(Math.abs(left))} events`;
  return left >= 0 ? `${amount} left` : `overspent by ${amount}`;
}

export function fmtBurn(v: number | null | undefined): string {
  if (v === null || v === undefined) return '—';
  return `${v >= 10 ? v.toFixed(0) : v.toFixed(2).replace(/\.?0+$/, '')}×`;
}

export function fmtDateTime(ms: number): string {
  return new Date(ms).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' });
}

/**
 * Link to the monitor editor prefilled with a burn rate alert for a metric SLO: the error rate in
 * percent over the last hour against 14.4× the allowed error rate (2% of a 30-day budget per hour),
 * warning at 6×. The `slo:<id>` tag lists the monitor on the SLO.
 */
export function burnRateMonitorLink(slo: SloItem): string | null {
  if (slo.type !== 'metric' || !slo.goodQuery || !slo.totalQuery) return null;
  const target = slo.targets.find((t) => t.window === slo.primaryWindow)?.target ?? slo.targets[0]?.target ?? 99.9;
  const allowed = 100 - target;
  const round = (v: number) => String(Number(v.toPrecision(4)));
  const params = new URLSearchParams({
    name: `Burn rate of ${slo.name} is high`,
    query: `(${slo.totalQuery} - ${slo.goodQuery}) / ${slo.totalQuery} * 100`,
    window: '60',
    aggregation: 'avg',
    critical: round(allowed * 14.4),
    warning: round(allowed * 6),
    tags: [`slo:${slo.id}`, ...slo.tags].join(', '),
    message: `The error budget of the SLO "${slo.name}" (target ${target}%) is burning fast.`,
  });
  return `/monitors/create/metric?${params}`;
}
