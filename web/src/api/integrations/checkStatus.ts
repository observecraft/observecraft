// Datadog's check status widget (`check_status`) over our service checks: the `service_check.status` gauge
// (0 OK, 1 WARNING, 2 CRITICAL, 3 UNKNOWN) tagged `check:<name>`, one series per run of the check (host and tags).
import type { MetricSeries } from '../types';

export interface CheckStatusDefinition {
  check?: unknown;
  grouping?: unknown;
  /** `grouping: check`: the one group (`host:a,env:prod`) the widget shows. */
  group?: unknown;
  /** `grouping: cluster`: tags whose values each get a status of their own. */
  group_by?: unknown;
  tags?: unknown;
  [key: string]: unknown;
}

export interface CheckStatusTile {
  /** The tile's group-by values (`endpoint:…`), empty for the whole cluster. */
  label: string;
  /** The worst status among its runs. */
  status: number;
  /** Runs in each status: OK, WARNING, CRITICAL, UNKNOWN. */
  counts: [number, number, number, number];
}

const strings = (v: unknown): string[] => (Array.isArray(v) ? v.filter((t): t is string => typeof t === 'string' && t.trim() !== '').map((t) => t.trim()) : []);

/** The group-by tags of a cluster (template variables written in `group_by` filter instead, as later versions of the same dashboards write them). */
function groupKeys(def: CheckStatusDefinition): string[] {
  return def.grouping === 'check' ? [] : strings(def.group_by).filter((t) => !t.startsWith('$'));
}

/** The metric query of the widget's runs: one series per host and group-by value, the worst status in each step. */
export function checkStatusQuery(def: CheckStatusDefinition, resolve: (query: string) => string): string {
  const check = typeof def.check === 'string' ? def.check : '';
  const group = def.grouping === 'check' && typeof def.group === 'string' ? def.group.split(',') : [];
  const variables = def.grouping === 'check' ? [] : strings(def.group_by).filter((t) => t.startsWith('$'));
  const filters = [`check:${check}`, ...group, ...strings(def.tags), ...variables].map((t) => t.trim()).filter((t, i, all) => t && t !== '*' && all.indexOf(t) === i);
  return resolve(`max:service_check.status{${filters.join(',')}} by {${[...new Set(['host', ...groupKeys(def)])].join(',')}}`);
}

/** Worse first: CRITICAL, WARNING, UNKNOWN, OK. */
const SEVERITY = [0, 2, 3, 1];

/** The series' tag pairs (`host:a, endpoint:b`). */
function pairs(name: string): Map<string, string> {
  if (name === '*') return new Map();
  return new Map(
    name.split(', ').map((pair) => {
      const i = pair.indexOf(':');
      return i < 0 ? [pair, ''] : [pair.slice(0, i), pair.slice(i + 1)];
    }),
  );
}

/** A tile per group-by value (one for the whole cluster without group_by): the worst latest status and the runs in each, worst tiles first. */
export function checkStatusTiles(def: CheckStatusDefinition, series: MetricSeries[]): CheckStatusTile[] {
  const keys = groupKeys(def);
  const tiles = new Map<string, CheckStatusTile>();
  for (const s of series) {
    const last = s.points.at(-1)?.[1];
    if (last === undefined) continue;
    const status = Math.min(3, Math.max(0, Math.round(last)));
    const tags = s.tags ? new Map(Object.entries(s.tags)) : pairs(s.name);
    const label = keys.map((k) => `${k}:${tags.get(k) ?? 'N/A'}`).join(', ');
    const tile = tiles.get(label) ?? { label, status, counts: [0, 0, 0, 0] };
    tile.counts[status]++;
    if (SEVERITY[status] > SEVERITY[tile.status]) tile.status = status;
    tiles.set(label, tile);
  }
  return [...tiles.values()].sort((a, b) => SEVERITY[b.status] - SEVERITY[a.status] || a.label.localeCompare(b.label));
}
