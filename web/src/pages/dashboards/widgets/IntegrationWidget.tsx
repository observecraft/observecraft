import { keepPreviousData, useQuery } from '@tanstack/react-query';
import type { EChartsOption } from 'echarts';
import { Chart } from '../../../components/charts/Chart';
import type { IntegrationWidgetDefinition } from '../../../api/integrations/evaluate';
import { queryIntegrationPanel } from '../../../api/integrations/panels';
import { aggregate } from '../../../api/integrations/evaluate';
import { palette } from '../../../theme/chartTheme';
import { useDashboardContext } from './context';
import { WidgetState } from './MetricWidgets';
import { searchEvents } from '../../../api/events/client';
import { formatMetricValue, formatUnitValue, presentationUnit } from '../../../api/integrations/units';

/** Renders the actual upstream requests; never substitutes a different metric or formula. */
export function IntegrationWidget({ def }: { def: IntegrationWidgetDefinition }) {
  return def.type === 'list_stream' || def.requests?.some((r) => r.queries?.some((q) => q.data_source === 'events')) ? <IntegrationEvents def={def} /> : <IntegrationMetric def={def} />;
}

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
                      <div>{event.text}</div>
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
    content = (
      <div className="qv">
        <span className="qv-value" style={{ color }}>
          {v === undefined ? '—' : formatMetricValue(v, series[0]?.unit, def.precision ?? 2)}
          {def.custom_unit && <small>{def.custom_unit}</small>}
        </span>
      </div>
    );
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
    const option: EChartsOption = ['sunburst', 'treemap'].includes(def.type)
      ? {
          tooltip: { trigger: 'item', confine: true, valueFormatter: (value) => formatMetricValue(Number(value), series[0]?.unit) },
          series: [
            { type: def.type as 'sunburst' | 'treemap', data: values, label: { show: true }, ...(def.type === 'treemap' ? { roam: false, nodeClick: false, breadcrumb: { show: false } } : {}) },
          ],
        }
      : {
          color: palette(),
          grid: { left: 85, right: axisUnits.length > 1 ? 85 : 16, top: 12, bottom: def.show_legend ? 50 : 26 },
          tooltip: { trigger: 'axis', confine: true },
          legend: def.show_legend ? { bottom: 0, type: 'scroll' } : undefined,
          xAxis: { type: 'time' },
          yAxis: axisUnits.map((unit, index) => ({
            position: index === 0 ? 'left' : 'right',
            offset: Math.max(0, index - 1) * 65,
            type: def.yaxis?.scale === 'log' ? 'log' : 'value',
            min: def.yaxis?.min && def.yaxis.min !== 'auto' ? Number(def.yaxis.min) : undefined,
            max: def.yaxis?.max && def.yaxis.max !== 'auto' ? Number(def.yaxis.max) : undefined,
            axisLabel: { formatter: (value: number) => formatUnitValue(value, unit, 2) },
          })) as EChartsOption['yAxis'],
          series: series.map((s) => ({
            name: s.name,
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
  const supported = ['timeseries', 'query_value', 'query_table', 'toplist', 'sunburst', 'treemap', 'check_status'].includes(def.type);
  return (
    <WidgetState loading={data.isPending} error={!supported ? new Error(`The ${def.type} panel requires a data source that is not supported yet.`) : data.error} empty={!series.length}>
      {content}
    </WidgetState>
  );
}
