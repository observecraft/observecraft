// Shared bits of the Cloud Cost pages: money formatting, calendar ranges, providers and filters.
import { useQuery } from '@tanstack/react-query';
import { costTags, queryCosts, type CostQueryRequest, type CostTag, type CostType } from '../../api/cloudCost/client';

export const DAY = 86_400_000;

/** `$222K`, `$5.49K`, `$447.28`, `-$59.34`: compact above a thousand. */
export function money(v: number, opts: { precise?: boolean } = {}): string {
  const sign = v < 0 ? '-' : '';
  const a = Math.abs(v);
  if (!opts.precise && a >= 1e6) return `${sign}$${(a / 1e6).toFixed(a >= 1e7 ? 1 : 2).replace(/\.0+$/, '')}M`;
  if (!opts.precise && a >= 1e3) return `${sign}$${(a / 1e3).toFixed(a >= 1e5 ? 0 : a >= 1e4 ? 1 : 2).replace(/\.0+$/, '')}K`;
  if (a > 0 && a < 1) return `${sign}${(a * 100).toFixed(2)}¢`;
  return `${sign}$${a.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
}

/** Axis labels: `$15k`, `$500`. */
export function moneyAxis(v: number): string {
  const a = Math.abs(v);
  const sign = v < 0 ? '-' : '';
  if (a >= 1e6) return `${sign}$${+(a / 1e6).toFixed(1)}M`;
  if (a >= 1e3) return `${sign}$${+(a / 1e3).toFixed(1)}k`;
  return `${sign}$${+a.toFixed(a < 1 ? 2 : 0)}`;
}

/** `+$28.1K`, `-$9.93K`. */
export function signedMoney(v: number): string {
  return `${v > 0 ? '+' : ''}${money(v)}`;
}

/** Change from `prev` to `cur` in percent; null when there was nothing before. */
export function pctChange(cur: number, prev: number): number | null {
  if (Math.abs(prev) < 0.005) return null;
  return ((cur - prev) / Math.abs(prev)) * 100;
}

export function pctLabel(p: number | null, digits = 1): string {
  if (p == null) return 'new';
  const s = Math.abs(p) >= 1000 ? Math.round(p).toLocaleString('en-US') : p.toFixed(Math.abs(p) >= 100 ? 0 : digits);
  return `${p > 0 ? '+' : ''}${s}%`;
}

// ---------------------------------------------------------------- calendar

export const startOfDay = (ms: number) => Math.floor(ms / DAY) * DAY;
export const today = () => startOfDay(Date.now());

export function monthStart(ms: number, offset = 0): number {
  const d = new Date(ms);
  return Date.UTC(d.getUTCFullYear(), d.getUTCMonth() + offset, 1);
}

export function daysInMonth(ms: number): number {
  return (monthStart(ms, 1) - monthStart(ms)) / DAY;
}

const MONTHS = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];
export const monthName = (ms: number) => MONTHS[new Date(ms).getUTCMonth()];
export const shortDate = (ms: number) => new Date(ms).toLocaleDateString('en-US', { month: 'short', day: 'numeric', timeZone: 'UTC' });
export const longDate = (ms: number) => new Date(ms).toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric', timeZone: 'UTC' }).toUpperCase();

/** `SEP 30 - OCT 28`. */
export function rangeLabel(from: number, to: number): string {
  const f = new Date(from);
  const t = new Date(to);
  const opts: Intl.DateTimeFormatOptions = { month: 'short', day: 'numeric', timeZone: 'UTC' };
  const y = f.getUTCFullYear() !== t.getUTCFullYear() ? { year: 'numeric' as const } : {};
  return `${f.toLocaleDateString('en-US', { ...opts, ...y })} - ${t.toLocaleDateString('en-US', { ...opts, ...y })}`.toUpperCase();
}

/** Cost ranges: `1mo` = the last 30 days through today. */
export const RANGES: { value: string; label: string; days: number }[] = [
  { value: '1w', label: 'Past 1 Week', days: 7 },
  { value: '2w', label: 'Past 2 Weeks', days: 14 },
  { value: '1mo', label: 'Past 1 Month', days: 30 },
  { value: '3mo', label: 'Past 3 Months', days: 91 },
  { value: '6mo', label: 'Past 6 Months', days: 182 },
  { value: '1y', label: 'Past 1 Year', days: 365 },
];

export function rangeOf(value: string, end = today()): { from: number; to: number } {
  const r = RANGES.find((x) => x.value === value) ?? RANGES[2];
  return { from: end - (r.days - 1) * DAY, to: end + DAY - 1 };
}

// ---------------------------------------------------------------- providers

export const CLOUDS = ['aws', 'azure', 'gcp', 'oci'];

const PROVIDER_LABELS: Record<string, string> = { aws: 'AWS', azure: 'Azure', gcp: 'Google Cloud', oci: 'Oracle Cloud', kubernetes: 'Kubernetes' };
export const providerLabel = (p: string) => PROVIDER_LABELS[p] ?? p;
/** A provider's short name before a service ("Google GeminiAPI"). */
export const providerVendor = (p: string) => ({ aws: 'AWS', azure: 'Azure', gcp: 'Google', oci: 'Oracle' })[p] ?? p;

const PROVIDER_COLORS: Record<string, string> = { aws: '#e8890c', azure: '#2f7fd8', gcp: '#3aa757', oci: '#c74634' };
export function providerColor(p: string): string {
  if (PROVIDER_COLORS[p]) return PROVIDER_COLORS[p];
  let h = 0;
  for (const c of p) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return `hsl(${h % 360} 45% 45%)`;
}

export const isCloud = (p: string) => CLOUDS.includes(p);

// ---------------------------------------------------------------- filters

/** Selected filter values by tag; empty = all. */
export type Filters = Record<string, string[]>;

const quote = (v: string) => (/[\s():"]/.test(v) ? `"${v.replace(/"/g, '\\"')}"` : v);

/** Filters (and a base scope) as cost search syntax: `providername:aws team:(ml OR data)`. */
export function filterQuery(filters: Filters, base = ''): string {
  const parts = base ? [base] : [];
  for (const [k, vs] of Object.entries(filters)) {
    if (!vs.length) continue;
    parts.push(vs.length === 1 ? `${k}:${quote(vs[0])}` : `${k}:(${vs.map(quote).join(' OR ')})`);
  }
  return parts.join(' ');
}

export function useCosts(req: CostQueryRequest | null) {
  return useQuery({ queryKey: ['cloud-cost', 'query', req], queryFn: ({ signal }) => queryCosts(req!, signal), enabled: !!req, placeholderData: (prev) => prev });
}

/** Values of the cost tags over a range, for the filter dropdowns. */
export function useCostTags(range: { from: number; to: number }, query = '', costType: CostType = 'amortized') {
  return useQuery({ queryKey: ['cloud-cost', 'tags', range.from, range.to, query, costType], queryFn: ({ signal }) => costTags({ range, query, costType }, signal), staleTime: 60_000 });
}

export function tagValues(tags: CostTag[] | undefined, key: string): string[] {
  return tags?.find((t) => t.key === key)?.values.map((v) => v.value) ?? [];
}

/** The label of a group's tags ("aws, AmazonEC2"). */
export const groupLabel = (tags: Record<string, string>) => Object.values(tags).join(', ');
