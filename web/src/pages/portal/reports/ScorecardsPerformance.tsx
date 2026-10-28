import { useMemo } from 'react';
import { Link } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { Chart } from '../../../components/charts/Chart';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { LevelBar } from '../common';
import { useScorecards } from '../queries';
import { useScorecardsReport } from '../catalog/queries';
import { useReports } from './ReportsLayout';

/** Reports → Scorecards Performance: each team's mean scorecard level per day, and where its services are now. */
export function ScorecardsPerformance() {
  const { range, teams } = useReports();
  const overview = useScorecards();
  const q = useScorecardsReport({ range, teams });
  const option = useMemo<EChartsOption | null>(() => {
    if (!q.data?.teams.length) return null;
    return {
      grid: { left: 36, right: 12, top: 16, bottom: 48 },
      tooltip: { trigger: 'axis', valueFormatter: (v) => (typeof v === 'number' ? `L${v.toFixed(2)}` : '—') },
      legend: { bottom: 0, type: 'scroll' },
      xAxis: { type: 'category', data: q.data.days.map((d) => new Date(d).toLocaleDateString([], { month: 'short', day: 'numeric' })) },
      yAxis: { type: 'value', min: 0, max: 3, interval: 1, axisLabel: { formatter: 'L{value}' } },
      series: q.data.teams.map((t) => ({ name: t.team || 'No team', type: 'line', showSymbol: false, connectNulls: true, data: t.levels })),
    };
  }, [q.data]);
  if (overview.isPending || q.isPending) return <Loading label="Loading the report…" />;
  if (overview.isError) return <ErrorState error={overview.error} onRetry={() => overview.refetch()} />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  if (!overview.data.setUp)
    return (
      <div className="pr-report">
        <h1 className="pr-title">Scorecards Performance</h1>
        <div className="pr-empty">
          <p>Scorecards aren't set up yet. Once they are, every evaluation records each service's level here.</p>
          <Link className="btn btn-primary" to="/portal/scorecards">
            Set up Scorecards
          </Link>
        </div>
      </div>
    );
  const rows = q.data.teams;
  return (
    <div className="pr-report">
      <h1 className="pr-title">Scorecards Performance</h1>
      <section className="pr-section">
        <header className="pr-section-head">
          <h2>Mean level per team</h2>
          <span className="pr-muted">Recorded at each day's last evaluation</span>
        </header>
        <div className="pr-card">{option ? <Chart option={option} height={260} /> : <p className="pr-muted pr-chart-empty">No evaluation in this period yet.</p>}</div>
      </section>
      <section className="pr-section">
        <header className="pr-section-head">
          <h2>Services by level</h2>
          <span className="pr-muted">On the last evaluated day</span>
        </header>
        {rows.length ? (
          <div className="pr-table-wrap">
            <table className="pr-table">
              <thead>
                <tr>
                  <th>Team</th>
                  <th>Services</th>
                  <th>Levels</th>
                  <th>L0</th>
                  <th>L1</th>
                  <th>L2</th>
                  <th>L3</th>
                  <th>Mean level</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((t) => {
                  const last = [...t.levels].reverse().find((v) => v !== null && v !== undefined);
                  return (
                    <tr key={t.team || '-'}>
                      <td>{t.team ? <strong>{t.team}</strong> : <span className="pr-muted">No team</span>}</td>
                      <td>{t.current.reduce((a, b) => a + b, 0)}</td>
                      <td>
                        <LevelBar levels={t.current} />
                      </td>
                      {t.current.map((n, l) => (
                        <td key={l}>{n}</td>
                      ))}
                      <td>{last === undefined || last === null ? '—' : `L${last.toFixed(2)}`}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        ) : (
          <p className="pr-muted">No evaluation in this period yet.</p>
        )}
      </section>
    </div>
  );
}
