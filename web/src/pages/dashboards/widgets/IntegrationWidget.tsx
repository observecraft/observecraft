import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { useMemo } from 'react';
import type { TimeRange } from '../../../api/types';
import type { EChartsOption, SeriesOption } from 'echarts';
import { Chart } from '../../../components/charts/Chart';
import type { IntegrationWidgetDefinition } from '../../../api/integrations/evaluate';
import { queryIntegrationPanel, type PanelSeries } from '../../../api/integrations/panels';
import { aggregate } from '../../../api/integrations/evaluate';
import { palette, seriesPalette, token } from '../../../theme/chartTheme';
import { overlaySeries, overlayStatusToken } from '../../../lib/overlays';
import { changeRows, type ChangeRow } from '../../../api/integrations/change';
import { useDashboardContext } from './context';
import { WidgetState } from './MetricWidgets';
import { groupTags, IntegrationTable } from './IntegrationTable';
import { fills, formatStyle, matchFormat } from './conditionalFormats';
import { searchEvents } from '../../../api/events/client';
import { useNavigate } from 'react-router-dom';
import { SourceIcon, STATUS_TOKENS } from '../../events/eventVisuals';
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
  // `avg:scope{$cluster}` → `avg:scope{kube_cluster_name:shop-prod}`, or `{*}` when it matches everything.
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
  const { range: dashboardRange, resolveSearch } = useDashboardContext();
  const range = useWidgetRange(def, dashboardRange);
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
            grid: { left: 48, right: 16, top: 16, bottom: 26 },
            tooltip: { trigger: 'axis', confine: true },
            xAxis: { type: 'time' },
            yAxis: { type: 'value', minInterval: 1 },
            series: [...new Set(result.data?.histogram.flatMap((b) => Object.keys(b.counts)) ?? [])].map((status) => ({
              name: status,
              type: 'bar',
              stack: 'events',
              color: token(STATUS_TOKENS[status] ?? '--status-info'),
              data: result.data?.histogram.map((b) => [b.t, b.counts[status] ?? 0]),
            })),
          }}
        />
      ) : (
        <div className="wt">
          <div className="wt-scroll">
            <table className="iw-ev">
              <thead>
                <tr>
                  <th className="iw-ev-source">Source</th>
                  <th>Title</th>
                  <th className="iw-ev-date">Date</th>
                </tr>
              </thead>
              <tbody>
                {result.data?.items.map((event) => (
                  <tr key={event.id} title={event.title}>
                    <td className="iw-ev-source">
                      <span className="status-bar" style={{ background: `var(${STATUS_TOKENS[event.alertType] ?? '--status-info'})` }} />
                      <SourceIcon source={event.source} size={18} />
                    </td>
                    <td>
                      <div className="iw-ev-line">
                        <span className="iw-ev-title">{event.title}</span>
                        {event.tags[0] && <span className="iw-ev-tag">{event.tags[0]}</span>}
                        {event.tags.length > 1 && <span className="iw-ev-tag iw-ev-more">+{event.tags.length - 1}</span>}
                      </div>
                    </td>
                    <td className="iw-ev-date">{formatEventDate(event.timestamp)}</td>
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

const SPAN_SECONDS: Record<string, number> = { m: 60, h: 3600, d: 86400, w: 604800, mo: 2592000 };
const UNIT_SECONDS: Record<string, number> = { minute: 60, hour: 3600, day: 86400, week: 604800, month: 2592000 };

/** A widget's own time frame (`time.live_span` "4h", or `{ type: 'live', unit, value }`) as a label and its seconds. */
export function widgetSpan(def: IntegrationWidgetDefinition): { label: string; seconds: number } | undefined {
  const time = def.time as { live_span?: string; type?: string; unit?: string; value?: number } | undefined;
  const m = /^(\d+)(mo|m|h|d|w)$/.exec(time?.live_span ?? '');
  if (m) return { label: time!.live_span!, seconds: Number(m[1]) * SPAN_SECONDS[m[2]] };
  if (time?.type === 'live' && time.unit && time.value && UNIT_SECONDS[time.unit])
    return { label: `${time.value}${time.unit === 'month' ? 'mo' : time.unit[0]}`, seconds: time.value * UNIT_SECONDS[time.unit] };
  return undefined;
}

/** The range a widget shows: its own time frame up to the dashboard's end, or the dashboard's range. */
export function useWidgetRange(def: IntegrationWidgetDefinition, range: TimeRange): TimeRange {
  const span = widgetSpan(def);
  return useMemo(() => (span ? { from: range.to - span.seconds * 1000, to: range.to } : range), [span?.seconds, range]);
}

/** "Oct 30, 12:53 pm". */
const formatEventDate = (t: number) => {
  const d = new Date(t);
  return `${d.toLocaleDateString('en-US', { month: 'short', day: 'numeric' })}, ${d.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' }).toLowerCase()}`;
};

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
  const { range: dashboardRange, resolve, metrics: shared, overlays } = useDashboardContext();
  // A widget with its own time frame reads it directly, not the dashboard's shared stream.
  const range = useWidgetRange(def, dashboardRange);
  const metrics = range === dashboardRange ? shared : undefined;
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
  // A query value's timeseries background: the same requests over time, summed across groups.
  const background = def.type === 'query_value' && !!def.timeseries_background;
  const trend = useQuery({
    queryKey: ['integration-panel', 'background', identity, range, metrics?.version],
    queryFn: () => queryIntegrationPanel({ ...def, type: 'timeseries' }, resolve, range, metrics?.source),
    enabled: background && (!metrics || metrics.ready),
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
  if (def.type === 'check_status' && def.grouping === 'cluster') {
    // One tile: how many checks are in the worst status there is, in that status' color.
    const counts = [0, 1, 2, 3].map((status) => values.filter((v) => v.value === status).length);
    const worst = [2, 1, 3, 0].find((status) => counts[status] > 0);
    const palette = worst === undefined ? undefined : ['white_on_green', 'white_on_yellow', 'white_on_red', 'white_on_gray'][worst];
    content = (
      <div className={`qv${palette ? ' qv-fill' : ''}`} style={palette ? formatStyle(palette) : undefined}>
        <span className="qv-value">{worst === undefined ? '—' : counts[worst].toLocaleString()}</span>
      </div>
    );
  } else if (def.type === 'check_status') {
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
    const format = matchFormat(v, def.requests[0]?.conditional_formats);
    const fill = format && fills(format.palette);
    const text = v === undefined ? '—' : formatMetricValue(v, series[0]?.unit, def.precision ?? 2);
    // Long values ("14 nodes") shrink to fit the tile's width.
    const fontSize = `min(62cqh, 30cqw, ${Math.round(150 / (text.length + 0.3 * (def.custom_unit?.length ?? 0)))}cqw)`;
    const points = new Map<number, number>();
    for (const s of trend.data ?? []) for (const [t, value] of s.points) points.set(t, (points.get(t) ?? 0) + value);
    content = (
      <div className={`qv${fill ? ' qv-fill' : ''}${background ? ' qv-trend' : ''}`} style={format ? formatStyle(format.palette) : undefined}>
        {background && points.size > 1 && (
          <div className="qv-background">
            <Chart
              height="100%"
              option={{
                animation: false,
                grid: { left: 0, right: 0, top: 4, bottom: 0 },
                xAxis: { type: 'time', show: false },
                yAxis: { type: 'value', show: false, scale: (def.timeseries_background as { yaxis?: { include_zero?: boolean } }).yaxis?.include_zero !== true },
                series: [
                  {
                    type: def.timeseries_background && (def.timeseries_background as { type?: string }).type === 'bars' ? 'bar' : 'line',
                    data: [...points].sort((a, b) => a[0] - b[0]),
                    showSymbol: false,
                    silent: true,
                    color: fill ? 'rgba(255, 255, 255, 0.45)' : token('--border-strong'),
                    lineStyle: { width: 1 },
                    areaStyle: { opacity: fill ? 0.25 : 0.35 },
                  },
                ],
              }}
            />
          </div>
        )}
        <span className="qv-value" style={{ fontSize }}>
          {text}
          {def.custom_unit && <small>{def.custom_unit}</small>}
        </span>
      </div>
    );
  } else if (def.type === 'change') {
    const request = def.requests[0] as (typeof def.requests)[number] & { change_type?: string; increase_good?: boolean; order_by?: string; order_dir?: string };
    content = <IntegrationChange rows={changeRows(series, request)} unit={series.find((s) => !s.column?.includes('__shift'))?.unit} relative={request.change_type === 'relative'} />;
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
  } else if (def.type === 'treemap') {
    content = <IntegrationTreemap series={series} />;
  } else if (def.type === 'query_table') {
    content = <IntegrationTable def={def} series={series} />;
  } else if (def.type === 'toplist') {
    content = <IntegrationToplist def={def} series={series} />;
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
    const option: EChartsOption =
      def.type === 'sunburst'
        ? {
            tooltip: { trigger: 'item', confine: true, valueFormatter: (value) => formatMetricValue(Number(value), series[0]?.unit) },
            series: [{ type: 'sunburst', data: values, label: { show: true } }],
          }
        : {
            color: palette(),
            grid: { left: 8, right: 16, top: 12, bottom: def.show_legend ? 50 : 26, containLabel: true },
            tooltip: { trigger: 'axis', confine: true },
            // Series read by their tag values ("prod, checkout-api"), as upstream legends show them.
            legend: def.show_legend
              ? { bottom: 0, type: 'scroll', formatter: (name: string) => (name.includes(':') ? [...groupTags(name.replace(/^.* · /, '')).values()].join(', ') : name) }
              : undefined,
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
            series: [
              ...(series.map((s, i) => ({
                name: s.name,
                color: colorOf(s, i),
                data: s.points.map(([t, value]) => [t, value * presentationUnit(s.unit).scale]),
                yAxisIndex: axisUnits.indexOf(presentationUnit(s.unit).unit),
                tooltip: { valueFormatter: (value: unknown) => formatUnitValue(Number(value), presentationUnit(s.unit).unit) },
                type: s.display === 'bars' ? 'bar' : 'line',
                showSymbol: false,
                lineStyle: { type: s.dashed ? 'dashed' : 'solid' },
                ...(s.display === 'area' ? { areaStyle: { opacity: 0.4 }, stack: 'total' } : {}),
              })) as SeriesOption[]),
              ...(overlays?.length ? [overlaySeries(overlays, (status) => token(overlayStatusToken(status)))] : []),
            ],
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

/**
 * A treemap nested by the group-by tags, as the upstream widget draws it: a "Total" header, a labelled
 * box per value of the first tag (in its own color), and the series inside it. Only positive values have an area.
 */
function IntegrationTreemap({ series }: { series: PanelSeries[] }) {
  const unit = series[0]?.unit;
  const format = (value: number) => formatMetricValue(value, unit, 2);
  type Node = { name: string; value: number; children?: Node[] };
  const root: Node = { name: 'Total', value: 0, children: [] };
  for (const s of series) {
    const value = s.points.at(-1)?.[1];
    if (value === undefined || !(value > 0)) continue;
    const tags = [...groupTags(s.group ?? s.name)];
    // Nest by every tag but the last; the last one names the box.
    let parent = root;
    parent.value += value;
    for (const [key, tag] of tags.slice(0, -1)) {
      const name = `${key}: ${tag}`;
      let child = parent.children!.find((c) => c.name === name);
      if (!child) parent.children!.push((child = { name, value: 0, children: [] }));
      child.value += value;
      parent = child;
    }
    parent.children!.push({ name: tags.at(-1)?.[1] ?? s.name, value });
  }
  const option: EChartsOption = {
    tooltip: { confine: true, formatter: (p) => `${(p as { name: string }).name}<br/><b>${format(Number((p as { value: unknown }).value))}</b>` },
    series: [
      {
        type: 'treemap',
        data: root.children,
        roam: false,
        nodeClick: false,
        breadcrumb: { show: false },
        width: '100%',
        height: '100%',
        top: 0,
        left: 0,
        label: {
          show: true,
          formatter: (p) => (p.name ? `${p.name}\n{v|${format(Number(p.value))}}` : ''),
          rich: { v: { fontWeight: 600, fontSize: 13, lineHeight: 18 } },
          fontSize: 12,
          overflow: 'truncate',
          color: token('--text'),
          textBorderWidth: 0,
        },
        // The (virtual) root hands each first-tag box its own color; the series inside it share it.
        levels: [
          {
            color: palette(),
            colorMappingBy: 'index',
            itemStyle: { borderWidth: 0, gapWidth: 2 },
          },
          {
            // The header is the box's border, in a shade of its color.
            itemStyle: { borderWidth: 1, gapWidth: 1, borderColorSaturation: 0.7 },
            upperLabel: {
              show: true,
              height: 20,
              color: token('--text'),
              textBorderWidth: 0,
              formatter: (p) => (p.name ? `${p.name}  {v|${format(Number(p.value))}}` : ''),
              rich: { v: { color: token('--text'), fontWeight: 600 } },
            },
          },
          { itemStyle: { borderWidth: 1, gapWidth: 1, borderColorSaturation: 0.6 } },
        ],
      },
    ],
  };
  return (
    <div className="iw-tm">
      <div className="iw-tm-total">
        <span>Total</span>
        <strong>{format(root.value)}</strong>
      </div>
      <Chart option={option} height="100%" />
    </div>
  );
}

/**
 * A toplist as bars, largest first. The stacked display groups the series by their first tag: one bar
 * per value of it, split by the other tags (an image's bar split by its tags).
 */
function IntegrationToplist({ def, series }: { def: IntegrationWidgetDefinition; series: PanelSeries[] }) {
  const stacked = (def.style as { display?: { type?: string } } | undefined)?.display?.type === 'stacked';
  const unit = series[0]?.unit;
  const colors = palette();
  type Bar = { name: string; value: number; parts: { name: string; value: number }[] };
  const bars = new Map<string, Bar>();
  for (const s of series) {
    const value = s.points.at(-1)?.[1];
    if (value === undefined || !Number.isFinite(value)) continue;
    const tags = [...groupTags(s.group ?? s.name).values()];
    const name = stacked && tags.length > 1 ? tags[0] : tags.length ? tags.join(', ') : s.name;
    const bar = bars.get(name) ?? { name, value: 0, parts: [] };
    bar.value += value;
    bar.parts.push({ name: tags.slice(stacked ? 1 : 0).join(', ') || name, value });
    bars.set(name, bar);
  }
  const limit = (def.requests[0] as { sort?: { count?: number } } | undefined)?.sort?.count ?? 100;
  // A bar in a conditional format's color, in its pale shade so the label stays readable.
  const barColor = (value: number) => {
    const format = matchFormat(value, def.requests[0]?.conditional_formats);
    const hue = format && /_on_(?:light_)?(red|yellow|green|gray)$/.exec(format.palette)?.[1];
    return hue ? `var(--cf-light-${hue})` : undefined;
  };
  const rows = [...bars.values()].sort((a, b) => b.value - a.value).slice(0, limit);
  const max = Math.max(0, ...rows.map((r) => r.value));
  return (
    <ol className="tl iw-tl">
      {rows.map((r) => (
        <li key={r.name} title={`${r.name}: ${formatMetricValue(r.value, unit, 2)}`}>
          <span className="tl-value">{formatMetricValue(r.value, unit, 2)}</span>
          <span className="tl-bar-wrap">
            <span className="iw-tl-bar" style={{ width: `${max > 0 ? (Math.max(0, r.value) / max) * 100 : 0}%` }}>
              {r.parts
                .sort((a, b) => b.value - a.value)
                .map((p, i) => (
                  <span
                    key={p.name}
                    title={`${p.name}: ${formatMetricValue(p.value, unit, 2)}`}
                    style={{ flexGrow: Math.max(0, p.value), background: stacked ? colors[i % colors.length] : barColor(r.value) }}
                  />
                ))}
            </span>
            <span className="tl-label">{r.name}</span>
          </span>
        </li>
      ))}
    </ol>
  );
}

/** A round axis end above `max` and its ticks: 0, the middle and the end. */
function axisTicks(max: number): number[] {
  if (!(max > 0)) return [0, 1];
  const magnitude = 10 ** Math.floor(Math.log10(max));
  const end = [1, 2, 2.5, 5, 10].map((n) => n * magnitude).find((n) => n >= max) ?? 10 * magnitude;
  return [0, end / 2, end];
}

/**
 * A change panel as the upstream widget draws it: per group, a bar from its earlier value to its present
 * one with an arrow at the present end, green when it moved the good way, and the change beside it.
 */
function IntegrationChange({ rows, unit, relative }: { rows: ChangeRow[]; unit?: string; relative: boolean }) {
  const ticks = axisTicks(Math.max(0, ...rows.flatMap((r) => [r.present, r.past])));
  const end = ticks.at(-1)!;
  const at = (value: number) => `${(Math.max(0, value) / end) * 100}%`;
  const delta = (r: ChangeRow) =>
    Number.isNaN(r.change)
      ? '—'
      : `${r.change > 0 ? '+' : r.change < 0 ? '-' : ''}${relative ? `${Math.abs(r.change).toLocaleString('en-US', { maximumFractionDigits: 1 })}%` : formatMetricValue(Math.abs(r.change), unit, 1)}`;
  return (
    <div className="iw-chg">
      <div className="iw-chg-row iw-chg-axis">
        <span />
        <span className="iw-chg-track">
          {ticks.map((t) => (
            <span key={t} style={{ left: at(t) }}>
              {formatMetricValue(t, unit, 1)}
            </span>
          ))}
        </span>
        <span />
      </div>
      <div className="iw-chg-rows">
        {rows.map((r) => {
          const tags = [...groupTags(r.group).values()].join(', ') || r.group;
          const [from, to] = [Math.min(r.past, r.present), Math.max(r.past, r.present)];
          return (
            <div
              key={r.group}
              className={`iw-chg-row ${r.change === 0 ? '' : r.good ? 'iw-chg-good' : 'iw-chg-bad'}`}
              title={`${tags}: ${formatMetricValue(r.past, unit, 2)} → ${formatMetricValue(r.present, unit, 2)}`}
            >
              <span className="iw-chg-label">{tags}</span>
              <span className="iw-chg-track">
                {ticks.map((t) => (
                  <span key={t} className="iw-chg-grid" style={{ left: at(t) }} />
                ))}
                <span className={`iw-chg-bar ${r.present >= r.past ? 'iw-chg-up' : 'iw-chg-down'}`} style={{ left: at(from), width: `calc(${at(to)} - ${at(from)})` }} />
              </span>
              <span className="iw-chg-delta">{delta(r)}</span>
            </div>
          );
        })}
      </div>
    </div>
  );
}
