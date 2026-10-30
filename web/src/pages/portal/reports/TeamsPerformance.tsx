import { useQuery } from '@tanstack/react-query';
import { Link } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { BellRing, Info, PhoneCall, Target } from 'lucide-react';
import { searchIncidents, SEVERITIES, type IncidentSummary } from '../../../api/incidents/client';
import { Chart } from '../../../components/charts/Chart';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { useTeamsReport } from '../catalog/queries';
import { useReports } from './ReportsLayout';
import { ReportSection } from './widgets';

const fmtDuration = (minutes: number | null | undefined) => {
  if (minutes === null || minutes === undefined) return '—';
  if (minutes < 60) return `${Math.round(minutes)} min`;
  if (minutes < 48 * 60) return `${(minutes / 60).toFixed(1)} h`;
  return `${(minutes / 1440).toFixed(1)} d`;
};

const COVERAGE: { key: 'slos' | 'monitors' | 'onCallSchedules'; title: string; icon: typeof Target; text: string; learn: string; add: string; addTo: string }[] = [
  {
    key: 'slos',
    title: 'SLOs',
    icon: Target,
    text: 'Targets for what users notice (availability, latency), with an error budget that tells a team when to slow down.',
    learn: '/slo',
    add: 'Add SLOs',
    addTo: '/slo/create',
  },
  {
    key: 'monitors',
    title: 'Monitors',
    icon: BellRing,
    text: "Alerts on a team's services so problems reach the right people before customers report them.",
    learn: '/monitors',
    add: 'Add Monitors',
    addTo: '/monitors/create',
  },
  {
    key: 'onCallSchedules',
    title: 'On-Call',
    icon: PhoneCall,
    text: 'A rotation that says who answers pages for the team, at any hour.',
    learn: '/on-call',
    add: 'Add a Schedule',
    addTo: '/on-call/schedules/new',
  },
];

/** Reports → Teams Performance: incident response per team, then what each team is missing. */
export function TeamsPerformance() {
  const { range, teams } = useReports();
  const q = useTeamsReport({ range, teams });
  if (q.isPending) return <Loading label="Loading the report…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const data = q.data;

  return (
    <div className="pr-report">
      <h1 className="pr-title">Teams Performance</h1>
      {data.totalTeams < 3 && (
        <div className="pr-banner" role="note">
          <Info size={16} />
          <div>
            <strong>Set up more teams</strong>
            <p>This report compares teams. Create your teams and give services an owner (in the catalog or Scorecards) so incidents, SLOs and monitors are counted for them.</p>
          </div>
          <Link className="btn btn-sm" to="/teams">
            Manage teams
          </Link>
        </div>
      )}

      <IncidentResponse />

      <div className="pr-divider">
        <span>Expand teams observability</span>
      </div>

      {COVERAGE.map((c) => {
        const missing = data.teams.filter((t) => !t[c.key]);
        return (
          <section key={c.key} className="pr-coverage">
            <div className="pr-coverage-icon">
              <c.icon size={20} />
            </div>
            <div className="pr-coverage-body">
              <h3>{c.title}</h3>
              <p>{c.text}</p>
              <p className="pr-muted">
                {missing.length
                  ? `${data.teams.length - missing.length} of ${data.teams.length} teams have ${c.title === 'On-Call' ? 'an On-Call schedule' : c.title}. Missing: ${missing
                      .slice(0, 6)
                      .map((t) => t.name)
                      .join(', ')}${missing.length > 6 ? ` and ${missing.length - 6} more` : ''}.`
                  : `Every team has ${c.title === 'On-Call' ? 'an On-Call schedule' : c.title}.`}
              </p>
            </div>
            <div className="pr-coverage-actions">
              <Link className="btn btn-sm" to={c.learn}>
                Learn more
              </Link>
              {missing.length > 0 && (
                <Link className="btn btn-sm btn-primary" to={c.addTo}>
                  {c.add}
                </Link>
              )}
            </div>
          </section>
        );
      })}
    </div>
  );
}

const SEV_COLOR: Record<string, string> = { 'SEV-1': '#7a1438', 'SEV-2': '#c2185b', 'SEV-3': '#e0457b', 'SEV-4': '#f08cad', 'SEV-5': '#f7c4d5', UNKNOWN: '#d9416f' };
const sevColor = (s: string) => SEV_COLOR[s] ?? SEV_COLOR.UNKNOWN;

/** A stacked bar of incidents per severity. */
function SeverityBar({ counts }: { counts: Map<string, number> }) {
  return (
    <span className="tp-sevbar">
      {SEVERITIES.filter((s) => counts.get(s)).map((s) => (
        <i key={s} title={`${s}: ${counts.get(s)}`} style={{ flexGrow: counts.get(s), background: sevColor(s) }} />
      ))}
    </span>
  );
}

const bySeverity = (list: IncidentSummary[]) => {
  const m = new Map<string, number>();
  for (const i of list) m.set(i.severity, (m.get(i.severity) ?? 0) + 1);
  return m;
};

/** Teams Performance → Incident Response: incidents by severity, per week, and per team. */
function IncidentResponse() {
  const { range, teams } = useReports();
  const q = useQuery({ queryKey: ['portal', 'reports', 'incidents', range], queryFn: () => searchIncidents({ range, limit: 1000, offset: 0 }), retry: 1 });
  if (q.isPending) return <Loading label="Loading incidents…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const all = q.data.incidents.filter((i) => !i.test);
  const shown = teams.length ? all.filter((i) => i.teams.some((t) => teams.includes(t))) : all;
  const sev = bySeverity(shown);
  const week = 7 * 86_400_000;
  const weeks: number[] = [];
  for (let t = Math.floor(range.from / week) * week; t <= range.to; t += week) weeks.push(t);
  const weekly: EChartsOption = {
    grid: { left: 8, right: 12, top: 14, bottom: 40, containLabel: true },
    tooltip: { trigger: 'axis' },
    legend: { bottom: 0 },
    xAxis: { type: 'time' },
    yAxis: { type: 'value', minInterval: 1 },
    series: SEVERITIES.filter((s) => sev.get(s)).map((s) => ({
      name: s,
      type: 'bar',
      stack: 'sev',
      barMaxWidth: 14,
      color: sevColor(s),
      data: weeks.map((t) => [t + week / 2, shown.filter((i) => i.severity === s && i.declaredAt >= t && i.declaredAt < t + week).length]),
    })),
  };
  const byTeam = new Map<string, IncidentSummary[]>();
  for (const i of shown) for (const t of i.teams.length ? i.teams : ['N/A']) byTeam.set(t, [...(byTeam.get(t) ?? []), i]);
  const impact = (list: IncidentSummary[]) => {
    const d = list.filter((i) => i.customerImpact && i.resolvedAt).map((i) => i.resolvedAt! - i.declaredAt);
    return d.length ? d.reduce((a, b) => a + b, 0) / d.length : null;
  };
  const rows = [...byTeam].map(([team, list]) => ({ team, list, impact: impact(list) })).sort((a, b) => (b.impact ?? -1) - (a.impact ?? -1) || b.list.length - a.list.length);
  return (
    <ReportSection id="incident-response" title="Incident Response">
      <div className="tp-grid">
        <div className="tp-left">
          <div className="rw-card tp-statuses">
            <div className="tp-card-head">Incidents Statuses</div>
            <SeverityBar counts={sev} />
            <div className="tp-sev-cards">
              {['SEV-1', 'SEV-2'].map((s) => (
                <div key={s} className="tp-sev" style={{ borderLeftColor: sevColor(s) }}>
                  <b>{s}</b>
                  <span>
                    <strong>{sev.get(s) ?? 0}</strong> Incidents
                  </span>
                </div>
              ))}
              <div className="tp-sev tp-sev-wide">
                <b>Incidents without teams</b>
                <span>
                  <strong>{shown.filter((i) => !i.teams.length).length}</strong> Incidents
                </span>
              </div>
            </div>
          </div>
          <div className="rw-card tp-weekly">
            <div className="tp-card-head">Weekly Incident Count by Severity</div>
            <Chart option={weekly} height="100%" />
          </div>
        </div>
        <div className="rw-card tp-trends">
          <div className="tp-card-head">Incident Trends by Team</div>
          <table className="rw-legend">
            <thead>
              <tr>
                <th>TEAM</th>
                <th>AVG CUSTOMER IMPACT</th>
                <th>TREND</th>
                <th>SEVERITY BREAKDOWN</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => (
                <tr key={r.team}>
                  <td>{r.team}</td>
                  <td>{r.impact === null ? '—' : fmtDuration(r.impact / 60_000)}</td>
                  <td>
                    <Sparkline values={weeks.map((t) => r.list.filter((i) => i.declaredAt >= t && i.declaredAt < t + week).length)} />
                  </td>
                  <td>
                    <SeverityBar counts={bySeverity(r.list)} />
                  </td>
                </tr>
              ))}
              {!rows.length && (
                <tr>
                  <td colSpan={4} className="pr-muted">
                    No incidents in this period.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </div>
    </ReportSection>
  );
}

/** Weekly counts as a small line. */
function Sparkline({ values }: { values: number[] }) {
  const max = Math.max(1, ...values);
  const pts = values.map((v, i) => `${(i / Math.max(1, values.length - 1)) * 100},${20 - (v / max) * 18}`).join(' ');
  return (
    <svg className="tp-spark" viewBox="0 0 100 20" preserveAspectRatio="none" aria-hidden="true">
      <polyline points={pts} fill="none" stroke="var(--chart-1)" strokeWidth="1.5" vectorEffect="non-scaling-stroke" />
    </svg>
  );
}
