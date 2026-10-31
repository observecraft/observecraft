import { useMemo } from 'react';
import { Link } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import type { RumNamedCount } from '../../api/rum/client';
import { Chart } from '../../components/charts/Chart';
import { TimePicker } from '../../components/shell/PageHeader';
import { useTime } from '../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { compact } from '../../lib/format';
import { token } from '../../theme/chartTheme';
import { fmtMsShort, useSelectedApplication } from './common';
import { useRumApplications, useRumSummary } from './queries';
import { SelectApplicationHero } from './RumLayout';

const minute = 60_000;

/** Core Web Vitals thresholds (good, poor), as web.dev defines them. */
const VITALS: Record<string, [number, number]> = { loading: [2500, 4000], lcp: [2500, 4000], inp: [200, 500], cls: [0.1, 0.25] };
const grade = (k: string, v: number | null | undefined) => (v == null ? '' : v <= VITALS[k][0] ? 'good' : v <= VITALS[k][1] ? 'fair' : 'poor');

/** RUM → Summary: an application's performance, errors, deployments and usage. */
export function RumSummaryPage() {
  const [selected] = useSelectedApplication();
  const apps = useRumApplications();
  const app = apps.data?.find((a) => a.id === selected);
  if (apps.isPending) return <Loading label="Loading applications…" />;
  if (!app) return <SelectApplicationHero />;
  return <Summary appId={app.id} appName={app.name} />;
}

function Summary({ appId, appName }: { appId: string; appName: string }) {
  const { range, relative } = useTime();
  const r = useMemo(() => ({ from: Math.floor(range.from / minute) * minute, to: Math.ceil(range.to / minute) * minute }), [range.from, range.to]);
  const q = useRumSummary({ range: r, applicationId: appId });
  const s = q.data;
  const option = useMemo<EChartsOption>(
    () => ({
      grid: { left: 44, right: 12, top: 12, bottom: 24 },
      tooltip: { trigger: 'axis' },
      xAxis: { type: 'time', min: r.from, max: r.to },
      yAxis: { type: 'value', minInterval: 1 },
      series: [{ name: 'Sessions', type: 'bar', itemStyle: { color: token('--chart-2') }, data: (s?.sessionsOverTime ?? []).map((b) => [b.t, b.count]) }],
    }),
    [s, r.from, r.to],
  );
  const explore = (q: string, type = 'view') => `/rum/explorer?application=${appId}&type=${type}${q ? `&q=${encodeURIComponent(q)}` : ''}`;
  return (
    <div className="rum-page">
      <div className="rum-toolbar">
        <span className="muted">
          <b>{appName}</b> · {relative.label}
        </span>
        <span className="syn-grow" />
        <TimePicker compact />
      </div>
      {q.isPending ? (
        <Loading label="Loading the summary…" />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : s!.sessions === 0 ? (
        <section className="rum-empty">
          <h3>No sessions in the {relative.label.toLowerCase()}</h3>
          <p className="muted">
            Once the SDK sends events, this page shows the application&rsquo;s performance, errors and usage. <Link to={`/rum/applications/${appId}`}>Check the SDK setup</Link>.
          </p>
        </section>
      ) : (
        <div className="rum-summary-grid">
          <section className="rum-card">
            <h3>Performance Monitoring</h3>
            <div className="rum-vitals">
              <Vital label="Loading Time p75" value={fmtMsShort(s!.loadingTimeP75)} g={grade('loading', s!.loadingTimeP75)} to={explore('')} />
              <Vital label="Largest Contentful Paint p75" value={fmtMsShort(s!.largestContentfulPaintP75)} g={grade('lcp', s!.largestContentfulPaintP75)} to={explore('')} />
              <Vital label="Interaction To Next Paint p75" value={fmtMsShort(s!.interactionToNextPaintP75)} g={grade('inp', s!.interactionToNextPaintP75)} to={explore('')} />
              <Vital
                label="Cumulative Layout Shift p75"
                value={s!.cumulativeLayoutShiftP75 == null ? '—' : s!.cumulativeLayoutShiftP75.toFixed(3)}
                g={grade('cls', s!.cumulativeLayoutShiftP75)}
                to={explore('')}
              />
            </div>
          </section>
          <section className="rum-card">
            <h3>Application Errors</h3>
            <div className="rum-big-pair">
              <Link to={explore('', 'error')}>
                <span>Errors</span>
                <b>{compact(s!.errors)}</b>
              </Link>
              <div>
                <span>Errors Rate</span>
                <b>{s!.errorRate.toFixed(s!.errorRate < 10 ? 1 : 0)}</b>
                <small className="muted">per minute</small>
              </div>
            </div>
            <Bars title="Top Views with Highest Error Rate" rows={s!.topErrorViews} value={(x) => x.value.toFixed(2)} tone="error" link={(x) => explore(`@view.name:${x.name}`, 'error')} />
          </section>
          <section className="rum-card">
            <h3>Track Deployments</h3>
            <Bars title="Sessions by version" head={['VERSION', 'ALL SESSIONS']} rows={s!.versions} value={(x) => compact(x.count)} link={(x) => explore(`version:${x.name}`, 'session')} />
          </section>
          <section className="rum-card">
            <h3>User Analytics</h3>
            <div className="rum-big-pair">
              <Link to={explore('', 'session')}>
                <span>Total Sessions</span>
                <b>{compact(s!.sessions)}</b>
              </Link>
              <div>
                <span>Crash-free Sessions</span>
                <b>{s!.crashFreeSessions == null ? '—' : `${s!.crashFreeSessions.toFixed(1)}%`}</b>
              </div>
            </div>
            <Bars title="Most Visited Pages" rows={s!.topViews} value={(x) => compact(x.count)} link={(x) => explore(`@view.name:${x.name}`)} />
          </section>
          <section className="rum-card rum-card-wide">
            <h3>Sessions</h3>
            <Chart option={option} height={180} />
          </section>
        </div>
      )}
    </div>
  );
}

function Vital({ label, value, g, to }: { label: string; value: string; g: string; to: string }) {
  return (
    <Link to={to} className={`rum-vital ${g}`}>
      <b>{value}</b>
      <span>{label}</span>
    </Link>
  );
}

function Bars({
  title,
  head,
  rows,
  value,
  tone,
  link,
}: {
  title: string;
  head?: [string, string];
  rows: RumNamedCount[];
  value: (x: RumNamedCount) => string;
  tone?: 'error';
  link: (x: RumNamedCount) => string;
}) {
  const max = Math.max(1, ...rows.map((x) => (tone === 'error' ? x.value : x.count)));
  return (
    <div className="rum-bars">
      <h4>{title}</h4>
      {head && (
        <div className="rum-bars-head">
          <span>{head[0]}</span>
          <span>{head[1]}</span>
        </div>
      )}
      {rows.length === 0 && <p className="muted">Nothing in this time range.</p>}
      {rows.map((x) => (
        <Link key={x.name} to={link(x)} className="rum-bar-row">
          <span className="rum-bar-value">{value(x)}</span>
          <span className="rum-bar-track">
            <i className={tone ?? ''} style={{ width: `${((tone === 'error' ? x.value : x.count) / max) * 100}%` }} />
            <span>{x.name}</span>
          </span>
        </Link>
      ))}
    </div>
  );
}
