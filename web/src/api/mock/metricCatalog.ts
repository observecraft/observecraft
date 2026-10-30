// Mock Metrics Catalog: volume from the mock metrics engine's series, related assets from the other mocks.
import type { CatalogStats, MetricDetail, VolumePoint } from '../metricCatalog/client';
import type { TimeRange } from '../types';
import { metricEntities, metricVolume } from './metricsEngine';
import { getDashboard, searchDashboards } from './dashboards';
import { getNotebook, searchNotebooks } from './notebooks';
import { searchMonitors } from './monitors';
import { searchSlos } from './slos';

const DAY = 86_400_000;

/** A stable 0..1 per name. */
function unit(name: string): number {
  let h = 2166136261;
  for (let i = 0; i < name.length; i++) h = Math.imul(h ^ name.charCodeAt(i), 16777619);
  return (h >>> 0) / 2 ** 32;
}

function timeline(range: TimeRange, series: number, seed: string): VolumePoint[] {
  const step = range.to - range.from <= 2 * DAY ? 300_000 : 3_600_000;
  const out: VolumePoint[] = [];
  for (let t = Math.floor(range.from / step) * step; t <= range.to; t += step) {
    out.push({ t, series: Math.max(0, Math.round(series * (0.92 + 0.08 * Math.sin(t / 3_600_000 + unit(seed) * 6)))) });
  }
  return out;
}

export async function catalogStats(range: TimeRange): Promise<CatalogStats> {
  const { metrics, totalSeries } = metricVolume();
  return {
    metrics: metrics.map((m) => ({
      name: m.name,
      kind: m.kind,
      series: m.series,
      previousSeries: Math.round(m.series * (0.7 + unit(m.name) * 0.6)),
      firstSeen: Date.now() - Math.round((1 + unit(m.name) * 700) * DAY),
    })),
    timeline: timeline(range, totalSeries, 'all'),
  };
}

const wholeWord = (name: string) => new RegExp(`(^|[^A-Za-z0-9_.])${name.replace(/[.*+?^${}()|[\]\\-]/g, '\\$&')}([^A-Za-z0-9_.]|$)`);

export async function metricDetail(name: string, range: TimeRange): Promise<MetricDetail> {
  const entities = metricEntities(name) ?? [];
  const keys = new Map<string, Set<string>>();
  for (const e of entities) for (const [k, v] of Object.entries(e)) (keys.get(k) ?? keys.set(k, new Set()).get(k)!).add(v);
  const tags = [...keys].map(([key, values]) => ({ key, count: values.size, values: [...values].sort().slice(0, 50) })).sort((a, b) => a.key.localeCompare(b.key));
  const re = wholeWord(name);
  const page = await searchDashboards({ limit: 500, offset: 0, query: '', preset: 'all', sort: { field: 'title', dir: 'asc' } } as Parameters<typeof searchDashboards>[0]);
  const dashboards = (await Promise.all(page.dashboards.map((d) => getDashboard(d.id).catch(() => null)))).filter((d) => d && re.test(JSON.stringify(d.widgets)));
  const notebookPage = await searchNotebooks({ limit: 200, offset: 0, query: '', view: 'all', sort: 'title', dir: 'asc' });
  const notebooks = (await Promise.all(notebookPage.notebooks.map((n) => getNotebook(n.id).catch(() => null)))).filter((n) => n && re.test(JSON.stringify(n.cells)));
  const monitors = searchMonitors({ limit: 500, offset: 0, query: '' } as Parameters<typeof searchMonitors>[0]).monitors.filter((m) => re.test(m.query));
  const slos = (
    await searchSlos({ limit: 500, offset: 0, query: '', burnRates: [], creators: [], envs: [], services: [], states: [], tags: [], teams: [], types: [], sort: null, window: '30d' } as Parameters<
      typeof searchSlos
    >[0])
  ).slos.filter((s) => re.test(JSON.stringify(s)));
  return {
    name,
    series: entities.length,
    hosts: keys.get('host')?.size ?? 0,
    tagValues: tags.reduce((n, t) => n + t.count, 0),
    firstSeen: entities.length ? Date.now() - Math.round((1 + unit(name) * 700) * DAY) : null,
    tags,
    timeline: timeline(range, entities.length, name),
    related: {
      dashboards: dashboards.map((d) => ({ id: d!.id, title: d!.title })),
      notebooks: notebooks.map((n) => ({ id: n!.id, title: n!.title })),
      monitors: monitors.map((m) => ({ id: String(m.id), title: m.name })),
      slos: slos.map((s) => ({ id: s.id, title: s.name })),
    },
  };
}
