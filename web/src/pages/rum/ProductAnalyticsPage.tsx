import { useMemo } from 'react';
import { NavLink, Outlet, useLocation } from 'react-router-dom';
import { useQuery, keepPreviousData } from '@tanstack/react-query';
import type { EChartsOption } from 'echarts';
import { Activity, BarChart3, ChartLine, ChevronDown, FlaskConical, Gauge, HeartPulse, MonitorSmartphone, MousePointerClick, Settings, ShieldCheck, Users, Waypoints } from 'lucide-react';
import { productAnalyticsReport, type AnalyticsItem, type AnalyticsPage, type AnalyticsReport, type AnalyticsValue } from '../../api/rum/client';
import { Chart } from '../../components/charts/Chart';
import { TimePicker } from '../../components/shell/PageHeader';
import { TimeProvider, useTime } from '../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { compact } from '../../lib/format';
import { token } from '../../theme/chartTheme';
import { fmtMsShort, useSelectedApplication } from './common';
import { useRumApplications } from './queries';
import { ProductAnalyticsIntro } from './ProductAnalyticsIntro';
import { ApplicationSelector } from './RumLayout';
import './rum.css';

const PULSE: [string, string, typeof Users][] = [
  ['users', 'Users', Users],
  ['devices', 'App & Devices', MonitorSmartphone],
  ['traffic', 'Traffic', Waypoints],
  ['engagement', 'Engagement', MousePointerClick],
  ['performance', 'Performance', Gauge],
];

/** Product Analytics: its own navigation (Product Pulse pages), the application and the time range. */
export function ProductAnalyticsLayout() {
  return (
    <TimeProvider initial="1mo">
      <div className="pa-layout">
        <nav className="pa-nav">
          <div className="pa-brand">
            <ChartLine size={20} /> Product Analytics
          </div>
          <NavLink to="/product-analytics" end className={({ isActive }) => (isActive ? 'active' : '')}>
            <BarChart3 size={16} /> Product Analytics
          </NavLink>
          <span className="pa-group">
            <HeartPulse size={16} /> Product Pulse <ChevronDown size={14} />
          </span>
          {PULSE.map(([to, label]) => (
            <NavLink key={to} to={`/product-analytics/${to}`} className={({ isActive }) => `pa-sub${isActive ? ' active' : ''}`}>
              {label}
            </NavLink>
          ))}
          <hr />
          <NavLink to="/product-analytics/experiments" className={({ isActive }) => (isActive ? 'active' : '')}>
            <FlaskConical size={16} /> Experiments
          </NavLink>
          <span className="pa-disabled" title="Not available yet">
            <Activity size={16} /> Metrics
          </span>
          <span className="pa-disabled" title="Not available yet">
            <ShieldCheck size={16} /> Protocols
          </span>
          <NavLink to="/rum/applications">
            <Settings size={16} /> Settings
          </NavLink>
        </nav>
        <div className="pa-main">
          <Outlet />
        </div>
      </div>
    </TimeProvider>
  );
}

const TITLES: Record<AnalyticsPage, [string, string]> = {
  home: ['Product Pulse', ''],
  users: ['Users', 'Who uses your product, where they are, and how usage changes over time'],
  devices: ['App & Devices', 'The versions, browsers and devices your users run'],
  traffic: ['Traffic', 'How users arrive on your product and what they look at'],
  engagement: ['Engagement', 'What users do and how often they come back'],
  performance: ['Performance', 'Where users run into errors, slowness and frustration'],
};

function usePage(): AnalyticsPage {
  const path = useLocation().pathname.split('/')[2] ?? '';
  return (['users', 'devices', 'traffic', 'engagement', 'performance'].includes(path) ? path : 'home') as AnalyticsPage;
}

/** A Product Pulse page: KPI cards, then the page's charts. */
export function ProductAnalyticsPage() {
  const page = usePage();
  const [app] = useSelectedApplication();
  const apps = useRumApplications();
  const { range } = useTime();
  const day = 86_400_000;
  const r = useMemo(() => ({ from: Math.floor(range.from / 60_000) * 60_000, to: Math.ceil(range.to / 60_000) * 60_000 }), [range.from, range.to]);
  const q = useQuery({
    queryKey: ['rum', 'analytics', page, app, r],
    queryFn: ({ signal }) => productAnalyticsReport({ range: r, page, applicationId: app }, signal),
    placeholderData: keepPreviousData,
  });
  const [title, subtitle] = TITLES[page];
  const appName = apps.data?.find((a) => a.id === app)?.name;
  const longRange = r.to - r.from >= 2 * day;
  return (
    <div className="pa-page">
      <header className="pa-top">
        <span className="pa-crumb">
          <HeartPulse size={16} /> {page === 'home' ? 'Home' : title}
        </span>
        <ApplicationSelector />
      </header>
      {apps.data?.length === 0 ? (
        <ProductAnalyticsIntro />
      ) : page === 'home' ? (
        <div className="pa-hello">
          <h2>Welcome to Product Analytics</h2>
          <p className="muted">{appName ? `How people use ${appName}` : 'How people use your applications'}, from the sessions RUM collects.</p>
        </div>
      ) : (
        <div className="pa-hello">
          <h2>{title}</h2>
          <p className="muted">{subtitle}</p>
        </div>
      )}
      {apps.data?.length !== 0 && (
        <>
          {page === 'home' && <h3 className="pa-section">Product Pulse</h3>}
          <div className="pa-filter">
            <span className="muted">{appName ?? 'All applications'}</span>
            <span className="syn-grow" />
            <TimePicker compact />
          </div>
          {q.isPending ? <Loading label="Loading…" /> : q.isError ? <ErrorState error={q.error} onRetry={() => q.refetch()} /> : <Body page={page} r={q.data} range={r} daily={longRange} />}
        </>
      )}
    </div>
  );
}

const pct = (v: number | null | undefined) => (v == null ? '—' : `${v < 10 ? v.toFixed(1) : Math.round(v)}%`);
const num = (v: number | null | undefined, digits = 2) => (v == null ? '—' : v >= 1000 ? compact(Math.round(v)) : Number(v.toFixed(digits)).toString());

function Kpi({ label, v, fmt, help, lowerIsBetter }: { label: string; v: AnalyticsValue; fmt: (x: number | null | undefined) => string; help?: string; lowerIsBetter?: boolean }) {
  const change = v.change;
  const good = change != null && (lowerIsBetter ? change < 0 : change > 0);
  const bad = change != null && (lowerIsBetter ? change > 0 : change < 0);
  return (
    <div className="pa-kpi" title={help}>
      <span>{label}</span>
      <b>{fmt(v.value)}</b>
      {change !== undefined && <em className={good ? 'up' : bad ? 'down' : ''}>{change == null ? '0% —' : `${change > 0 ? '+' : ''}${Math.round(change)}%`}</em>}
    </div>
  );
}

function Body({ page, r, range, daily }: { page: AnalyticsPage; r: AnalyticsReport; range: { from: number; to: number }; daily: boolean }) {
  if (!r.activeUsers.value) return <section className="rum-empty">No RUM sessions in this time range. Product Analytics reads the sessions of your RUM applications.</section>;
  const ms = (x: number | null | undefined) => fmtMsShort(x);
  const kpis: Record<AnalyticsPage, React.ReactNode> = {
    home: (
      <>
        <Kpi label="Active users" v={r.activeUsers} fmt={(x) => num(x, 0)} />
        <Kpi label="Load time" v={r.loadTime} fmt={ms} help="75th percentile of the views' loading time" lowerIsBetter />
        <Kpi label="Session duration" v={r.sessionDuration} fmt={ms} />
        <Kpi label="Bounce rate" v={r.bounceRate} fmt={pct} lowerIsBetter />
        <Kpi label="Sessions w/ errors" v={r.sessionsWithErrors} fmt={pct} lowerIsBetter />
      </>
    ),
    users: (
      <>
        <Kpi label="Total active users" v={{ ...r.activeUsers, change: undefined }} fmt={(x) => num(x, 0)} />
        <Kpi label="New users" v={{ ...r.newUsers, change: undefined }} fmt={(x) => num(x, 0)} help="First seen in this time range" />
        <Kpi label="Returning users" v={{ ...r.returningUsers, change: undefined }} fmt={(x) => num(x, 0)} help="Seen before this time range too" />
        <Kpi label="Average views per user" v={{ ...r.viewsPerUser, change: undefined }} fmt={(x) => num(x, 1)} />
      </>
    ),
    devices: (
      <>
        <Kpi label="Versions in use" v={{ ...r.versionsInUse, change: undefined }} fmt={(x) => num(x, 0)} />
        <Kpi label="Load time" v={{ ...r.loadTime, change: undefined }} fmt={ms} />
        <Kpi label="Desktop" v={{ ...r.desktop, change: undefined }} fmt={pct} />
        <Kpi label="Mobile & other devices" v={{ value: r.desktop.value == null ? null : 100 - r.desktop.value, change: undefined }} fmt={pct} />
      </>
    ),
    traffic: (
      <>
        <Kpi label="Page bounce rate" v={{ ...r.bounceRate, change: undefined }} fmt={pct} />
        <Kpi label="Avg. Time on page" v={{ ...r.timeOnPage, change: undefined }} fmt={ms} />
        <Kpi label="Avg. Pages per session" v={{ ...r.viewsPerSession, change: undefined }} fmt={(x) => num(x)} />
        <Kpi label="Total number of sessions" v={{ ...r.sessions, change: undefined }} fmt={(x) => num(x, 0)} />
      </>
    ),
    engagement: (
      <>
        <Kpi label="Session duration" v={{ ...r.sessionDuration, change: undefined }} fmt={ms} />
        <Kpi label="Sessions per user" v={{ ...r.sessionsPerUser, change: undefined }} fmt={(x) => num(x)} />
        <Kpi label="Page views per session" v={{ ...r.viewsPerSession, change: undefined }} fmt={(x) => num(x)} />
        <Kpi label="Users online" v={{ ...r.usersOnline, change: undefined }} fmt={(x) => num(x, 0)} help="Users with an event in the last 5 minutes" />
      </>
    ),
    performance: (
      <>
        <Kpi label="Sessions with errors" v={{ ...r.sessionsWithErrors, change: undefined }} fmt={pct} />
        <Kpi label="Load time" v={{ ...r.loadTime, change: undefined }} fmt={ms} />
        <Kpi label="Total issue count" v={{ ...r.issueCount, change: undefined }} fmt={(x) => num(x, 0)} />
        <Kpi label="Views with frustration" v={{ ...r.viewsWithFrustration, change: undefined }} fmt={pct} />
      </>
    ),
  };
  const over = (name: string, color: string) => <Bars r={r} range={range} daily={daily} name={name} color={color} />;
  return (
    <>
      <div className={`pa-kpis${page === 'home' ? ' home' : ''}`}>{kpis[page]}</div>
      {page === 'home' && <section className="pa-card pa-joined">{over('Active users over time', '--chart-1')}</section>}
      {page === 'users' && (
        <>
          <section className="pa-card">
            <h4>Active users trend</h4>
            {over('', '--chart-2')}
          </section>
          <div className="pa-two">
            <List title="Country breakdown" sub="Where your users come from." items={r.countries} />
            <List title="Power users" sub="By the number of views in the time range." items={r.powerUsers} />
          </div>
        </>
      )}
      {page === 'devices' && (
        <div className="pa-two">
          <section className="pa-card">
            <h4>User distribution by app version</h4>
            <Donut items={r.versions} />
          </section>
          <List title="Browser breakdown" items={r.browsers} strong />
        </div>
      )}
      {page === 'traffic' && (
        <>
          <section className="pa-card">
            <h4>Page views over time</h4>
            {over('', '--chart-2')}
          </section>
          <div className="pa-two">
            <List title="Traffic sources" items={r.sources} />
            <List title="Campaign sources" head={['@VIEW.URL_QUERY.UTM_SOURCE', 'COUNT OF VIEWS']} items={r.campaigns} />
          </div>
        </>
      )}
      {page === 'engagement' && (
        <div className="pa-two">
          <section className="pa-card">
            <h4>User retention curve</h4>
            <p className="muted">Overall retention of your product.</p>
            <p className="pa-nodata">(No data)</p>
          </section>
          <List title="Most popular views" sub="The views opened most in the time range." items={r.popularViews} />
        </div>
      )}
      {page === 'performance' && (
        <div className="pa-two">
          <section className="pa-card">
            <h4>Views with highest frustration count</h4>
            <p className="muted">On average</p>
            <Treemap items={r.frustrationByView} />
          </section>
          <section className="pa-card">
            <h4>Issue list</h4>
            <table className="mon-table syn-table pa-issues">
              <thead>
                <tr>
                  <th>ISSUE DETAILS</th>
                  <th className="syn-col-num">ERROR COUNT</th>
                </tr>
              </thead>
              <tbody>
                {r.issues.map((i) => (
                  <tr key={i.name}>
                    <td className="rum-msg">{i.name}</td>
                    <td className="syn-col-num">{compact(i.value)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            {!r.issues.length && <p className="muted">No errors in this time range.</p>}
          </section>
        </div>
      )}
    </>
  );
}

function Bars({ r, range, daily, name, color }: { r: AnalyticsReport; range: { from: number; to: number }; daily: boolean; name: string; color: string }) {
  const option = useMemo<EChartsOption>(
    () => ({
      grid: { left: 48, right: 12, top: name ? 34 : 12, bottom: 26 },
      title: name ? { text: name, left: 0, top: 0, textStyle: { fontSize: 13, fontWeight: 'normal' } } : undefined,
      tooltip: { trigger: 'axis' },
      xAxis: daily
        ? { type: 'category', data: r.overTime.map((b) => new Date(b.t).toLocaleDateString(undefined, { month: 'short', day: 'numeric' })) }
        : { type: 'time', min: range.from, max: range.to },
      yAxis: { type: 'value', minInterval: 1 },
      series: [{ type: 'bar', barMaxWidth: 48, itemStyle: { color: token(color) }, data: daily ? r.overTime.map((b) => b.count) : r.overTime.map((b) => [b.t, b.count]) }],
    }),
    [r, range.from, range.to, daily, name, color],
  );
  return <Chart option={option} height={300} />;
}

function List({ title, sub, head, items, strong }: { title: string; sub?: string; head?: [string, string]; items: AnalyticsItem[]; strong?: boolean }) {
  const max = Math.max(1, ...items.map((i) => i.value));
  return (
    <section className="pa-card">
      <h4>{title}</h4>
      {sub && <p className="muted">{sub}</p>}
      {head && (
        <div className="rum-bars-head">
          <span>{head[0]}</span>
          <span>{head[1]}</span>
        </div>
      )}
      <div className="rum-bars">
        {items.map((i) => (
          <div key={i.name} className="rum-bar-row pa-bar">
            <span className="rum-bar-value">{compact(Math.round(i.value))}</span>
            <span className="rum-bar-track">
              <i className={strong ? 'strong' : 'violet'} style={{ width: `${(i.value / max) * 100}%` }} />
              <span>{i.name}</span>
            </span>
          </div>
        ))}
        {!items.length && <p className="muted">No data in this time range.</p>}
      </div>
    </section>
  );
}

function Donut({ items }: { items: AnalyticsItem[] }) {
  const total = items.reduce((a, b) => a + b.value, 0);
  const option = useMemo<EChartsOption>(
    () => ({
      tooltip: { trigger: 'item' },
      title: { text: compact(total), subtext: 'users', left: 'center', top: 'center', textStyle: { fontSize: 22 } },
      series: [
        {
          type: 'pie',
          radius: ['48%', '75%'],
          label: { formatter: (p) => `${p.name}\n${compact(Number(p.value))} users` },
          data: items.map((i, n) => ({ name: i.name, value: i.value, itemStyle: { color: token(`--chart-${(n % 8) + 1}`) } })),
        },
      ],
    }),
    [items, total],
  );
  return items.length ? <Chart option={option} height={320} /> : <p className="pa-nodata">(No data)</p>;
}

function Treemap({ items }: { items: AnalyticsItem[] }) {
  const option = useMemo<EChartsOption>(
    () => ({
      tooltip: { valueFormatter: (v) => Number(v).toFixed(2) },
      series: [
        {
          type: 'treemap',
          roam: false,
          nodeClick: false,
          breadcrumb: { show: false },
          label: { formatter: (p) => `${p.name}\n${Number(p.value).toFixed(2)}` },
          data: items.map((i, n) => ({ name: i.name, value: i.value, itemStyle: { color: token(`--chart-${(n % 8) + 1}`) } })),
        },
      ],
    }),
    [items],
  );
  return items.length ? <Chart option={option} height={340} /> : <p className="pa-nodata">No frustration signals in this time range.</p>;
}
