import { useMemo } from 'react';
import { Link } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { BellRing, Info, PhoneCall, Target } from 'lucide-react';
import type { PortalTeamReport } from '../../../api/portalCatalog/types';
import { Chart } from '../../../components/charts/Chart';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { useTeamsReport } from '../catalog/queries';
import { useReports } from './ReportsLayout';

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
  const { range, teams, rangeLabel } = useReports();
  const q = useTeamsReport({ range, teams });
  const option = useMemo<EChartsOption | null>(() => {
    if (!q.data) return null;
    const withIncidents = q.data.teams.filter((t) => t.incidentCount > 0);
    return {
      grid: { left: 36, right: 12, top: 16, bottom: 48 },
      tooltip: { trigger: 'axis' },
      legend: { bottom: 0, type: 'scroll' },
      xAxis: {
        type: 'category',
        data: q.data.buckets.map((b) => new Date(b).toLocaleDateString([], { month: 'short', day: 'numeric' })),
      },
      yAxis: { type: 'value', minInterval: 1 },
      series: withIncidents.map((t) => ({ name: t.name, type: 'bar', stack: 'incidents', barMaxWidth: 28, data: t.incidents })),
    };
  }, [q.data]);
  if (q.isPending) return <Loading label="Loading the report…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const data = q.data;
  const total = data.teams.reduce((n, t) => n + t.incidentCount, 0);
  const resolved = data.teams.filter((t) => t.mttrMinutes !== null && t.mttrMinutes !== undefined);

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

      <section className="pr-section">
        <header className="pr-section-head">
          <h2>Incident Response</h2>
          <span className="pr-muted">
            {total} incident{total === 1 ? '' : 's'} · {rangeLabel}
          </span>
        </header>
        <div className="pr-cards">
          <div className="pr-card pr-card-wide">
            <div className="pr-card-title">Incidents per team</div>
            {total ? option && <Chart option={option} height={240} /> : <p className="pr-muted pr-chart-empty">No incidents involved these teams in this period.</p>}
          </div>
          <div className="pr-card">
            <div className="pr-card-title">Mean time to resolve</div>
            {resolved.length ? (
              <ul className="pr-bars">
                {[...resolved]
                  .sort((a, b) => (b.mttrMinutes ?? 0) - (a.mttrMinutes ?? 0))
                  .slice(0, 8)
                  .map((t) => {
                    const max = Math.max(...resolved.map((r) => r.mttrMinutes ?? 0), 1);
                    return (
                      <li key={t.handle}>
                        <span className="pr-bar-label">{t.name}</span>
                        <span className="pr-bar">
                          <span style={{ width: `${((t.mttrMinutes ?? 0) * 100) / max}%` }} />
                        </span>
                        <span className="pr-bar-value">{fmtDuration(t.mttrMinutes)}</span>
                      </li>
                    );
                  })}
              </ul>
            ) : (
              <p className="pr-muted">No incident was resolved in this period.</p>
            )}
          </div>
        </div>
        <TeamTable teams={data.teams} />
      </section>

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

function TeamTable({ teams }: { teams: PortalTeamReport[] }) {
  if (!teams.length) return <p className="pr-muted">No teams match.</p>;
  return (
    <div className="pr-table-wrap">
      <table className="pr-table">
        <thead>
          <tr>
            <th>Team</th>
            <th>Members</th>
            <th>Services</th>
            <th>Incidents</th>
            <th>Open</th>
            <th>MTTR</th>
            <th>SLOs</th>
            <th>Monitors</th>
            <th>On-Call</th>
          </tr>
        </thead>
        <tbody>
          {teams.map((t) => (
            <tr key={t.handle}>
              <td>
                <strong>{t.name}</strong> <span className="pr-muted">@{t.handle}</span>
              </td>
              <td>{t.members}</td>
              <td>{t.services}</td>
              <td>{t.incidentCount}</td>
              <td className={t.openIncidents ? 'pr-alert' : undefined}>{t.openIncidents}</td>
              <td>{fmtDuration(t.mttrMinutes)}</td>
              <td className={t.slos ? undefined : 'pr-muted'}>{t.slos}</td>
              <td className={t.monitors ? undefined : 'pr-muted'}>{t.monitors}</td>
              <td className={t.onCallSchedules ? undefined : 'pr-muted'}>{t.onCallSchedules ? `${t.onCallSchedules} schedule${t.onCallSchedules === 1 ? '' : 's'}` : 'None'}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
