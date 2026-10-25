// Renderers for metric-backed widgets. Each resolves template variables, fetches through the
// shared query hooks, and renders loading / error / empty states inside its frame.
import { useMemo, useState, type ReactNode } from 'react';
import type { EChartsOption } from 'echarts';
import { ArrowDown, ArrowUp, Search } from 'lucide-react';
import type { ConditionalFormat, WidgetDefinition } from '../../../api/dashboards/types';
import { Chart } from '../../../components/charts/Chart';
import { hhmm } from '../../../lib/format';
import { palette, token } from '../../../theme/chartTheme';
import { useDistributionData, useGroupedData, useScalarData, useTimeseriesData } from '../queries';
import { formatValue, useDashboardContext } from './context';

type Def<T extends WidgetDefinition['type']> = Extract<WidgetDefinition, { type: T }>;

export function WidgetState({ loading, error, empty, children }: { loading: boolean; error: unknown; empty?: boolean; children: ReactNode }) {
  if (loading) return <div className="wd-state"><span className="spinner" /></div>;
  if (error) return <div className="wd-state wd-error">{error instanceof Error ? error.message : 'Query failed'}</div>;
  if (empty) return <div className="wd-state">No data</div>;
  return <>{children}</>;
}

function axisLabel(v: number, span: number): string {
  if (span > 2 * 86400e3) return new Date(v).toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
  return hhmm(v);
}

// ---------------------------------------------------------------- timeseries

export function TimeseriesWidget({ def }: { def: Def<'timeseries'> }) {
  const { range, resolve } = useDashboardContext();
  const queries = def.requests.map((r) => resolve(r.query));
  const q = useTimeseriesData(queries, range);
  const option = useMemo<EChartsOption>(() => {
    const series = q.data ?? [];
    const span = range.to - range.from;
    const showLegend = series.length > 1 && series.length <= 12;
    return {
      grid: { left: 40, right: 10, top: 10, bottom: showLegend ? 46 : 22 },
      tooltip: { trigger: 'axis', confine: true, order: 'valueDesc' },
      legend: showLegend ? { type: 'scroll', bottom: 0, left: 0, right: 0, formatter: (n: string) => (n.length > 32 ? `${n.slice(0, 32)}…` : n) } : undefined,
      xAxis: { type: 'time', axisLabel: { formatter: (v: number) => axisLabel(v, span) } },
      yAxis: { type: 'value', max: def.yMax, axisLabel: { formatter: (v: number) => formatValue(v, 1) } },
      series: series.map((s, i) => ({
        name: def.requests.length > 1 && def.requests[i]?.alias ? def.requests[i].alias : s.name,
        type: def.display === 'bars' ? 'bar' : 'line',
        stack: def.stacked || def.display === 'bars' ? 'total' : undefined,
        areaStyle: def.display === 'area' ? { opacity: 0.55 } : undefined,
        lineStyle: { width: def.display === 'area' ? 0.8 : 1.4 },
        barMaxWidth: 10,
        data: s.points,
      })),
    };
  }, [q.data, def, range]);
  return (
    <WidgetState loading={q.isPending} error={q.error} empty={q.data?.length === 0}>
      <Chart option={option} height="100%" />
    </WidgetState>
  );
}

// ---------------------------------------------------------------- query value

const PALETTE_VAR: Record<ConditionalFormat['palette'], string> = {
  green: '--qv-green',
  yellow: '--qv-yellow',
  red: '--qv-red',
  gray: '--qv-gray',
  blue: '--qv-blue',
};

function pickFormat(v: number, formats: ConditionalFormat[]): ConditionalFormat | undefined {
  return formats.find((f) => (f.comparator === '>' ? v > f.value : f.comparator === '>=' ? v >= f.value : f.comparator === '<' ? v < f.value : v <= f.value));
}

export function QueryValueWidget({ def }: { def: Def<'query_value'> }) {
  const { range, resolve } = useDashboardContext();
  const q = useScalarData(resolve(def.request.query), def.aggregator, range);
  const v = q.data ?? 0;
  const fmt = pickFormat(v, def.formats);
  const color = fmt ? `var(${PALETTE_VAR[fmt.palette]})` : 'var(--text)';
  return (
    <WidgetState loading={q.isPending} error={q.error}>
      <div className={`qv${def.fill && fmt ? ' qv-fill' : ''}`} style={def.fill && fmt ? { background: color } : { color }}>
        <span className="qv-value">
          {formatValue(v, def.precision)}
          {def.unit && <small>{def.unit}</small>}
        </span>
      </div>
    </WidgetState>
  );
}

// ---------------------------------------------------------------- grouped (toplist, change, bar, pie, treemap, table)

export function ToplistWidget({ def }: { def: Def<'toplist'> }) {
  const { range, resolve } = useDashboardContext();
  const q = useGroupedData(resolve(def.request.query), def.aggregator, range);
  const rows = (q.data ?? []).slice(0, def.limit);
  const max = Math.max(1, ...rows.map((r) => r.value));
  return (
    <WidgetState loading={q.isPending} error={q.error} empty={rows.length === 0}>
      <ol className="tl">
        {rows.map((r) => (
          <li key={r.group} title={r.group}>
            <span className="tl-value">{formatValue(r.value, 1)}</span>
            <span className="tl-bar-wrap">
              <span className="tl-bar" style={{ width: `${(r.value / max) * 100}%` }} />
              <span className="tl-label">{Object.values(r.tags).join(', ') || r.group}</span>
            </span>
          </li>
        ))}
      </ol>
    </WidgetState>
  );
}

export function ChangeWidget({ def }: { def: Def<'change'> }) {
  const { range, resolve } = useDashboardContext();
  const query = resolve(def.request.query);
  const span = range.to - range.from;
  const prevRange = useMemo(() => ({ from: range.from - span, to: range.from }), [range.from, span]);
  const now = useGroupedData(query, def.aggregator, range);
  const prev = useGroupedData(query, def.aggregator, prevRange);
  const prevBy = new Map((prev.data ?? []).map((r) => [r.group, r.value]));
  const rows = (now.data ?? []).slice(0, def.limit).map((r) => ({ ...r, delta: r.value - (prevBy.get(r.group) ?? r.value) }));
  const max = Math.max(1, ...rows.map((r) => Math.abs(r.delta)));
  return (
    <WidgetState loading={now.isPending || prev.isPending} error={now.error ?? prev.error} empty={rows.length === 0}>
      <div className="chg">
        {rows.map((r) => (
          <div key={r.group} className="chg-row" title={r.group}>
            <span className="chg-label">{Object.values(r.tags).join(', ')}</span>
            <span className="chg-track">
              <span className={`chg-bar ${r.delta >= 0 ? 'up' : 'down'}`} style={{ width: `${Math.max(2, (Math.abs(r.delta) / max) * 100)}%` }} />
            </span>
            <span className={`chg-delta ${r.delta >= 0 ? 'up' : 'down'}`}>
              {r.delta >= 0 ? <ArrowUp size={11} /> : <ArrowDown size={11} />}
              {r.delta >= 0 ? '+' : ''}
              {formatValue(r.delta, 0)}
            </span>
          </div>
        ))}
      </div>
    </WidgetState>
  );
}

export function BarChartWidget({ def }: { def: Def<'bar_chart'> }) {
  const { range, resolve } = useDashboardContext();
  const q = useGroupedData(resolve(def.request.query), def.aggregator, range);
  const rows = (q.data ?? []).slice(0, def.limit);
  const option = useMemo<EChartsOption>(
    () => ({
      grid: { left: 40, right: 10, top: 10, bottom: 40 },
      tooltip: { trigger: 'axis', confine: true },
      xAxis: { type: 'category', data: rows.map((r) => Object.values(r.tags).join(', ')), axisLabel: { interval: 0, rotate: rows.length > 5 ? 30 : 0, fontSize: 10 } },
      yAxis: { type: 'value', axisLabel: { formatter: (v: number) => formatValue(v, 1) } },
      series: [{ type: 'bar', data: rows.map((r, i) => ({ value: r.value, itemStyle: { color: palette()[i % 8] } })), barMaxWidth: 36 }],
    }),
    [rows],
  );
  return (
    <WidgetState loading={q.isPending} error={q.error} empty={rows.length === 0}>
      <Chart option={option} height="100%" />
    </WidgetState>
  );
}

export function PieWidget({ def }: { def: Def<'pie'> | Def<'treemap'> }) {
  const { range, resolve } = useDashboardContext();
  const q = useGroupedData(resolve(def.request.query), def.aggregator, range);
  const rows = q.data ?? [];
  const option = useMemo<EChartsOption>(() => {
    const data = rows.map((r, i) => ({ name: Object.values(r.tags).join(', ') || r.group, value: r.value, itemStyle: { color: palette()[i % 8] } }));
    if (def.type === 'treemap')
      return {
        tooltip: { formatter: (p: unknown) => { const x = p as { name: string; value: number }; return `${x.name}: ${formatValue(x.value, 1)}`; } },
        series: [{ type: 'treemap', roam: false, nodeClick: false, breadcrumb: { show: false }, width: '100%', height: '100%', itemStyle: { borderColor: token('--bg'), borderWidth: 2 }, label: { fontSize: 12 }, data }],
      };
    return {
      tooltip: { trigger: 'item', confine: true },
      legend: { type: 'scroll', orient: 'vertical', right: 0, top: 'middle', textStyle: { fontSize: 11 } },
      series: [{ type: 'pie', radius: ['45%', '75%'], center: ['35%', '50%'], label: { show: false }, data }],
    };
  }, [rows, def.type]);
  return (
    <WidgetState loading={q.isPending} error={q.error} empty={rows.length === 0}>
      <Chart option={option} height="100%" />
    </WidgetState>
  );
}

export function TableWidget({ def }: { def: Def<'table'> }) {
  const { range, resolve } = useDashboardContext();
  const [search, setSearch] = useState('');
  const [sort, setSort] = useState<{ col: number; dir: 1 | -1 }>({ col: 0, dir: -1 });
  // Up to 4 columns, each its own grouped query (hooks need a fixed count).
  const cols = [0, 1, 2, 3].map((i) => def.requests[i]);
  const r0 = useGroupedData(cols[0] ? resolve(cols[0].query) : '', def.aggregator, range);
  const r1 = useGroupedData(cols[1] ? resolve(cols[1].query) : '', def.aggregator, range);
  const r2 = useGroupedData(cols[2] ? resolve(cols[2].query) : '', def.aggregator, range);
  const r3 = useGroupedData(cols[3] ? resolve(cols[3].query) : '', def.aggregator, range);
  const results = [r0, r1, r2, r3].slice(0, def.requests.length);
  const keys = Object.keys(r0.data?.[0]?.tags ?? {});
  // Result objects change identity every render; depend on their data instead.
  const rows = useMemo(() => {
    const maps = results.map((r) => new Map((r.data ?? []).map((g) => [g.group, g.value])));
    return (r0.data ?? [])
      .map((g) => ({ tags: g.tags, group: g.group, values: maps.map((m) => m.get(g.group)) }))
      .filter((row) => !search || Object.values(row.tags).join(' ').toLowerCase().includes(search.toLowerCase()))
      .sort((a, b) => ((a.values[sort.col] ?? 0) - (b.values[sort.col] ?? 0)) * sort.dir)
      .slice(0, def.limit);
  }, [r0.data, r1.data, r2.data, r3.data, search, sort, def.limit]);
  return (
    <WidgetState loading={results.some((r) => r.isPending)} error={results.find((r) => r.error)?.error}>
      <div className="wt">
        {def.searchable && (
          <label className="wt-search">
            <Search size={13} />
            <input placeholder="Search" value={search} onChange={(e) => setSearch(e.target.value)} />
          </label>
        )}
        <div className="wt-scroll">
          <table>
            <thead>
              <tr>
                {keys.map((k) => (
                  <th key={k}>#{k.toUpperCase()}</th>
                ))}
                {def.requests.map((r, i) => (
                  <th key={i} className="num" onClick={() => setSort((s) => ({ col: i, dir: s.col === i ? ((-s.dir) as 1 | -1) : -1 }))}>
                    {sort.col === i && (sort.dir < 0 ? '↓ ' : '↑ ')}
                    {(r.alias ?? r.query).toUpperCase()}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((row) => (
                <tr key={row.group}>
                  {keys.map((k) => (
                    <td key={k}>{row.tags[k]}</td>
                  ))}
                  {row.values.map((v, i) => (
                    <td key={i} className="num">
                      {v === undefined || v === 0 ? '—' : formatValue(v, 1)}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
    </WidgetState>
  );
}

// ---------------------------------------------------------------- heatmap / distribution

export function HeatmapWidget({ def }: { def: Def<'heatmap'> | Def<'distribution'> }) {
  const { range, resolve } = useDashboardContext();
  const q = useDistributionData(resolve(def.request.query), range);
  const option = useMemo<EChartsOption>(() => {
    const d = q.data;
    if (!d) return {};
    if (def.type === 'distribution') {
      const totals = d.buckets.map((_, bi) => d.counts.reduce((a, row) => a + row[bi], 0));
      return {
        grid: { left: 40, right: 10, top: 10, bottom: 26 },
        tooltip: { trigger: 'axis', confine: true },
        xAxis: { type: 'category', data: d.buckets.map((b) => `≤${formatValue(b, 0)}`) },
        yAxis: { type: 'value' },
        series: [{ type: 'bar', data: totals, barCategoryGap: '6%', color: token('--chart-2') }],
      };
    }
    const max = Math.max(1, ...d.counts.flat());
    return {
      grid: { left: 44, right: 10, top: 8, bottom: 24 },
      tooltip: { confine: true },
      xAxis: { type: 'category', data: d.times.map((t) => hhmm(t)), splitArea: { show: false } },
      yAxis: { type: 'category', data: d.buckets.map((b) => formatValue(b, 0)) },
      visualMap: { show: false, min: 0, max, inRange: { color: [token('--bg-muted'), token('--chart-2'), '#3b1f8f'] } },
      series: [{ type: 'heatmap', data: d.counts.flatMap((row, ti) => row.map((c, bi) => [ti, bi, c])), itemStyle: { borderColor: token('--bg'), borderWidth: 1 } }],
    };
  }, [q.data, def.type]);
  return (
    <WidgetState loading={q.isPending} error={q.error}>
      <Chart option={option} height="100%" />
    </WidgetState>
  );
}
