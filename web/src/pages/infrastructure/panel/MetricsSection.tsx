import { useMemo, useState } from 'react';
import type { EChartsOption } from 'echarts';
import { useNavigate } from 'react-router-dom';
import { ExternalLink, Gauge, Puzzle, Search, TriangleAlert } from 'lucide-react';
import type { HostDetail, HostMetrics } from '../../../api/infrastructure/types';
import type { MetricSeries } from '../../../api/types';
import { Chart } from '../../../components/charts/Chart';
import { useTime } from '../../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { Select } from '../../../components/ui/Dropdown';
import { snapRange } from '../../../lib/time';
import { token } from '../../../theme/chartTheme';
import { HpSection } from './HpSection';
import { useHostMetrics } from '../queries';

function axisTime(v: number): string {
  const d = new Date(v);
  if (d.getHours() === 0 && d.getMinutes() === 0) return d.toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
}

function timeseries(series: MetricSeries[], opts: { stack?: boolean; area?: boolean; colors: string[]; yName?: string; max?: number }): EChartsOption {
  return {
    color: opts.colors,
    grid: { left: 40, right: 8, top: 14, bottom: 52 },
    tooltip: { trigger: 'axis' },
    legend: { type: 'scroll', bottom: 0, left: 0, right: 0, textStyle: { fontSize: 11 }, formatter: (n: string) => (n.length > 24 ? `${n.slice(0, 24)}…` : n) },
    // Narrow charts: few ticks, and never let labels overlap.
    xAxis: { type: 'time', splitNumber: 4, axisLabel: { formatter: (v: number) => axisTime(v), hideOverlap: true } },
    yAxis: { type: 'value', name: opts.yName, nameLocation: 'middle', nameGap: 28, nameTextStyle: { fontSize: 10, color: token('--chart-axis') }, max: opts.max },
    series: series.map((s) => ({
      name: s.name,
      type: 'line',
      stack: opts.stack ? 'total' : undefined,
      areaStyle: opts.area ? { opacity: 0.85 } : undefined,
      lineStyle: { width: opts.area ? 0 : 1.2 },
      data: s.points,
    })),
  };
}

function Charts({ m }: { m: HostMetrics }) {
  // CPU: idle drawn on top so the busy portion reads from the bottom, as in the reference.
  const cpu = useMemo(() => timeseries([...m.cpu.filter((s) => !s.name.includes('idle')), ...m.cpu.filter((s) => s.name.includes('idle'))], { stack: true, area: true, max: 100, yName: 'Percent', colors: ['#7b5cf0', '#3d7fe0', '#c8b6ff', '#f2a93b', '#a9cdf5'] }), [m]);
  const load = useMemo(() => timeseries(m.load, { colors: ['#3d7fe0', '#5b3bd6', '#f2c12e'] }), [m]);
  const mem = useMemo(() => {
    const [usable, total] = m.memory;
    return timeseries([usable, { name: total.name, points: total.points.map(([t, v], i) => [t, Math.max(0, v - (usable.points[i]?.[1] ?? 0))] as [number, number]) }], { stack: true, area: true, yName: 'Gibibytes', colors: ['#a9cdf5', '#3d7fe0'] });
  }, [m]);
  const treemap = useMemo<EChartsOption>(
    () => ({
      tooltip: { formatter: (p: unknown) => { const x = p as { name: string; value: number }; return `${x.name}: ${x.value}%`; } },
      series: [
        {
          type: 'treemap',
          roam: false,
          nodeClick: false,
          breadcrumb: { show: false },
          width: '100%',
          height: '100%',
          itemStyle: { borderColor: token('--bg'), borderWidth: 2, gapWidth: 2 },
          label: { show: true, formatter: (p: unknown) => { const x = p as { name: string; value: number }; return x.value > 10 ? `{n|${x.name}}\n{v|${x.value}}` : ''; }, rich: { n: { fontSize: 22, color: '#fff' }, v: { fontSize: 40, fontWeight: 700, color: '#1c2333' } } },
          data: m.processMemory.map((p, i) => ({ ...p, itemStyle: { color: ['#13c2e8', '#e39ae8', '#ffd60a', '#2ec4a6', '#f07e95', '#c8ccd6'][i % 6] } })),
        },
      ],
    }),
    [m],
  );
  return (
    <div className="hm-grid">
      <div>
        <div className="hm-title">CPU usage (%)</div>
        <Chart option={cpu} height={190} />
      </div>
      <div>
        <div className="hm-title">
          Processes memory usage <span className="muted-text">(% of total)</span>
        </div>
        <Chart option={treemap} height={190} />
      </div>
      <div>
        <div className="hm-title">Load Averages 1-5-15</div>
        <Chart option={load} height={190} />
      </div>
      <div>
        <div className="hm-title">Memory breakdown</div>
        <Chart option={mem} height={190} />
      </div>
    </div>
  );
}

function fmtUptime(sec: number): string {
  const d = Math.floor(sec / 86400);
  const h = Math.floor((sec % 86400) / 3600);
  return d ? `${d}d ${h}h` : `${h}h`;
}

export function MetricsSection({ host }: { host: HostDetail }) {
  const navigate = useNavigate();
  const { range: live } = useTime();
  const { to } = snapRange(live);
  // Host charts look back a full day like the reference, independent of the 15m list window.
  const range = useMemo(() => ({ from: to - 24 * 3600e3, to }), [to]);
  const q = useHostMetrics(host.name, range);
  const [integration, setIntegration] = useState(host.integrations[0]?.name ?? '');
  const [filter, setFilter] = useState('');
  const current = host.integrations.find((i) => i.name === integration);
  const brokenCount = host.integrations.filter((i) => i.status === 'broken').length;

  return (
    <HpSection
      id="metrics"
      title="Metrics"
      icon={Gauge}
      extra={
        <>
          <span className="hm-stats">
            <span>CPU Usage</span> <strong>{host.cpuPct}%</strong>
            <span>Memory Usage</span> <strong>{host.memPct}%</strong>
            <span>NTP Offset</span> <strong>{host.ntpOffsetMs}ms</strong>
            <span>System Uptime</span> <strong>{fmtUptime(host.uptimeSec)}</strong>
          </span>
          <button className="btn" onClick={() => navigate('/dashboards')}>
            View in Dashboard <ExternalLink size={13} />
          </button>
        </>
      }
    >
      {q.isPending ? <Loading label="Loading metrics…" /> : q.isError ? <ErrorState error={q.error} onRetry={() => q.refetch()} /> : <Charts m={q.data} />}

      {host.integrations.length > 0 && (
        <div className="hm-integrations">
          <h4>
            <Puzzle size={16} /> Integrations {brokenCount > 0 && <span className="hm-broken-badge">{brokenCount} BROKEN</span>}
          </h4>
          <div className="hm-int-bar">
            <span className="hm-int-label">Integration</span>
            <Select
              value={integration}
              options={host.integrations.map((i) => ({ value: i.name, label: i.name, hint: i.status === 'broken' ? 'broken' : undefined }))}
              onChange={setIntegration}
              triggerClassName="dd-trigger hm-int-select"
            />
            <span className="hm-int-label">Metric</span>
            <label className="hm-int-filter">
              <Search size={14} />
              <input placeholder="Filter metrics by name" value={filter} onChange={(e) => setFilter(e.target.value)} />
            </label>
            <button className="btn" onClick={() => navigate('/integrations')}>
              View Integration Dashboard <ExternalLink size={13} />
            </button>
          </div>
          {current?.status === 'broken' ? (
            <>
              <div className="hm-broken">
                <TriangleAlert size={16} /> <em>This integration is broken</em>
                <button className="btn btn-sm" onClick={() => navigate('/integrations?q=nginx')}>
                  View Integration <ExternalLink size={12} />
                </button>
              </div>
              <div className="hm-empty">
                <TriangleAlert size={56} className="hm-empty-icon" />
                No corresponding metrics found
              </div>
            </>
          ) : (
            <ul className="hm-metric-list">
              {current?.metrics
                .filter((x) => x.includes(filter))
                .map((x) => (
                  <li key={x} className="mono">
                    {x}
                  </li>
                ))}
            </ul>
          )}
        </div>
      )}
    </HpSection>
  );
}
