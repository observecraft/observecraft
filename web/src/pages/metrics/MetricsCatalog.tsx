import { useMemo, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { useQuery } from '@tanstack/react-query';
import { ArrowDownRight, ArrowUpRight, ArrowDownWideNarrow, ArrowUpNarrowWide, Download, PanelLeftClose, PanelLeftOpen, Search } from 'lucide-react';
import type { EChartsOption } from 'echarts';
import { catalogStats, type MetricStats } from '../../api/metricCatalog/client';
import { listMetricRules, metricRulesOverview } from '../../api/metricRules/client';
import { listLogMetrics } from '../../api/logMetrics/client';
import { Chart } from '../../components/charts/Chart';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Pager } from '../../components/ui/Pager';
import { TimePicker } from '../../components/shell/PageHeader';
import { useTime } from '../../components/shell/TimeContext';
import { timeAgo } from '../../lib/format';
import { token } from '../../theme/chartTheme';
import { useMetricCatalog } from '../dashboards/queries';
import { Facet, FacetCheck, toggle } from '../monitors/common';
import { KIND_LABEL } from './MetricsLayout';
import { MetricPanel } from './MetricPanel';
import { compactCount, distributionBase, downloadCsv, lastTagConfiguration, metricOrigin, ORIGINS, TYPE_GROUPS, typeGroup, type Origin, type TypeGroup } from './catalog';

const PAGE = 50;

type SortKey = 'name' | 'firstSeen' | 'tagConfig' | 'ingested' | 'indexed' | 'change';

export interface CatalogRow {
  name: string;
  kind: string;
  unit?: string | null;
  description: string;
  tags: string[];
  origin: Origin;
  group: TypeGroup;
  firstSeen?: number | null;
  tagConfig?: number;
  ingested: number;
  indexed: number;
  change: number;
}

/** Series per bucket as bars, for the catalog's volume charts. */
function volumeOption(points: [number, number][], color: string): EChartsOption {
  return {
    grid: { left: 8, right: 12, top: 12, bottom: 22, containLabel: true },
    tooltip: { trigger: 'axis', confine: true, valueFormatter: (v) => Number(v).toLocaleString('en-US') },
    xAxis: { type: 'time', axisLabel: { hideOverlap: true } },
    yAxis: { type: 'value', axisLabel: { formatter: (v: number) => compactCount(v) } },
    series: [{ type: 'bar', data: points, color, barMaxWidth: 12 }],
  };
}

/** Metrics → Catalog: every metric with its volume, origin and type; a row opens its details. */
export function MetricsCatalog() {
  const [params, setParams] = useSearchParams();
  const { range } = useTime();
  const catalog = useMetricCatalog();
  const stats = useQuery({ queryKey: ['metrics', 'catalog-stats', range], queryFn: ({ signal }) => catalogStats(range, signal) });
  const overview = useQuery({ queryKey: ['metrics', 'rules-overview', range], queryFn: ({ signal }) => metricRulesOverview(range, signal) });
  const rules = useQuery({ queryKey: ['metrics', 'rules'], queryFn: listMetricRules });
  const logMetrics = useQuery({ queryKey: ['logs', 'metrics'], queryFn: listLogMetrics });
  const [text, setText] = useState(params.get('q') ?? '');
  const [tagText, setTagText] = useState('');
  const [origins, setOrigins] = useState<Origin[]>([]);
  const [groups, setGroups] = useState<TypeGroup[]>([]);
  const [controls, setControls] = useState(true);
  const [sort, setSort] = useState<{ key: SortKey; desc: boolean }>({ key: 'name', desc: false });
  const [offset, setOffset] = useState(0);
  const selected = params.get('metric');
  const select = (name: string | null) =>
    setParams(
      (p) => {
        const next = new URLSearchParams(p);
        if (name) next.set('metric', name);
        else next.delete('metric');
        return next;
      },
      { replace: true },
    );

  const rows = useMemo<CatalogRow[]>(() => {
    const byName = new Map((stats.data?.metrics ?? []).map((m) => [m.name, m]));
    const indexed = new Map([...(overview.data?.filtered ?? []), ...(overview.data?.top ?? [])].map((m) => [m.name, m.indexed]));
    const logs = new Set((logMetrics.data ?? []).map((m) => m.name));
    return (catalog.data ?? []).map((m) => {
      // Distribution statistics share their distribution's series.
      const s: MetricStats | undefined = byName.get(m.name) ?? byName.get(distributionBase(m));
      const ingested = s?.series ?? 0;
      return {
        name: m.name,
        kind: m.kind ?? 'gauge',
        unit: m.unit,
        description: m.description,
        tags: m.tags,
        origin: metricOrigin(m, logs),
        group: typeGroup(m.kind),
        firstSeen: s?.firstSeen,
        tagConfig: lastTagConfiguration(m.name, rules.data ?? []),
        ingested,
        indexed: indexed.get(m.name) ?? ingested,
        change: s ? s.series - s.previousSeries : 0,
      };
    });
  }, [catalog.data, stats.data, overview.data, rules.data, logMetrics.data]);

  if (catalog.isPending) return <Loading label="Loading metrics…" />;
  if (catalog.isError) return <ErrorState error={catalog.error} onRetry={() => catalog.refetch()} />;

  const t = text.trim().toLowerCase();
  const tag = tagText.trim().toLowerCase();
  // Facet counts ignore their own facet's selection, so other values stay pickable.
  const byText = rows.filter((r) => (!t || r.name.toLowerCase().includes(t)) && (!tag || r.tags.some((g) => g.toLowerCase().includes(tag))));
  const count = <K extends 'origin' | 'group'>(key: K, list: CatalogRow[]) => list.reduce<Record<string, number>>((acc, r) => ((acc[r[key]] = (acc[r[key]] ?? 0) + 1), acc), {});
  const originCounts = count(
    'origin',
    byText.filter((r) => !groups.length || groups.includes(r.group)),
  );
  const groupCounts = count(
    'group',
    byText.filter((r) => !origins.length || origins.includes(r.origin)),
  );
  const filtered = byText.filter((r) => (!origins.length || origins.includes(r.origin)) && (!groups.length || groups.includes(r.group)));
  const value = (r: CatalogRow): string | number => (sort.key === 'name' ? r.name : sort.key === 'firstSeen' ? (r.firstSeen ?? 0) : sort.key === 'tagConfig' ? (r.tagConfig ?? 0) : r[sort.key]);
  const sorted = [...filtered].sort((a, b) => {
    const [x, y] = [value(a), value(b)];
    const c = typeof x === 'string' ? x.localeCompare(y as string) : (x as number) - (y as number);
    return sort.desc ? -c : c;
  });
  const page = sorted.slice(offset, offset + PAGE);
  const timeline = (stats.data?.timeline ?? []).map((p) => [p.t, p.series] as [number, number]);
  // Indexing controls keep fewer series queryable: the indexed share of the metrics they cover.
  const ingestedNow = (stats.data?.metrics ?? []).reduce((n, m) => n + m.series, 0);
  const dropped = (overview.data?.filtered ?? []).reduce((n, m) => n + Math.max(0, m.ingested - m.indexed), 0);
  const indexedShare = ingestedNow ? Math.max(0, 1 - dropped / ingestedNow) : 1;
  const header = (key: SortKey, label: string, numeric = false) => (
    <th
      className={numeric ? 'num' : undefined}
      onClick={() => {
        setSort((s) => ({ key, desc: s.key === key ? !s.desc : numeric || key === 'firstSeen' }));
        setOffset(0);
      }}
    >
      {sort.key === key && (sort.desc ? <ArrowDownWideNarrow size={13} className="mc-sort" /> : <ArrowUpNarrowWide size={13} className="mc-sort" />)}
      {label}
    </th>
  );
  const explorer = (metric: string) => `/metrics?metric=${encodeURIComponent(metric)}`;
  return (
    <div className="mx-page mc-page">
      <div className="mc-head">
        <TimePicker />
      </div>
      <div className="mc-charts">
        <section>
          <h3>Total Estimated Ingested Series</h3>
          <p className="muted">Distinct tag combinations every metric reported, per interval.</p>
          <div className="mc-chart">{stats.isPending ? <Loading /> : <Chart option={volumeOption(timeline, token('--chart-1'))} height="100%" />}</div>
        </section>
        <section>
          <h3>Total Estimated Indexed Series</h3>
          <p className="muted">
            The series that stay queryable once <Link to="/metrics/settings/rules">indexing controls</Link> keep only some tags.
          </p>
          <div className="mc-chart">
            {stats.isPending ? (
              <Loading />
            ) : (
              <Chart
                option={volumeOption(
                  timeline.map(([at, v]) => [at, Math.round(v * indexedShare)]),
                  token('--chart-2'),
                )}
                height="100%"
              />
            )}
          </div>
        </section>
      </div>
      <div className="mc-search">
        <label>
          <span className="mc-search-label">Metric</span>
          <Search size={15} />
          <input
            placeholder="Search metrics"
            value={text}
            onChange={(e) => {
              setText(e.target.value);
              setOffset(0);
            }}
          />
        </label>
        <label>
          <span className="mc-search-label">Tag</span>
          <input
            placeholder="Filter by tag key"
            value={tagText}
            onChange={(e) => {
              setTagText(e.target.value);
              setOffset(0);
            }}
          />
        </label>
      </div>
      <div className={`mc-body${controls ? '' : ' mc-no-controls'}`}>
        {controls && (
          <aside className="mc-facets">
            <p className="muted">{origins.length || groups.length ? `${origins.length + groups.length} facets applied` : 'No facets applied'}</p>
            <Facet title="Metric Type">
              {TYPE_GROUPS.map((g) => (
                <FacetCheck key={g} checked={!groups.length || groups.includes(g)} onChange={() => (setGroups((x) => toggle(x, g)), setOffset(0))} label={g} count={groupCounts[g] ?? 0} />
              ))}
            </Facet>
            <Facet title="Metric Origin">
              {ORIGINS.map((o) => (
                <FacetCheck key={o} checked={!origins.length || origins.includes(o)} onChange={() => (setOrigins((x) => toggle(x, o)), setOffset(0))} label={o} count={originCounts[o] ?? 0} />
              ))}
            </Facet>
          </aside>
        )}
        <div className="mc-results">
          <div className="mc-toolbar">
            <button className="link-btn" onClick={() => setControls((c) => !c)}>
              {controls ? <PanelLeftClose size={15} /> : <PanelLeftOpen size={15} />} {controls ? 'Hide Controls' : 'Show Controls'}
            </button>
            <span className="muted mc-total">{sorted.length ? `${offset + 1}–${Math.min(offset + PAGE, sorted.length)} of ${sorted.length.toLocaleString()}` : 'No metrics'}</span>
            <button
              className="btn btn-sm"
              title="Download as CSV"
              onClick={() =>
                downloadCsv(
                  'metrics.csv',
                  ['metric', 'type', 'origin', 'first_submitted', 'ingested_series', 'indexed_series', 'change'],
                  sorted.map((r) => [r.name, r.kind, r.origin, r.firstSeen ? new Date(r.firstSeen).toISOString() : '', r.ingested, r.indexed, r.change]),
                )
              }
            >
              <Download size={14} />
            </button>
          </div>
          <div className="sx-results">
            <table className="sx-table mc-table">
              <thead>
                <tr>
                  {header('name', 'METRIC NAME')}
                  <th>TYPE</th>
                  {header('firstSeen', 'FIRST SUBMITTED')}
                  {header('tagConfig', 'LAST TAG CONFIGURATION')}
                  {header('ingested', 'EST. INGESTED SERIES', true)}
                  {header('indexed', 'EST. INDEXED SERIES', true)}
                  {header('change', 'CHANGE IN VOLUME', true)}
                </tr>
              </thead>
              <tbody>
                {page.map((r) => (
                  <tr key={r.name} className={selected === r.name ? 'selected' : undefined} onClick={() => select(r.name)}>
                    <td className="mono ellipsis" title={r.description || r.name}>
                      {r.name}
                    </td>
                    <td>
                      <span className="mx-kind">{KIND_LABEL[r.kind] ?? r.kind}</span>
                    </td>
                    <td className="muted">{r.firstSeen ? timeAgo(r.firstSeen) : '—'}</td>
                    <td className="muted">{r.tagConfig ? timeAgo(r.tagConfig) : '—'}</td>
                    <td className="num">{stats.isPending ? '…' : compactCount(r.ingested)}</td>
                    <td className="num">{stats.isPending ? '…' : compactCount(r.indexed)}</td>
                    <td className="num">
                      {r.change === 0 ? (
                        <span className="muted">—</span>
                      ) : (
                        <span className={`mc-change ${r.change > 0 ? 'up' : 'down'}`}>
                          {compactCount(Math.abs(r.change))} {r.change > 0 ? <ArrowUpRight size={13} /> : <ArrowDownRight size={13} />}
                        </span>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <Pager offset={offset} limit={PAGE} total={sorted.length} onChange={setOffset} />
        </div>
      </div>
      {selected && <MetricPanel row={rows.find((r) => r.name === selected)} name={selected} explorerLink={explorer(selected)} rules={rules.data ?? []} onClose={() => select(null)} />}
    </div>
  );
}
