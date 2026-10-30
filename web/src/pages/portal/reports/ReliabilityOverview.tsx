import { useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import { useQuery } from '@tanstack/react-query';
import { ArrowRight, ExternalLink, Gauge } from 'lucide-react';
import type { EChartsOption } from 'echarts';
import { searchIncidents, type IncidentSummary } from '../../../api/incidents/client';
import { servicesNamed } from '../../../api/portalCatalog/reports';
import { Chart } from '../../../components/charts/Chart';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { Select } from '../../../components/ui/Dropdown';
import { palette, token } from '../../../theme/chartTheme';
import { useOnCallTeams, useReportSlos } from '../catalog/queries';
import { RangeSelect, useReports } from './ReportsLayout';
import { BigNumber, ReportSection, short } from './widgets';

const ALL = '*';
const DAY = 86_400_000;
const STATES = ['active', 'stable', 'resolved'];

/** 5400 s → "1.5h"; durations of incidents. */
function duration(ms: number): string {
  const s = ms / 1000;
  if (s < 60) return `${Math.round(s)}s`;
  if (s < 3600) return `${+(s / 60).toFixed(1)}m`;
  if (s < 86400) return `${+(s / 3600).toFixed(1)}h`;
  return `${+(s / 86400).toFixed(1)}d`;
}

/** Incidents per value of `key`, as a donut with the total inside (0 when there are none). */
function CountPie({ title, counts }: { title: string; counts: [string, number][] }) {
  const total = counts.reduce((n, [, v]) => n + v, 0);
  const colors = palette();
  const option: EChartsOption = {
    tooltip: { trigger: 'item' },
    legend: { bottom: 0, type: 'scroll' },
    series: [
      {
        type: 'pie',
        radius: ['45%', '70%'],
        center: ['50%', '45%'],
        label: { show: false },
        data: counts.map(([name, value], i) => ({ name, value, itemStyle: { color: colors[i % colors.length] } })),
      },
    ],
    graphic: [{ type: 'text', left: 'center', top: '40%', style: { text: short(total), fontSize: 26, fontWeight: 600 } }],
  };
  return (
    <div className="rw-card rw-pie">
      <div className="rw-card-title">{title}</div>
      {total ? <Chart option={option} height="100%" /> : <strong className="rw-zero">0</strong>}
    </div>
  );
}

/** The mean of a duration per period, as a line (empty periods have no point). */
function TrendCard({ title, points }: { title: string; points: [number, number | null][] }) {
  const option: EChartsOption = {
    grid: { left: 8, right: 12, top: 14, bottom: 22, containLabel: true },
    tooltip: { trigger: 'axis', valueFormatter: (v) => (typeof v === 'number' ? duration(v) : '—') },
    xAxis: { type: 'time', axisLabel: { hideOverlap: true } },
    yAxis: { type: 'value', axisLabel: { formatter: (v: number) => (v ? duration(v) : '0') } },
    series: [{ type: 'line', showSymbol: false, connectNulls: false, color: token('--chart-1'), data: points }],
  };
  return (
    <div className="rw-card rw-trend">
      <div className="rw-card-title">{title}</div>
      <Chart option={option} height="100%" />
    </div>
  );
}

const mean = (v: number[]) => (v.length ? v.reduce((a, b) => a + b, 0) / v.length : null);

/** Reports → Reliability Overview: incidents by severity and state, how fast they're repaired and resolved, and SLOs. */
export function ReliabilityOverview() {
  const { range, teams } = useReports();
  const teamList = useOnCallTeams();
  const slos = useReportSlos();
  const [team, setTeam] = useState(teams[0] ?? ALL);
  const [service, setService] = useState(ALL);
  const [severity, setSeverity] = useState(ALL);
  const [state, setState] = useState(ALL);
  const [rollup, setRollup] = useState<'daily' | 'weekly'>('weekly');
  const [groupBy, setGroupBy] = useState<'team' | 'entity'>('team');
  const incidents = useQuery({ queryKey: ['portal', 'reports', 'incidents', range], queryFn: () => searchIncidents({ range, limit: 1000, offset: 0 }), retry: 1 });

  const shown = useMemo(
    () =>
      (incidents.data?.incidents ?? []).filter(
        (i: IncidentSummary) =>
          !i.test &&
          (team === ALL || i.teams.includes(team)) &&
          (service === ALL || i.services.includes(service)) &&
          (severity === ALL || i.severity === severity) &&
          (state === ALL || i.status === state),
      ),
    [incidents.data, team, service, severity, state],
  );
  if (incidents.isPending || slos.isPending) return <Loading label="Loading incidents and SLOs…" />;
  if (incidents.isError) return <ErrorState error={incidents.error} onRetry={() => incidents.refetch()} />;
  if (slos.isError) return <ErrorState error={slos.error} onRetry={() => slos.refetch()} />;

  const count = (key: (i: IncidentSummary) => string) => {
    const m = new Map<string, number>();
    for (const i of shown) m.set(key(i), (m.get(key(i)) ?? 0) + 1);
    return [...m].sort((a, b) => b[1] - a[1]);
  };
  const resolved = shown.filter((i) => i.resolvedAt);
  // Repair: declared → resolved; resolve: detected → resolved (detection can come before the declaration).
  const repairOf = (i: IncidentSummary) => i.resolvedAt! - i.declaredAt;
  const resolveOf = (i: IncidentSummary) => i.resolvedAt! - Math.min(i.detectedAt || i.declaredAt, i.declaredAt);
  const period = rollup === 'weekly' ? 7 * DAY : DAY;
  const trend = (of: (i: IncidentSummary) => number): [number, number | null][] => {
    const out: [number, number | null][] = [];
    for (let t = Math.floor(range.from / period) * period; t <= range.to; t += period) out.push([t, mean(resolved.filter((i) => i.resolvedAt! >= t && i.resolvedAt! < t + period).map(of))]);
    return out;
  };
  const allServices = [...new Set((incidents.data?.incidents ?? []).flatMap((i) => i.services))].sort();
  const options = (values: string[]) => [{ value: ALL, label: '*' }, ...values.map((v) => ({ value: v, label: v }))];
  const rollupLabel = rollup === 'weekly' ? 'Weekly' : 'Daily';

  // SLOs of the selected team (all of them when none is selected).
  const sloList = (slos.data?.slos ?? []).filter((s) => team === ALL || s.teams.includes(team));
  const states = (list: typeof sloList) => ({
    breached: list.filter((s) => s.status.state === 'breached').length,
    warning: list.filter((s) => s.status.state === 'warning').length,
    ok: list.filter((s) => s.status.state === 'ok').length,
    noData: list.filter((s) => s.status.state === 'no_data').length,
    total: list.length,
    // The share of SLOs meeting their target, among those with data.
    score: list.length ? (100 * list.filter((s) => s.status.state === 'ok' || s.status.state === 'warning').length) / Math.max(1, list.filter((s) => s.status.state !== 'no_data').length) : null,
  });
  const groups = new Map<string, typeof sloList>();
  for (const s of sloList) {
    const keys = groupBy === 'team' ? s.teams : servicesNamed([s.goodQuery, s.totalQuery, s.sliceQuery, ...s.tags]);
    for (const k of keys.length ? keys : ['N/A']) groups.set(k, [...(groups.get(k) ?? []), s]);
  }
  const summary = states(sloList);

  return (
    <div className="pr-report">
      <div className="rw-filters">
        <Select label="team" value={team} options={[{ value: ALL, label: '*' }, ...(teamList.data ?? []).map((t) => ({ value: t.handle, label: t.name }))]} onChange={setTeam} />
        <Select label="services" value={service} options={options(allServices)} onChange={setService} />
        <Select label="incident-severity" value={severity} options={options(['SEV-1', 'SEV-2', 'SEV-3', 'SEV-4', 'SEV-5', 'UNKNOWN'])} onChange={setSeverity} />
        <Select label="incident-state" value={state} options={options(STATES)} onChange={setState} />
        <span className="rw-inline">
          Roll up
          <Select
            value={rollup}
            options={[
              { value: 'daily', label: 'Daily' },
              { value: 'weekly', label: 'Weekly' },
            ]}
            onChange={(v) => setRollup(v as 'daily' | 'weekly')}
          />
        </span>
        <span className="rw-inline">
          Group by
          <span className="btn-group">
            <button className={`btn btn-sm${groupBy === 'team' ? ' btn-active' : ''}`} onClick={() => setGroupBy('team')}>
              Team
            </button>
            <button className={`btn btn-sm${groupBy === 'entity' ? ' btn-active' : ''}`} onClick={() => setGroupBy('entity')}>
              Entity
            </button>
          </span>
        </span>
        <span className="rw-push">
          <RangeSelect />
        </span>
      </div>
      <h1 className="pr-title">Reliability Overview</h1>

      <ReportSection id="incident-trends" title="Incident Trends">
        <div className="rw-reliability">
          <CountPie title="Incidents by Severity" counts={count((i) => i.severity)} />
          <CountPie title="Incidents by State" counts={count((i) => i.status)} />
          <BigNumber title="Mean Time to Repair" value={mean(resolved.map(repairOf)) ?? 0} format={duration} />
          <TrendCard title={`${rollupLabel} Mean Time to Repair`} points={trend(repairOf)} />
          <BigNumber title="Mean Time to Resolve" value={mean(resolved.map(resolveOf)) ?? 0} format={duration} />
          <TrendCard title={`${rollupLabel} Mean Time to Resolve`} points={trend(resolveOf)} />
        </div>
      </ReportSection>

      <ReportSection id="slos" title={`SLO Summary by ${groupBy === 'team' ? 'Team' : 'Entity'}`}>
        {sloList.length === 0 ? (
          <div className="rw-unlock">
            <span className="rw-pill">
              <Gauge size={15} /> 0 SLOs
            </span>
            <h3>Add SLOs to populate this report with insights on your product reliability.</h3>
            <div className="rw-unlock-actions">
              <Link className="btn" to="/slo">
                Learn More <ExternalLink size={14} />
              </Link>
              <Link className="btn btn-primary" to="/slo/create">
                Get Started With SLOs <ArrowRight size={14} />
              </Link>
            </div>
          </div>
        ) : (
          <>
            <div className="rw-tiles rw-tiles-6 rw-slo-tiles">
              <BigNumber title="SCORE" value={summary.score} format={(n) => n.toFixed(1)} tone="var(--status-ok)" />
              <BigNumber title="BREACHED" value={summary.breached} tone="var(--status-error)" />
              <BigNumber title="WARN" value={summary.warning} tone="var(--status-warn)" />
              <BigNumber title="OK" value={summary.ok} tone="var(--status-ok)" />
              <BigNumber title="NO DATA" value={summary.noData} />
              <BigNumber title="TOTAL" value={summary.total} />
            </div>
            <div className="rw-card">
              <div className="rw-card-title">SLO Summary grouped by {groupBy}</div>
              <table className="rw-legend rw-slo-table">
                <thead>
                  <tr>
                    <th>GROUP</th>
                    <th className="num">SCORE</th>
                    <th className="num">BREACHED</th>
                    <th className="num">WARN</th>
                    <th className="num">OK</th>
                    <th className="num">NO DATA</th>
                    <th className="num">TOTAL</th>
                    <th>BREAKDOWN</th>
                  </tr>
                </thead>
                <tbody>
                  {[...groups]
                    .map(([g, list]) => [g, states(list)] as const)
                    .sort((a, b) => b[1].total - a[1].total)
                    .map(([g, st]) => (
                      <tr key={g}>
                        <td>{g}</td>
                        <td className="num">{st.score === null ? '—' : st.score.toFixed(1)}</td>
                        <td className="num">{st.breached}</td>
                        <td className="num">{st.warning}</td>
                        <td className="num">{st.ok}</td>
                        <td className="num">{st.noData}</td>
                        <td className="num">{st.total}</td>
                        <td>
                          <span className="rw-breakdown">
                            <i style={{ flexGrow: st.breached, background: 'var(--status-error)' }} />
                            <i style={{ flexGrow: st.warning, background: 'var(--status-warn)' }} />
                            <i style={{ flexGrow: st.ok, background: 'var(--status-ok)' }} />
                            <i style={{ flexGrow: st.noData, background: 'var(--border-strong)' }} />
                          </span>
                        </td>
                      </tr>
                    ))}
                </tbody>
              </table>
            </div>
          </>
        )}
      </ReportSection>
    </div>
  );
}
