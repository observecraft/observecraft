import { useMemo } from 'react';
import { Link } from 'react-router-dom';
import { reliabilityByTeam } from '../../../api/portalCatalog/reports';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { useTime } from '../../../components/shell/TimeContext';
import { usePortalCatalog, useReportMonitors, useReportSlos } from '../catalog/queries';
import { useReports } from './ReportsLayout';

const fmtBudget = (v: number | null | undefined) => (v === null || v === undefined ? '—' : `${v.toFixed(1)}%`);

/** Reports → Reliability Overview: SLO status and error budget, and monitors in alert, per owning team. */
export function ReliabilityOverview() {
  const { teams } = useReports();
  const { range } = useTime();
  const slos = useReportSlos();
  const monitors = useReportMonitors();
  const catalog = usePortalCatalog(range, null);
  const owners = useMemo(() => new Map((catalog.data?.entities ?? []).filter((e) => e.kind === 'service' && e.owner).map((e) => [e.name, e.owner])), [catalog.data]);
  const rows = useMemo(() => reliabilityByTeam(slos.data?.slos ?? [], monitors.data?.monitors ?? [], owners, teams), [slos.data, monitors.data, owners, teams]);
  if (slos.isPending || monitors.isPending) return <Loading label="Loading SLOs and monitors…" />;
  if (slos.isError) return <ErrorState error={slos.error} onRetry={() => slos.refetch()} />;
  if (monitors.isError) return <ErrorState error={monitors.error} onRetry={() => monitors.refetch()} />;
  const ownerOf = (own: string[]) => (own.length ? own.join(', ') : '');
  const atRisk = slos.data.slos.filter((s) => s.status.state === 'breached' || s.status.state === 'warning').filter((s) => !teams.length || s.teams.some((t) => teams.includes(t)) || !s.teams.length);
  const alerting = monitors.data.monitors.filter((m) => m.status === 'alert').filter((m) => !teams.length || m.teams.some((t) => teams.includes(t)) || !m.teams.length);

  return (
    <div className="pr-report">
      <h1 className="pr-title">Reliability Overview</h1>
      <div className="pr-stats">
        <Stat label="SLOs" value={rows.reduce((n, r) => n + r.slos, 0)} />
        <Stat label="Breached" value={rows.reduce((n, r) => n + r.breached, 0)} tone="alert" />
        <Stat label="Warning" value={rows.reduce((n, r) => n + r.warning, 0)} tone="warn" />
        <Stat label="Monitors" value={rows.reduce((n, r) => n + r.monitors, 0)} />
        <Stat label="In alert" value={rows.reduce((n, r) => n + r.alerting, 0)} tone="alert" />
      </div>
      <section className="pr-section">
        <header className="pr-section-head">
          <h2>By team</h2>
          <span className="pr-muted">SLOs and monitors count for their teams, or for the owners of the services they name.</span>
        </header>
        {rows.length ? (
          <div className="pr-table-wrap">
            <table className="pr-table">
              <thead>
                <tr>
                  <th>Team</th>
                  <th>SLOs</th>
                  <th>Breached</th>
                  <th>Warning</th>
                  <th>Lowest error budget</th>
                  <th>Monitors</th>
                  <th>In alert</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((r) => (
                  <tr key={r.team || '-'}>
                    <td>{r.team ? <strong>{r.team}</strong> : <span className="pr-muted">No team</span>}</td>
                    <td>{r.slos}</td>
                    <td className={r.breached ? 'pr-alert' : undefined}>{r.breached}</td>
                    <td className={r.warning ? 'pr-warn' : undefined}>{r.warning}</td>
                    <td className={(r.minBudget ?? 100) < 0 ? 'pr-alert' : undefined}>{fmtBudget(r.minBudget)}</td>
                    <td>{r.monitors}</td>
                    <td className={r.alerting ? 'pr-alert' : undefined}>{r.alerting}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : (
          <p className="pr-muted">
            No SLOs or monitors yet. <Link to="/slo/create">Create an SLO</Link> or <Link to="/monitors/create">a monitor</Link>.
          </p>
        )}
      </section>
      <div className="pr-cards">
        <section className="pr-card">
          <div className="pr-card-title">SLOs at risk</div>
          {atRisk.length ? (
            <ul className="pr-list">
              {atRisk.slice(0, 12).map((s) => (
                <li key={s.id}>
                  <Link to={`/slo?slo=${encodeURIComponent(s.id)}`}>{s.name}</Link>
                  <span className={s.status.state === 'breached' ? 'pr-alert' : 'pr-warn'}>
                    {s.status.state} · budget {fmtBudget(s.status.budgetRemaining)}
                  </span>
                  {ownerOf(s.teams) && <span className="pr-muted">{ownerOf(s.teams)}</span>}
                </li>
              ))}
            </ul>
          ) : (
            <p className="pr-muted">Every SLO is within its target.</p>
          )}
        </section>
        <section className="pr-card">
          <div className="pr-card-title">Monitors in alert</div>
          {alerting.length ? (
            <ul className="pr-list">
              {alerting.slice(0, 12).map((m) => (
                <li key={m.id}>
                  <Link to={`/monitors/${m.id}`}>{m.name}</Link>
                  {ownerOf(m.teams) && <span className="pr-muted">{ownerOf(m.teams)}</span>}
                </li>
              ))}
            </ul>
          ) : (
            <p className="pr-muted">No monitor is alerting.</p>
          )}
        </section>
      </div>
    </div>
  );
}

function Stat({ label, value, tone }: { label: string; value: number; tone?: 'alert' | 'warn' }) {
  return (
    <div className={`pr-stat${tone && value ? ` pr-stat-${tone}` : ''}`}>
      <span className="pr-stat-value">{value}</span>
      <span className="pr-stat-label">{label}</span>
    </div>
  );
}
