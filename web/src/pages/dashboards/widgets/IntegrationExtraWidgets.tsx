// integrations-core widgets beyond charts and tables, drawn as Datadog draws them: the geomap (a world map
// coloured by value per country), the bar chart, the distribution (a histogram of the groups' values), the
// host map, free text, a log pattern list and the sunburst (pie).
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { useMemo, useState } from 'react';
import * as echarts from 'echarts';
import { Chart } from '../../../components/charts/Chart';
import type { IntegrationWidgetDefinition } from '../../../api/integrations/evaluate';
import { queryIntegrationPanel } from '../../../api/integrations/panels';
import { formatMetricValue } from '../../../api/integrations/units';
import { countryValues, distributionBuckets, groupLabel, hostmapColor, sunburstTree, type SunburstNode } from '../../../api/integrations/widgetData';
import { loadWorldMap } from '../../../api/integrations/worldMap';
import { logPatterns } from '../../../api/client';
import { palette, token } from '../../../theme/chartTheme';
import { currentTheme } from '../../../theme/mode';
import { useDashboardContext } from './context';
import { WidgetState } from './MetricWidgets';

type Value = { name: string; unit?: string; value: number };

/** Countries coloured by value (Datadog's `hostmap_blues` scale); hovering names the country and its value. */
export function IntegrationGeomap({ values }: { values: Value[] }) {
  const map = useQuery({
    queryKey: ['world-map'],
    queryFn: async () => {
      const world = await loadWorldMap();
      echarts.registerMap('world', world as Parameters<typeof echarts.registerMap>[1]);
      return true;
    },
    staleTime: Infinity,
  });
  const countries = countryValues(values);
  const unit = values[0]?.unit;
  const max = Math.max(1, ...countries.map((c) => c.value));
  return (
    <WidgetState loading={map.isPending} error={map.error} empty={false}>
      {map.data && (
        <Chart
          height="100%"
          option={{
            tooltip: {
              trigger: 'item',
              formatter: (p) =>
                `${(p as { name: string }).name}: ${(p as { value?: number }).value === undefined || Number.isNaN((p as { value: number }).value) ? '—' : formatMetricValue((p as { value: number }).value, unit)}`,
            },
            visualMap: { min: 0, max, show: false, inRange: { color: currentTheme() === 'dark' ? ['#1c3350', '#2a5fb0', '#5b9cf0', '#cfe3fb'] : ['#d6e6f5', '#6baed6', '#2171b5', '#08306b'] } },
            series: [
              {
                type: 'map',
                map: 'world',
                nameProperty: 'iso',
                roam: false,
                // Without a box ECharts fits the world in the widget, centred, its proportions kept.
                itemStyle: { areaColor: token('--bg-muted'), borderColor: token('--bg'), borderWidth: 0.5 },
                emphasis: { label: { show: false }, itemStyle: { areaColor: token('--primary') } },
                select: { disabled: true },
                data: countries.map((c) => ({ name: c.iso, value: c.value })),
              },
            ],
          }}
        />
      )}
    </WidgetState>
  );
}

/** One bar per group, tallest first. */
export function IntegrationBarChart({ values, colorOf }: { values: Value[]; colorOf: (index: number) => string }) {
  const sorted = [...values].sort((a, b) => b.value - a.value).slice(0, 50);
  const unit = sorted[0]?.unit;
  return (
    <Chart
      height="100%"
      option={{
        grid: { left: 52, right: 12, top: 12, bottom: 48 },
        tooltip: { trigger: 'axis', confine: true, valueFormatter: (v) => formatMetricValue(Number(v), unit) },
        xAxis: {
          type: 'category',
          data: sorted.map((v) => groupLabel(v.name)),
          axisLabel: { interval: 0, rotate: sorted.length > 6 || sorted.some((v) => groupLabel(v.name).length > 12) ? 30 : 0, width: 90, overflow: 'truncate' },
        },
        yAxis: { type: 'value', axisLabel: { formatter: (v: number) => formatMetricValue(v, unit, 1) } },
        series: [{ type: 'bar', data: sorted.map((v, i) => ({ value: v.value, itemStyle: { color: colorOf(i) } })), barMaxWidth: 48 }],
      }}
    />
  );
}

/** How many groups fall in each range of values. */
export function IntegrationDistribution({ values }: { values: Value[] }) {
  const buckets = distributionBuckets(values.map((v) => v.value));
  const unit = values[0]?.unit;
  return (
    <Chart
      height="100%"
      option={{
        grid: { left: 40, right: 12, top: 12, bottom: 28 },
        tooltip: { trigger: 'axis', confine: true },
        xAxis: { type: 'category', data: buckets.map((b) => `${formatMetricValue(b.from, unit, 1)} – ${formatMetricValue(b.to, unit, 1)}`), axisLabel: { hideOverlap: true } },
        yAxis: { type: 'value', minInterval: 1 },
        series: [{ type: 'bar', data: buckets.map((b) => b.count), barCategoryGap: '4%', color: token('--chart-1') }],
      }}
    />
  );
}

/** Text in the widget's size, colour and alignment. */
export function IntegrationFreeText({ def }: { def: IntegrationWidgetDefinition }) {
  const size = Number(def.font_size) || 0;
  return (
    <div
      className="iw-free-text"
      style={{
        color: typeof def.color === 'string' ? def.color : undefined,
        textAlign: def.text_align === 'center' || def.text_align === 'right' ? def.text_align : 'left',
        fontSize: size ? `${Math.min(size, 72)}px` : 'min(40cqh, 8cqw)',
      }}
    >
      {typeof def.text === 'string' ? def.text : ''}
    </div>
  );
}

/** Hosts as tiles, coloured by the fill query (green lowest, orange highest); hovering names the host and value. */
export function IntegrationHostmap({ def }: { def: IntegrationWidgetDefinition }) {
  const { range, resolve } = useDashboardContext();
  const requests = def.requests as unknown as { fill?: { q?: string } };
  const fill = requests?.fill?.q;
  const panel = useMemo(() => (fill ? ({ type: 'query_table', requests: [{ q: /\bby\b/.test(fill) ? fill : `${fill} by {host}` }] } as unknown as IntegrationWidgetDefinition) : null), [fill]);
  const data = useQuery({
    queryKey: ['integration-hostmap', fill, range, resolve(fill ?? '')],
    queryFn: () => queryIntegrationPanel(panel!, resolve, range),
    enabled: !!panel,
    placeholderData: keepPreviousData,
  });
  const hosts = (data.data ?? []).map((s) => ({ name: s.name, unit: s.unit, value: s.points.at(-1)?.[1] ?? 0 })).sort((a, b) => a.name.localeCompare(b.name));
  const [min, max] = [Math.min(...hosts.map((h) => h.value)), Math.max(...hosts.map((h) => h.value))];
  return (
    <WidgetState loading={!!panel && data.isPending} error={panel ? data.error : new Error('This host map has no fill query')} empty={!hosts.length}>
      <div className="iw-hostmap">
        {hosts.map((h) => (
          <span key={h.name} className="iw-hostmap-cell" style={{ background: hostmapColor(h.value, min, max) }} title={`${h.name}: ${formatMetricValue(h.value, h.unit)}`} />
        ))}
      </div>
    </WidgetState>
  );
}

/** The log patterns of the widget's query, most frequent first. */
export function IntegrationLogPatterns({ def }: { def: IntegrationWidgetDefinition }) {
  const { range, resolveSearch } = useDashboardContext();
  const query = resolveSearch(def.requests?.[0]?.query?.query_string ?? '');
  const result = useQuery({ queryKey: ['logs', 'patterns', 'integration-panel', query, range], queryFn: () => logPatterns({ range, query }), placeholderData: keepPreviousData });
  const patterns = [...(result.data ?? [])].sort((a, b) => b.count - a.count).slice(0, 50);
  return (
    <WidgetState loading={result.isPending} error={result.error} empty={!patterns.length}>
      <div className="wt">
        <div className="wt-scroll">
          <table className="iw-ev">
            <thead>
              <tr>
                <th className="iw-ev-date">Count</th>
                <th>Pattern</th>
              </tr>
            </thead>
            <tbody>
              {patterns.map((p) => (
                <tr key={p.pattern} title={p.sample}>
                  <td className="iw-ev-date">{p.count.toLocaleString()}</td>
                  <td className="mono">{p.pattern}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
    </WidgetState>
  );
}

/**
 * Datadog's sunburst (pie): a donut with a ring per group-by tag, the total in its centre (the hovered slice's value
 * instead while hovering), no labels on the slices, and a legend of the inner ring beside it, as a table (`table`),
 * a list (`inline`), only when the widget is wide enough (`automatic`) or none (`none`).
 */
export function IntegrationSunburst({ def, values }: { def: IntegrationWidgetDefinition; values: Value[] }) {
  const tree = useMemo(() => sunburstTree(values), [values]);
  const [hovered, setHovered] = useState<{ name: string; value: number } | null>(null);
  const unit = values[0]?.unit;
  const colors = palette();
  const legend = (def.legend as { type?: string } | undefined)?.type ?? 'automatic';
  const showTotal = def.hide_total !== true;
  // Outer rings take their inner slice's colour, lighter the further out they are.
  const color = (nodes: SunburstNode[], base?: string, depth = 0): object[] =>
    nodes.map((n, i) => {
      const c = base ?? colors[i % colors.length];
      return { name: n.name, value: n.value, itemStyle: { color: c, opacity: 1 - depth * 0.18 }, children: color(n.children, c, depth + 1) };
    });
  const center = hovered ?? (showTotal ? { name: 'Total', value: tree.total } : null);
  const option = useMemo(
    () => ({
      tooltip: {
        trigger: 'item' as const,
        confine: true,
        formatter: (p: unknown) =>
          `${(p as { treePathInfo: { name: string }[] }).treePathInfo
            .slice(1)
            .map((t) => t.name)
            .join(' › ')}: ${formatMetricValue((p as { value: number }).value, unit)}`,
      },
      series: [
        {
          type: 'sunburst' as const,
          radius: ['52%', '92%'],
          data: color(tree.nodes),
          sort: undefined,
          label: { show: false },
          itemStyle: { borderColor: token('--bg'), borderWidth: 1 },
          emphasis: { focus: 'ancestor' as const },
          nodeClick: false as const,
        },
      ],
    }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [tree, unit],
  );
  return (
    <WidgetState loading={false} error={null} empty={!tree.nodes.length}>
      <div className={`iw-pie iw-pie-${legend}`}>
        <div className="iw-pie-chart">
          <Chart height="100%" option={option} onHover={(p) => setHovered(p ? { name: p.name, value: Number(p.value) } : null)} />
          {center && (
            <div className="iw-pie-center">
              <span className="iw-pie-center-value">{formatMetricValue(center.value, unit)}</span>
              <span className="iw-pie-center-name">{center.name}</span>
            </div>
          )}
        </div>
        {legend !== 'none' && (
          <table className="iw-pie-legend">
            <tbody>
              {tree.legend.map((row, i) => (
                <tr key={row.name}>
                  <td>
                    <span className="iw-pie-swatch" style={{ background: colors[i % colors.length] }} />
                    {row.name}
                  </td>
                  {legend !== 'inline' && <td className="num">{formatMetricValue(row.value, unit)}</td>}
                  <td className="num muted">{row.percent.toFixed(1)}%</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </WidgetState>
  );
}
