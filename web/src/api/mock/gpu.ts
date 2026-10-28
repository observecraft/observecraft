// In-browser GPU Monitoring for mock mode: a small fictional GPU fleet of the demo shop (model
// training, search ranking, image embeddings), evaluated like the API: current values over the last
// minutes, states from utilization, facets over every device and the query applied to the list.
import type { GpuDevice, GpuFacet, GpuSearchPage, GpuSearchRequest, GpuSeries, GpuSource, GpuState, GpuSummary, GpuTimeseriesRequest } from '../gpu/client';
import { parseQuery } from '../../lib/searchQuery';
import { rng } from './generator';
import { matchesTerms } from './match';

const GIB = 1024 ** 3;
/** Below 5 % utilization a GPU is idle; below 30 % it is underutilized (the API's thresholds). */
export const IDLE_PCT = 5;
export const UNDERUSED_PCT = 30;

export function gpuState(util: number | null | undefined): GpuState {
  if (util == null) return 'no_data';
  if (util < IDLE_PCT) return 'idle';
  if (util < UNDERUSED_PCT) return 'underutilized';
  return 'active';
}

interface Seed {
  host: string;
  index: number;
  model: string;
  memGiB: number;
  cluster: string | null;
  /** Typical utilization (%), the swing around it and the power draw at full load (W). */
  util: number;
  swing: number;
  maxPowerW: number;
  workloads: string[];
  source: GpuSource;
}

const SEEDS: Seed[] = [
  ...[0, 1, 2, 3].map((i) => ({
    host: 'i-0a1b2c3d4e5f60007',
    index: i,
    model: 'NVIDIA A10G',
    memGiB: 24,
    cluster: 'shop-ml',
    util: [88, 84, 91, 79][i],
    swing: 8,
    maxPowerW: 300,
    workloads: ['ml/recs-trainer-0'],
    source: 'datadog_gpu' as const,
  })),
  ...[0, 1].map((i) => ({
    host: 'i-0a1b2c3d4e5f60008',
    index: i,
    model: 'NVIDIA L4',
    memGiB: 24,
    cluster: 'shop-ml',
    util: [46, 14][i],
    swing: 6,
    maxPowerW: 72,
    workloads: ['search/search-ranker-6c9d'],
    source: 'datadog_gpu' as const,
  })),
  ...[0, 1].map((i) => ({
    host: 'i-0a1b2c3d4e5f60009',
    index: i,
    model: 'NVIDIA L4',
    memGiB: 24,
    cluster: 'shop-ml',
    util: [0.4, 1.2][i],
    swing: 0.6,
    maxPowerW: 72,
    workloads: [],
    source: 'datadog_gpu' as const,
  })),
  { host: 'i-0a1b2c3d4e5f6000a', index: 0, model: 'NVIDIA T4', memGiB: 16, cluster: null, util: 34, swing: 10, maxPowerW: 70, workloads: ['catalog/image-embedder-5f7b'], source: 'dcgm_exporter' },
  { host: 'i-0a1b2c3d4e5f6000a', index: 1, model: 'NVIDIA T4', memGiB: 16, cluster: null, util: 3, swing: 2, maxPowerW: 70, workloads: [], source: 'dcgm_exporter' },
];

const uuid = (s: Seed) => `GPU-${s.host.slice(-4)}${s.index}a7c-4e1b-9d2f-0c3a5b6e7f8${s.index}`;

/** One device's values at time `t` (stable for a 30 s slot). */
function deviceAt(s: Seed, t: number): GpuDevice {
  const r = rng(Math.imul(Math.floor(t / 30_000), 2654435761) ^ (s.index * 97 + s.host.charCodeAt(s.host.length - 1) * 7919));
  r();
  const util = Math.max(0, Math.min(100, s.util + (r() - 0.5) * 2 * s.swing));
  const memUsed = s.memGiB * GIB * Math.min(0.95, 0.04 + (util / 100) * 0.8 + r() * 0.05);
  const utilPct = Math.round(util * 10) / 10;
  return {
    id: `${s.host}/${uuid(s).toLowerCase()}`,
    host: s.host,
    uuid: uuid(s),
    index: String(s.index),
    model: s.model,
    vendor: 'nvidia',
    driver: '550.90.07',
    cluster: s.cluster,
    utilPct,
    memUsedBytes: Math.round(memUsed),
    memTotalBytes: s.memGiB * GIB,
    tempC: Math.round((34 + util * 0.42 + r() * 3) * 10) / 10,
    powerW: Math.round(s.maxPowerW * (0.12 + (util / 100) * 0.8) * 10) / 10,
    workloads: s.workloads,
    state: gpuState(utilPct),
    sources: [s.source],
  };
}

function tags(d: GpuDevice): Record<string, string> {
  const out: Record<string, string> = { host: d.host, state: d.state };
  if (d.cluster) out.cluster = d.cluster;
  if (d.model) out.model = d.model;
  if (d.vendor) out.vendor = d.vendor;
  const ns = d.workloads.find((w) => w.includes('/'))?.split('/')[0];
  if (ns) out.kube_namespace = ns;
  return out;
}

const FACET_KEYS = ['cluster', 'host', 'model', 'vendor', 'state', 'kube_namespace'];

function facets(devices: GpuDevice[]): GpuFacet[] {
  return FACET_KEYS.map((key) => {
    const counts = new Map<string, number>();
    for (const d of devices) {
      const v = tags(d)[key];
      if (v !== undefined) counts.set(v, (counts.get(v) ?? 0) + 1);
    }
    const values = [...counts].map(([value, count]) => ({ value, count })).sort((a, b) => b.count - a.count || a.value.localeCompare(b.value));
    return { key, values };
  }).filter((f) => f.values.length > 0);
}

export function summarize(devices: GpuDevice[]): GpuSummary {
  const utils = devices.map((d) => d.utilPct).filter((u): u is number => u != null);
  return {
    gpus: devices.length,
    hosts: new Set(devices.map((d) => d.host)).size,
    avgUtilPct: utils.length ? Math.round((utils.reduce((a, b) => a + b, 0) / utils.length) * 10) / 10 : null,
    memUsedBytes: devices.reduce((a, d) => a + (d.memUsedBytes ?? 0), 0),
    memTotalBytes: devices.reduce((a, d) => a + (d.memTotalBytes ?? 0), 0),
    idle: devices.filter((d) => d.state === 'idle').length,
    underutilized: devices.filter((d) => d.state === 'underutilized').length,
  };
}

function filtered(devices: GpuDevice[], query: string): GpuDevice[] {
  const terms = parseQuery(query);
  return devices.filter((d) => matchesTerms((k) => tags(d)[k], `${d.id} ${d.model ?? ''} ${d.workloads.join(' ')}`, terms));
}

export async function searchGpus(req: GpuSearchRequest): Promise<GpuSearchPage> {
  const all = SEEDS.map((s) => deviceAt(s, req.range.to));
  const devices = filtered(all, req.query ?? '');
  return { enabled: true, summary: summarize(devices), devices, facets: facets(all) };
}

const SERIES: [string, string][] = [
  ['utilization', 'percent'],
  ['memory_used', 'byte'],
  ['temperature', 'degree celsius'],
  ['power', 'watt'],
];

/** Per interval and group: utilization and temperature average, memory and power add up (as the API does). */
export async function gpuTimeseries(req: GpuTimeseriesRequest): Promise<GpuSeries[]> {
  const step = Math.max(30_000, Math.ceil((req.range.to - req.range.from) / 60 / 30_000) * 30_000);
  const groupBy = req.groupBy || null;
  const out = new Map<string, GpuSeries>();
  for (let t = Math.ceil(req.range.from / step) * step; t <= req.range.to; t += step) {
    const groups = new Map<string | null, GpuDevice[]>();
    for (const d of filtered(
      SEEDS.map((s) => deviceAt(s, t)),
      req.query ?? '',
    )) {
      const key = groupBy ? (tags(d)[groupBy] ?? 'N/A') : null;
      groups.set(key, [...(groups.get(key) ?? []), d]);
    }
    for (const [group, devices] of groups) {
      const values = [devices.map((d) => d.utilPct), devices.map((d) => d.memUsedBytes), devices.map((d) => d.tempC), devices.map((d) => d.powerW)].map((xs) =>
        xs.filter((x): x is number => x != null),
      );
      values.forEach((xs, i) => {
        if (!xs.length) return;
        const total = xs.reduce((a, b) => a + b, 0);
        const v = i === 0 || i === 2 ? Math.round((total / xs.length) * 10) / 10 : i === 3 ? Math.round(total * 10) / 10 : total;
        const id = `${i}\u0000${group}`;
        const series = out.get(id) ?? { metric: SERIES[i][0], group, unit: SERIES[i][1], points: [] };
        series.points.push([t, v]);
        out.set(id, series);
      });
    }
  }
  const series = [...out.values()].sort((a, b) => SERIES.findIndex(([n]) => n === a.metric) - SERIES.findIndex(([n]) => n === b.metric) || (a.group ?? '').localeCompare(b.group ?? ''));
  if (!groupBy) for (const [metric, unit] of SERIES) if (!series.some((s) => s.metric === metric)) series.push({ metric, group: null, unit, points: [] });
  return series.sort((a, b) => SERIES.findIndex(([n]) => n === a.metric) - SERIES.findIndex(([n]) => n === b.metric));
}
