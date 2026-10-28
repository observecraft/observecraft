import { useMemo, useState } from 'react';
import type { EChartsOption } from 'echarts';
import { Chart } from '../../components/charts/Chart';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { palette } from '../../theme/chartTheme';
import { DAY, fmtDuration } from '../../api/onCall/logic';
import { Segmented, browserTimeZone } from './common';
import { useOnCallAnalytics, useOnCallTeams } from './queries';

const DOW = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
const RANGES: ['7', '30', '90'][number][] = ['7', '30', '90'];

/** On-Call → Analytics: pages per team over time, time to acknowledge and resolve, busy hours, noisy sources. */
export function AnalyticsPage() {
  const [days, setDays] = useState<'7' | '30' | '90'>('30');
  const [team, setTeam] = useState('*');
  const teams = useOnCallTeams();
  const tz = browserTimeZone();
  const minute = Math.floor(Date.now() / 60_000) * 60_000;
  const range = useMemo(() => ({ from: minute - Number(days) * DAY, to: minute + 60_000 }), [minute, days]);
  const q = useOnCallAnalytics({ range, teams: team === '*' ? [] : [team], timeZone: tz });
  const a = q.data;
  const option = useMemo<EChartsOption | null>(() => {
    if (!a) return null;
    const dayStarts: number[] = [];
    for (let t = range.from - (range.from % DAY); t <= range.to; t += DAY) dayStarts.push(t);
    const teamNames = [...new Set(a.perDay.map((d) => d.team))].sort();
    const colors = palette();
    const bucket = (ms: number) => Math.floor((ms - dayStarts[0]) / DAY);
    return {
      color: colors,
      grid: { left: 36, right: 12, top: 28, bottom: 24 },
      legend: { top: 0, left: 0, itemWidth: 10, itemHeight: 10 },
      tooltip: { trigger: 'axis' },
      xAxis: { type: 'category', data: dayStarts.map((t) => new Date(t).toLocaleDateString('en-US', { month: 'short', day: 'numeric' })) },
      yAxis: { type: 'value', minInterval: 1 },
      series: teamNames.map((tn) => {
        const data = dayStarts.map(() => 0);
        for (const d of a.perDay.filter((x) => x.team === tn)) {
          const i = bucket(d.day + 12 * 3_600_000);
          if (i >= 0 && i < data.length) data[i] += d.count;
        }
        return { name: tn ? `@${tn}` : 'No team', type: 'bar', stack: 'pages', data, barMaxWidth: 18 };
      }),
    };
  }, [a, range]);
  const max = Math.max(1, ...(a?.byHour ?? [0]));
  const ackRate = a && a.total ? Math.round((a.acknowledged / a.total) * 100) : null;
  return (
    <div className="onc-list-page">
      <div className="onc-main-bar">
        <Segmented value={days} onChange={setDays} options={RANGES.map((d) => [d, `Past ${d} days`] as ['7' | '30' | '90', string])} />
        <Select label="Teams" value={team} onChange={setTeam} options={[{ value: '*', label: 'All teams' }, ...(teams.data ?? []).map((t) => ({ value: t.handle, label: t.name }))]} />
        <span className="muted onc-showing">Days and hours in {tz}</span>
      </div>
      {q.isPending ? (
        <Loading label="Loading analytics…" />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        a && (
          <>
            <div className="onc-kpis">
              <div className="onc-kpi">
                <span>Pages</span>
                <b>{a.total}</b>
              </div>
              <div className="onc-kpi">
                <span>Acknowledged</span>
                <b>{ackRate == null ? '—' : `${ackRate}%`}</b>
              </div>
              <div className="onc-kpi">
                <span>Mean time to acknowledge</span>
                <b>{fmtDuration(a.mttaSeconds)}</b>
              </div>
              <div className="onc-kpi">
                <span>Mean time to resolve</span>
                <b>{fmtDuration(a.mttrSeconds)}</b>
              </div>
            </div>
            <section className="onc-card">
              <h2>Pages per team</h2>
              {option && a.total > 0 ? <Chart option={option} height={240} /> : <p className="muted">No pages in this period.</p>}
            </section>
            <div className="onc-two">
              <section className="onc-card">
                <h2>Pages by hour</h2>
                <div className="onc-heat">
                  <span />
                  {Array.from({ length: 24 }, (_, h) => (
                    <span key={h} className="onc-heat-h">
                      {h % 3 === 0 ? h : ''}
                    </span>
                  ))}
                  {DOW.map((d, di) => (
                    <div key={d} className="onc-heat-row">
                      <span className="onc-heat-d">{d}</span>
                      {Array.from({ length: 24 }, (_, h) => {
                        const n = a.byHour[di * 24 + h] ?? 0;
                        return (
                          <span
                            key={h}
                            className="onc-heat-cell"
                            title={`${d} ${String(h).padStart(2, '0')}:00 · ${n} page${n === 1 ? '' : 's'}`}
                            style={{ opacity: n ? 0.18 + 0.82 * (n / max) : 1 }}
                            data-empty={n ? undefined : ''}
                          />
                        );
                      })}
                    </div>
                  ))}
                </div>
              </section>
              <section className="onc-card">
                <h2>Noisiest sources</h2>
                <table className="onc-table compact">
                  <thead>
                    <tr>
                      <th>PAGE</th>
                      <th className="onc-col-num">PAGES</th>
                    </tr>
                  </thead>
                  <tbody>
                    {a.topSources.map((s) => (
                      <tr key={s.title} className="static">
                        <td className="ellipsis" title={s.title}>
                          {s.title} <span className="onc-chip">{s.source}</span>
                        </td>
                        <td className="onc-col-num">{s.count}</td>
                      </tr>
                    ))}
                    {!a.topSources.length && (
                      <tr className="static">
                        <td colSpan={2} className="muted">
                          No pages in this period.
                        </td>
                      </tr>
                    )}
                  </tbody>
                </table>
              </section>
            </div>
            <section className="onc-card">
              <h2>By team</h2>
              <table className="onc-table compact">
                <thead>
                  <tr>
                    <th>TEAM</th>
                    <th className="onc-col-num">PAGES</th>
                    <th className="onc-col-num">MTTA</th>
                    <th className="onc-col-num">MTTR</th>
                  </tr>
                </thead>
                <tbody>
                  {a.teams.map((t) => (
                    <tr key={t.team} className="static">
                      <td>{t.team ? `@${t.team}` : 'No team'}</td>
                      <td className="onc-col-num">{t.count}</td>
                      <td className="onc-col-num">{fmtDuration(t.mttaSeconds)}</td>
                      <td className="onc-col-num">{fmtDuration(t.mttrSeconds)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </section>
          </>
        )
      )}
    </div>
  );
}
