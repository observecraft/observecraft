import { useMemo, useState } from 'react';
import type { EChartsOption } from 'echarts';
import { PhoneCall } from 'lucide-react';
import type { OnCallAnalytics } from '../../api/onCall/client';
import { Chart } from '../../components/charts/Chart';
import { TimePicker } from '../../components/shell/PageHeader';
import { TimeProvider, useTime } from '../../components/shell/TimeContext';
import { ErrorState } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { palette } from '../../theme/chartTheme';
import { DAY, clampRange, fmtDuration } from '../../api/onCall/logic';
import { browserTimeZone } from './common';
import { useOnCallAnalytics, useOnCallTeams } from './queries';

const DOW = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
/** The analytics API answers at most this many days. */
const MAX_DAYS = 92;

/** On-Call → Analytics → Page Review: page volume, time to acknowledge and resolve, the most paged teams, busy hours. */
export function AnalyticsPage() {
  return (
    <TimeProvider initial="1w">
      <PageReview />
    </TimeProvider>
  );
}

function PageReview() {
  const [team, setTeam] = useState('*');
  const teams = useOnCallTeams();
  const tz = browserTimeZone();
  const { range: picked } = useTime();
  const minute = 60_000;
  const range = useMemo(() => clampRange({ from: Math.floor(picked.from / minute) * minute, to: Math.ceil(picked.to / minute) * minute }, MAX_DAYS), [picked.from, picked.to]);
  const q = useOnCallAnalytics({ range, teams: team === '*' ? [] : [team], timeZone: tz });
  const a = q.data;
  const empty = !a || a.total === 0;
  return (
    <div className="onc-review">
      <div className="onc-review-bar">
        <nav className="onc-subtabs">
          <span className="active">Page Review</span>
        </nav>
        <div className="onc-review-tools">
          <Select label="Team" value={team} onChange={setTeam} options={[{ value: '*', label: 'All teams' }, ...(teams.data ?? []).map((t) => ({ value: t.handle, label: t.name }))]} />
          <TimePicker compact />
        </div>
      </div>
      <div className="onc-summary">
        <div className="onc-hero">
          <div className="onc-kicker">
            <PhoneCall size={14} /> On-Call
          </div>
          <h1>Review how your teams get paged</h1>
        </div>
        {q.isError ? (
          <ErrorState error={q.error} onRetry={() => q.refetch()} />
        ) : (
          <>
            <section className="onc-card">
              <h2>On-Call Overview</h2>
              <p className="onc-card-sub">How many pages went out, and how fast they were acknowledged and resolved.</p>
              <div className="onc-kpis onc-kpis-3">
                <Kpi label="TOTAL PAGES" value={empty ? '–' : String(a.total)} hint={a && !empty ? `${Math.round((a.acknowledged / a.total) * 100)}% acknowledged` : undefined} />
                <Kpi label="MEAN TIME TO ACKNOWLEDGE (MTTA)" value={empty ? '–' : fmtDuration(a.mttaSeconds)} />
                <Kpi label="MEAN TIME TO RESOLVE (MTTR)" value={empty ? '–' : fmtDuration(a.mttrSeconds)} />
              </div>
            </section>
            <section className="onc-card">
              <h2>Organization Overview</h2>
              <p className="onc-card-sub">The teams paged most often, and when.</p>
              <div className="onc-panel">
                <div className="onc-panel-head">Most Paged Teams</div>
                <MostPagedTeams a={a} pending={q.isPending} />
              </div>
              <div className="onc-panel">
                <div className="onc-panel-head">Pages per team</div>
                <PagesPerTeam a={a} range={range} pending={q.isPending} />
              </div>
            </section>
            <section className="onc-card">
              <h2>Activity</h2>
              <p className="onc-card-sub">Busy hours (in {tz}) and the pages that fire most.</p>
              <div className="onc-two">
                <div className="onc-panel">
                  <div className="onc-panel-head">Pages by hour</div>
                  <Heatmap byHour={a?.byHour ?? []} />
                </div>
                <div className="onc-panel">
                  <div className="onc-panel-head">Noisiest sources</div>
                  <NoisiestSources a={a} pending={q.isPending} />
                </div>
              </div>
            </section>
          </>
        )}
      </div>
    </div>
  );
}

function Kpi({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <div className="onc-kpi">
      <span>{label}</span>
      <b>{value}</b>
      {hint && <small className="muted">{hint}</small>}
    </div>
  );
}

function NoData({ pending }: { pending: boolean }) {
  return <p className="muted onc-panel-empty">{pending ? 'Loading…' : 'No pages in this period.'}</p>;
}

function MostPagedTeams({ a, pending }: { a?: OnCallAnalytics; pending: boolean }) {
  if (!a?.teams.length) return <NoData pending={pending} />;
  const max = Math.max(...a.teams.map((t) => t.count));
  return (
    <table className="onc-table compact">
      <thead>
        <tr>
          <th>TEAM</th>
          <th>PAGES</th>
          <th className="onc-col-num">MTTA</th>
          <th className="onc-col-num">MTTR</th>
        </tr>
      </thead>
      <tbody>
        {a.teams.map((t) => (
          <tr key={t.team} className="static">
            <td>{t.team ? `@${t.team}` : 'No team'}</td>
            <td>
              <span className="onc-bar">
                <span style={{ width: `${(t.count / max) * 100}%` }} />
                <b>{t.count}</b>
              </span>
            </td>
            <td className="onc-col-num">{fmtDuration(t.mttaSeconds)}</td>
            <td className="onc-col-num">{fmtDuration(t.mttrSeconds)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function PagesPerTeam({ a, range, pending }: { a?: OnCallAnalytics; range: { from: number; to: number }; pending: boolean }) {
  const option = useMemo<EChartsOption | null>(() => {
    if (!a?.total) return null;
    const dayStarts: number[] = [];
    for (let t = range.from - (range.from % DAY); t <= range.to; t += DAY) dayStarts.push(t);
    const teamNames = [...new Set(a.perDay.map((d) => d.team))].sort();
    const bucket = (ms: number) => Math.floor((ms - dayStarts[0]) / DAY);
    return {
      color: palette(),
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
  return option ? <Chart option={option} height={240} /> : <NoData pending={pending} />;
}

function Heatmap({ byHour }: { byHour: number[] }) {
  const max = Math.max(1, ...byHour);
  return (
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
            const n = byHour[di * 24 + h] ?? 0;
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
  );
}

function NoisiestSources({ a, pending }: { a?: OnCallAnalytics; pending: boolean }) {
  if (!a?.topSources.length) return <NoData pending={pending} />;
  return (
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
      </tbody>
    </table>
  );
}
