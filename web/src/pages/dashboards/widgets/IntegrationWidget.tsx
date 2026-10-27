import { keepPreviousData, useQuery } from '@tanstack/react-query';
import type { EChartsOption } from 'echarts';
import { Chart } from '../../../components/charts/Chart';
import type { IntegrationWidgetDefinition } from '../../../api/integrations/evaluate';
import { queryIntegrationPanel } from '../../../api/integrations/panels';
import { aggregate } from '../../../api/integrations/evaluate';
import { palette, seriesPalette } from '../../../theme/chartTheme';
import { changeRows } from '../../../api/integrations/change';
import { useDashboardContext } from './context';
import { WidgetState } from './MetricWidgets';
import { searchEvents } from '../../../api/events/client';
import { useNavigate } from 'react-router-dom';
import { Markdown } from './markdown';
import { eventMarkdown } from '../../../lib/format';
import { searchKubernetes } from '../../../api/kubernetes/client';
import { infrastructureList, type InfrastructureRequest } from '../../../api/integrations/infrastructure';
import { COLUMNS } from '../../kubernetes/columns';
import { byteAxisInterval, formatMetricValue, formatUnitValue, presentationUnit } from '../../../api/integrations/units';

/** Renders the actual upstream requests; never substitutes a different metric or formula. */
export function IntegrationWidget({ def }: { def: IntegrationWidgetDefinition }) {
  if (def.type === 'list_stream' && def.requests?.[0]?.query?.data_source === 'infrastructure') return <InfrastructureListWidget def={def} />;
  return def.type === 'list_stream' || def.requests?.some((r) => r.queries?.some((q) => q.data_source === 'events')) ? <IntegrationEvents def={def} /> : <IntegrationMetric def={def} />;
}

/** Kubernetes resources, as the Kubernetes Explorer lists them; a row opens it in the Explorer. */
function InfrastructureListWidget({ def }: { def: IntegrationWidgetDefinition }) {
  const { resolve } = useDashboardContext();
  const navigate = useNavigate();
  // `avg:scope{$cluster}` → `avg:scope{kube_cluster_name:aks-dev}`, or `{*}` when it matches everything.
  const resolveVar = (name: string) => /\{(.*)\}/.exec(resolve(`avg:scope{$${name}}`))?.[1].replace(/^\*$/, '') ?? '';
  const list = infrastructureList(def.requests[0] as unknown as InfrastructureRequest, resolveVar);
  const result = useQuery({
    queryKey: ['kubernetes', 'integration-list', list],
    queryFn: async () => {
      const pages = await Promise.all(list.queries.map((query) => searchKubernetes({ kind: list.kind, query, limit: LIST_LIMIT, sort: list.sort?.column, desc: list.sort?.desc })));
      const rows = new Map(pages.flatMap((p) => p.items).map((r) => [r.uid, r]));
      return [...rows.values()].slice(0, LIST_LIMIT);
    },
    placeholderData: keepPreviousData,
  });
  const all = COLUMNS[list.kind];
  // Without columns, the kind's own ones minus the cluster and namespace a dashboard already filters by.
  const cols = list.columns ? list.columns.flatMap((key) => all.filter((c) => c.key === key)) : all.filter((c) => c.key !== 'cluster' && c.key !== 'namespace');
  const explorer = (params: Record<string, string>) => navigate(`/orchestration/explorer?${new URLSearchParams({ resource: list.kind, ...params })}`);
  const ctx = { filter: (key: string, value: string) => explorer({ q: `${key}:${value}` }) };
  return (
    <WidgetState loading={result.isPending} error={result.error} empty={!result.data?.length}>
      <div className="wt">
        <div className="wt-scroll">
          <table className="ox-table">
            <thead>
              <tr>
                {cols.map((c) => (
                  <th key={c.key} className={c.num ? 'num' : undefined}>
                    {c.label}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {result.data?.map((r) => (
                <tr key={r.uid} className="wt-link" onClick={() => explorer({ q: list.queries[0] ?? '', panel: r.uid })}>
                  {cols.map((c) => (
                    <td key={c.key} className={c.num ? 'num' : undefined}>
                      {c.render(r, ctx)}
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

/** Rows an infrastructure list shows (Datadog shows the first page too). */
const LIST_LIMIT = 50;

function IntegrationEvents({ def }: { def: IntegrationWidgetDefinition }) {
  const { range, resolveSearch } = useDashboardContext();
  const eventQuery = def.requests?.flatMap((r) => r.queries ?? []).find((q) => q.data_source === 'events');
  const search = (eventQuery as unknown as { search?: { query?: string } })?.search?.query;
  const query = resolveSearch((search ?? def.requests?.[0]?.query?.query_string ?? '').replace(/\$[a-zA-Z_]\w*/g, ''));
  const result = useQuery({ queryKey: ['events', 'integration-panel', query, range], queryFn: () => searchEvents({ range, query, limit: 25, offset: 0 }) });
  return (
    <WidgetState loading={result.isPending} error={result.error} empty={!result.data?.items.length}>
      {def.type === 'timeseries' ? (
        <Chart
          height="100%"
          option={{
            color: palette(),
            grid: { left: 48, right: 16, top: 16, bottom: 42 },
            tooltip: { trigger: 'axis', confine: true },
            legend: { bottom: 0 },
            xAxis: { type: 'time' },
            yAxis: { type: 'value', minInterval: 1 },
            series: [...new Set(result.data?.histogram.flatMap((b) => Object.keys(b.counts)) ?? [])].map((status) => ({
              name: status,
              type: 'bar',
              stack: 'events',
              data: result.data?.histogram.map((b) => [b.t, b.counts[status] ?? 0]),
            })),
          }}
        />
      ) : (
        <div className="wt">
          <div className="wt-scroll">
            <table>
              <tbody>
                {result.data?.items.map((event) => (
                  <tr key={event.id}>
                    <td>{new Date(event.timestamp).toLocaleTimeString()}</td>
                    <td>{event.alertType}</td>
                    <td>
                      <strong>{event.title}</strong>
                      <div className="event-text">
                        <Markdown source={eventMarkdown(event.text)} />
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}
    </WidgetState>
  );
}

/** A series' color: its request's palette, in order within the series sharing that palette. */
function seriesColors(series: { palette?: string }[]): (s: { palette?: string }, index: number) => string {
  const positions = new Map<number, number>();
  const seen = new Map<string, number>();
  series.forEach((s, i) => {
    const key = s.palette ?? '';
    positions.set(i, seen.get(key) ?? 0);
    seen.set(key, (seen.get(key) ?? 0) + 1);
  });
  return (s, index) => {
    const colors = seriesPalette(s.palette);
    return colors[(positions.get(index) ?? index) % colors.length];
  };
}

function IntegrationMetric({ def }: { def: IntegrationWidgetDefinition }) {
  const { range, resolve, metrics } = useDashboardContext();
  const identity = JSON.stringify(def).replace(/\$[a-zA-Z_]\w*/g, (v) => resolve(`avg:scope{${v}}`));
  const data = useQuery({
    queryKey: ['integration-panel', identity, range, metrics?.version],
    queryFn: () => {
      if (metrics?.error) throw metrics.error;
      return queryIntegrationPanel(def, resolve, range, metrics?.source);
    },
    enabled: !metrics || metrics.ready || !!metrics.error,
    gcTime: 30_000,
    placeholderData: keepPreviousData,
    retry: false,
  });
  const series = data.data ?? [];
  const colorOf = seriesColors(series);
  const values = series.map((s) => ({
    name: s.name,
    unit: s.unit,
    value:
      aggregate(
        s.points.map((p) => p[1]),
        'last',
      ) ?? 0,
  }));
  let content;
  if (def.type === 'check_status') {
    const labels = ['OK', 'Warning', 'Critical', 'Unknown'];
    content = (
      <div className="ms-counts">
        {[0, 1, 2, 3].map((status) => (
          <div key={status} className={`ms-count ms-${['ok', 'warn', 'alert', 'nodata'][status]}`}>
            <span>{labels[status]}</span>
            <strong>{values.filter((v) => v.value === status).length}</strong>
          </div>
        ))}
      </div>
    );
  } else if (def.type === 'query_value') {
    const v = aggregate(
      values.map((v) => v.value),
      'sum',
    );
    const formats = def.requests[0]?.conditional_formats ?? [];
    const match = formats.find((f) => v !== undefined && { '>': v > f.value, '>=': v >= f.value, '<': v < f.value, '<=': v <= f.value, '==': v === f.value }[f.comparator]);
    const color = match?.palette.includes('red') ? 'var(--qv-red)' : match?.palette.includes('green') ? 'var(--qv-green)' : undefined;
    const text = v === undefined ? '—' : formatMetricValue(v, series[0]?.unit, def.precision ?? 2);
    // Long values ("14 nodes") shrink to fit the tile's width.
    const fontSize = `min(62cqh, 30cqw, ${Math.round(150 / (text.length + 0.3 * (def.custom_unit?.length ?? 0)))}cqw)`;
    content = (
      <div className="qv">
        <span className="qv-value" style={{ color, fontSize }}>
          {text}
          {def.custom_unit && <small>{def.custom_unit}</small>}
        </span>
      </div>
    );
  } else if (def.type === 'change') {
    const request = def.requests[0] as (typeof def.requests)[number] & { change_type?: string; increase_good?: boolean; order_by?: string; order_dir?: string };
    const rows = changeRows(series, request);
    const unit = series.find((s) => !s.column?.includes('__shift'))?.unit;
    content = (
      <div className="wt">
        <div className="wt-scroll">
          <table>
            <thead>
              <tr>
                <th>Group</th>
                <th className="num">Value</th>
                <th className="num">Change</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => (
                <tr key={r.group}>
                  <td>{r.group === '*' ? def.title : r.group}</td>
                  <td className="num">{formatMetricValue(r.present, unit, 2)}</td>
                  <td className={`num change ${r.change === 0 || Number.isNaN(r.change) ? '' : r.good ? 'change-good' : 'change-bad'}`}>
                    {Number.isNaN(r.change)
                      ? '—'
                      : `${r.change > 0 ? '▲ ' : r.change < 0 ? '▼ ' : ''}${request.change_type === 'relative' ? `${Math.abs(r.change).toLocaleString('en-US', { maximumFractionDigits: 1 })}%` : formatMetricValue(Math.abs(r.change), unit, 2)}`}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
    );
  } else if (def.type === 'heatmap') {
    // How the series' values spread over time: series per value bucket per point in time.
    const times = [...new Set(series.flatMap((s) => s.points.map(([t]) => t)))].sort((a, b) => a - b);
    const all = series.flatMap((s) => s.points.map(([, v]) => v * presentationUnit(s.unit).scale));
    const [lo, hi] = [Math.min(...all), Math.max(...all)];
    const buckets = 10;
    const width = hi > lo ? (hi - lo) / buckets : 1;
    const counts = new Map<string, number>();
    for (const s of series)
      for (const [t, v] of s.points) {
        const b = Math.min(buckets - 1, Math.floor((v * presentationUnit(s.unit).scale - lo) / width));
        counts.set(`${times.indexOf(t)},${b}`, (counts.get(`${times.indexOf(t)},${b}`) ?? 0) + 1);
      }
    const unit = presentationUnit(series[0]?.unit).unit;
    const option: EChartsOption = {
      grid: { left: 8, right: 16, top: 12, bottom: 26, containLabel: true },
      tooltip: { confine: true },
      xAxis: { type: 'category', data: times.map((t) => new Date(t).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })) },
      yAxis: { type: 'category', data: Array.from({ length: buckets }, (_, b) => formatUnitValue(lo + b * width, unit, 2)) },
      visualMap: { show: false, min: 0, max: Math.max(1, ...counts.values()), inRange: { color: seriesPalette('blue').reverse() } },
      series: [{ type: 'heatmap', data: [...counts].map(([k, n]) => [...k.split(',').map(Number), n]) }],
    };
    content = <Chart option={option} height="100%" />;
  } else if (def.type === 'query_table') {
    const columns = [...new Set(series.map((s) => s.column ?? s.name))];
    const columnUnits = new Map(series.map((s) => [s.column ?? s.name, s.unit]));
    const rows = new Map<string, Map<string, number>>();
    for (const s of series) {
      const group = s.group ?? s.name;
      const row = rows.get(group) ?? new Map<string, number>();
      const value = s.points.at(-1)?.[1];
      if (value !== undefined) row.set(s.column ?? s.name, value);
      rows.set(group, row);
    }
    content = (
      <div className="wt">
        <div className="wt-scroll">
          <table>
            <thead>
              <tr>
                <th>Group</th>
                {columns.map((column) => (
                  <th className="num" key={column}>
                    {column}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {[...rows]
                .sort((a, b) => (b[1].get(columns[0]) ?? -Infinity) - (a[1].get(columns[0]) ?? -Infinity))
                .map(([group, row]) => (
                  <tr key={group}>
                    <td>{group}</td>
                    {columns.map((column) => (
                      <td className="num" key={column}>
                        {row.has(column) ? formatMetricValue(row.get(column)!, columnUnits.get(column), 2) : '—'}
                      </td>
                    ))}
                  </tr>
                ))}
            </tbody>
          </table>
        </div>
      </div>
    );
  } else if (def.type === 'toplist') {
    content = (
      <div className="wt">
        <div className="wt-scroll">
          <table>
            <thead>
              <tr>
                <th>Series</th>
                <th className="num">Value</th>
              </tr>
            </thead>
            <tbody>
              {values
                .sort((a, b) => b.value - a.value)
                .map((v, i) => (
                  <tr key={i}>
                    <td>{v.name}</td>
                    <td className="num">{formatMetricValue(v.value, v.unit, 2)}</td>
                  </tr>
                ))}
            </tbody>
          </table>
        </div>
      </div>
    );
  } else {
    const axisUnits = [...new Set(series.map((s) => presentationUnit(s.unit).unit))];
    /** Largest value drawn on an axis: stacked areas add up. */
    const axisMax = (unit: string) => {
      const totals = new Map<number, number>();
      let max = 0;
      for (const s of series.filter((s) => presentationUnit(s.unit).unit === unit)) {
        for (const [t, value] of s.points) {
          const v = value * presentationUnit(s.unit).scale;
          max = Math.max(max, v);
          if (s.display === 'area') totals.set(t, (totals.get(t) ?? 0) + v);
        }
      }
      return Math.max(max, ...totals.values());
    };
    const option: EChartsOption = ['sunburst', 'treemap'].includes(def.type)
      ? {
          tooltip: { trigger: 'item', confine: true, valueFormatter: (value) => formatMetricValue(Number(value), series[0]?.unit) },
          series: [
            { type: def.type as 'sunburst' | 'treemap', data: values, label: { show: true }, ...(def.type === 'treemap' ? { roam: false, nodeClick: false, breadcrumb: { show: false } } : {}) },
          ],
        }
      : {
          color: palette(),
          grid: { left: 8, right: 16, top: 12, bottom: def.show_legend ? 50 : 26, containLabel: true },
          tooltip: { trigger: 'axis', confine: true },
          legend: def.show_legend ? { bottom: 0, type: 'scroll' } : undefined,
          xAxis: { type: 'time', axisLabel: { hideOverlap: true } },
          yAxis: axisUnits.map((unit, index) => {
            // Binary units tick in round KiB/MiB/GiB, not in decimal steps of bytes.
            const interval = unit.split('/')[0] === 'By' && def.yaxis?.scale !== 'log' ? byteAxisInterval(axisMax(unit)) : undefined;
            return {
              position: index === 0 ? 'left' : 'right',
              offset: Math.max(0, index - 1) * 65,
              type: def.yaxis?.scale === 'log' ? 'log' : 'value',
              // Axes start at zero unless the panel says otherwise.
              scale: def.yaxis?.include_zero === false,
              min: def.yaxis?.min && def.yaxis.min !== 'auto' ? Number(def.yaxis.min) : undefined,
              max: def.yaxis?.max && def.yaxis.max !== 'auto' ? Number(def.yaxis.max) : interval ? ({ max }: { max: number }) => Math.ceil(max / interval) * interval : undefined,
              interval,
              axisLabel: { formatter: (value: number) => formatUnitValue(value, unit, 2) },
            };
          }) as EChartsOption['yAxis'],
          series: series.map((s, i) => ({
            name: s.name,
            color: colorOf(s, i),
            data: s.points.map(([t, value]) => [t, value * presentationUnit(s.unit).scale]),
            yAxisIndex: axisUnits.indexOf(presentationUnit(s.unit).unit),
            tooltip: { valueFormatter: (value: unknown) => formatUnitValue(Number(value), presentationUnit(s.unit).unit) },
            type: s.display === 'bars' ? 'bar' : 'line',
            showSymbol: false,
            lineStyle: { type: s.dashed ? 'dashed' : 'solid' },
            ...(s.display === 'area' ? { areaStyle: { opacity: 0.4 }, stack: 'total' } : {}),
          })),
        };
    content = <Chart option={option} height="100%" />;
  }
  const supported = ['timeseries', 'query_value', 'query_table', 'toplist', 'sunburst', 'treemap', 'check_status', 'change', 'heatmap'].includes(def.type);
  return (
    <WidgetState loading={data.isPending} error={!supported ? new Error(`The ${def.type} panel requires a data source that is not supported yet.`) : data.error} empty={!series.length}>
      {content}
    </WidgetState>
  );
}
