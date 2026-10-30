import { useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import type { EChartsOption } from 'echarts';
import { Link, useNavigate } from 'react-router-dom';
import { AlertTriangle, ArrowDown, ArrowRight, Braces, Bug, Database, GitCommitHorizontal, Hexagon, ListTree, ScrollText, Search, Server, Share2, ShieldCheck, Star, Target } from 'lucide-react';
import type { Dependency, ExplorerSpan, ServiceScope, ServiceSummary, SpanQuery } from '../../../api/apm/client';
import { queryTimeseries } from '../../../api/dashboards/client';
import { Select } from '../../../components/ui/Dropdown';
import { Chart } from '../../../components/charts/Chart';
import { useTime } from '../../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { CollapsibleSection } from '../../../components/ui/CollapsibleSection';
import { compact, formatBytes, formatLogDate, hhmm, timeAgo } from '../../../lib/format';
import { STATUS_COLORS, statusColor, token } from '../../../theme/chartTheme';
import { useDatabases, useDependencies, useDeployments, useExploreSpans, useResources, useServiceLogs, useSpanAnalytics, useSpanErrors } from '../queries';
import { errorTone, fmtLatency, fmtPct, fmtRate, KIND_LABEL, LineSpark, ServiceIcon, useStars } from '../visuals';
import { DependencyMap } from './DependencyMap';

function Pending({ q }: { q: { isPending: boolean; isError: boolean; error: unknown; refetch: () => unknown } }) {
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  return null;
}

/* ---------- Service Summary ---------- */

const LABEL_STEPS_MIN = [1, 2, 5, 10, 15, 30, 60, 120, 180, 360, 720, 1440];

/** An axis label formatter that prints only times on the step (local minutes), blank otherwise. */
function roundLabels(stepMs: number): (v: number) => string {
  const step = stepMs / 60e3;
  return (v) => {
    const d = new Date(v);
    return (d.getHours() * 60 + d.getMinutes()) % step === 0 && d.getSeconds() === 0 ? hhmm(v) : '';
  };
}

/** A round label step (ms) giving about five labels over the series' span. */
function labelStep(series: ServiceSummary['series']): number {
  const span = series.length > 1 ? series[series.length - 1].t - series[0].t : 3600e3;
  const minutes = LABEL_STEPS_MIN.find((m) => span / (m * 60e3) <= 6) ?? 1440;
  return minutes * 60e3;
}

function timeChart(summary: ServiceSummary, kind: 'requests' | 'errors' | 'latency'): EChartsOption {
  const s = summary.series;
  const base: EChartsOption = {
    grid: { left: 58, right: 10, top: 12, bottom: 44 },
    tooltip: { trigger: 'axis' },
    legend: { bottom: 0, left: 0, textStyle: { fontSize: 11 } },
    // About five labels whatever the range: ECharts packs line charts' ticks tighter than bars', so
    // only round times are labelled.
    xAxis: { type: 'time', axisLabel: { formatter: roundLabels(labelStep(s)), hideOverlap: true } },
  };
  if (kind === 'latency') {
    return {
      ...base,
      color: [token('--chart-1'), token('--chart-3'), token('--chart-5')],
      yAxis: { type: 'value', axisLabel: { formatter: (v: number) => fmtLatency(v) } },
      tooltip: { trigger: 'axis', valueFormatter: (v) => fmtLatency(Number(v)) },
      series: (['p50Ms', 'p95Ms', 'p99Ms'] as const).map((k) => ({
        name: k.slice(0, 3),
        type: 'line',
        showSymbol: false,
        lineStyle: { width: 1.4 },
        data: s.map((p) => [p.t, p[k]]),
      })),
    };
  }
  if (kind === 'errors') {
    return {
      ...base,
      color: [statusColor('error')],
      yAxis: { type: 'value', minInterval: 1 },
      series: [{ name: 'Errors', type: 'bar', barMaxWidth: 10, data: s.map((p) => [p.t, p.errors]) }],
    };
  }
  return {
    ...base,
    color: [token('--chart-1'), statusColor('error')],
    yAxis: { type: 'value', minInterval: 1 },
    series: [
      { name: 'Requests', type: 'bar', stack: 'r', barMaxWidth: 10, data: s.map((p) => [p.t, p.hits - p.errors]) },
      { name: 'Errors', type: 'bar', stack: 'r', barMaxWidth: 10, data: s.map((p) => [p.t, p.errors]) },
    ],
  };
}

/** Monitors, SLOs and incidents naming the service (its Software Catalog entity), when known. */
export interface Reliability {
  monitors: number;
  monitorsAlerting: number;
  slos: number;
  incidents: number;
  openIncidents: number;
}

export function SummarySection({ summary, reliability }: { summary: ServiceSummary; reliability: Reliability | null }) {
  const r = reliability;
  const req = useMemo(() => timeChart(summary, 'requests'), [summary]);
  const err = useMemo(() => timeChart(summary, 'errors'), [summary]);
  const lat = useMemo(() => timeChart(summary, 'latency'), [summary]);
  const bucket = summary.bucketMs >= 3600e3 ? `${summary.bucketMs / 3600e3}h` : `${Math.round(summary.bucketMs / 60e3)}m`;
  return (
    <CollapsibleSection id="summary" title="Service Summary" icon={Hexagon}>
      <div className="svc-cards">
        <div className="svc-card">
          <div className="svc-card-title">
            <AlertTriangle size={14} /> Incidents
          </div>
          {r?.incidents ? (
            <Link className={`svc-card-value${r.openIncidents ? ' tone-error' : ''}`} to={`/incidents?query=${encodeURIComponent(`service:${summary.service}`)}`}>
              {r.openIncidents ? `${r.openIncidents} open` : `${r.incidents} resolved`}
            </Link>
          ) : (
            <div className="svc-card-value muted">None</div>
          )}
        </div>
        <div className="svc-card">
          <div className="svc-card-title">
            <Target size={14} /> Health Monitors
          </div>
          {r?.monitors ? (
            <Link className={`svc-card-value${r.monitorsAlerting ? ' tone-error' : ''}`} to={`/monitors?query=${encodeURIComponent(`service:${summary.service}`)}`}>
              {r.monitorsAlerting ? `${r.monitorsAlerting} alerting` : `${r.monitors} OK`}
            </Link>
          ) : (
            <div className="svc-card-value muted">No monitors</div>
          )}
        </div>
        <div className="svc-card">
          <div className="svc-card-title">
            <ShieldCheck size={14} /> SLOs
          </div>
          {r?.slos ? (
            <Link className="svc-card-value" to={`/slo?query=${encodeURIComponent(`service:${summary.service}`)}`}>
              {r.slos}
            </Link>
          ) : (
            <div className="svc-card-value muted">None</div>
          )}
        </div>
        <div className="svc-card">
          <div className="svc-card-title">Requests</div>
          <div className="svc-card-value">{fmtRate(summary.reqPerSec)}</div>
          <div className="muted">{compact(summary.requests)} total</div>
        </div>
        <div className="svc-card">
          <div className="svc-card-title">Error rate</div>
          <div className={`svc-card-value ${errorTone(summary.errorRate)}`}>{fmtPct(summary.errorRate)}</div>
          <div className="muted">{compact(summary.errors)} errors</div>
        </div>
        <div className="svc-card">
          <div className="svc-card-title">p95 latency</div>
          <div className="svc-card-value">{fmtLatency(summary.p95Ms)}</div>
        </div>
      </div>
      <div className="svc-charts">
        <div>
          <div className="svc-chart-title">
            Requests and Errors <span className="muted">per {bucket}</span>
          </div>
          <Chart option={req} height={200} />
        </div>
        <div>
          <div className="svc-chart-title">
            Errors <span className="muted">per {bucket}</span>
          </div>
          <Chart option={err} height={200} />
        </div>
        <div>
          <div className="svc-chart-title">Latency</div>
          <Chart option={lat} height={200} />
        </div>
      </div>
    </CollapsibleSection>
  );
}

/* ---------- Resources ---------- */

type ResMeasure = 'count' | 'p95' | 'errors';
const RES_MEASURES: { id: ResMeasure; label: string }[] = [
  { id: 'count', label: 'Requests' },
  { id: 'p95', label: 'p95 Latency' },
  { id: 'errors', label: 'Errors' },
];

/** The service's entry spans as a span query (env, resource and version from the page's filters). */
function entryQuery(scope: ServiceScope, extra = ''): SpanQuery {
  const terms = [`service:${scope.service}`, scope.env && `env:${scope.env}`, scope.version && `version:${scope.version}`, extra].filter(Boolean);
  return { range: scope.range, scope: 'service_entry', query: terms.join(' '), filters: {} };
}

/** One Top 5 chart of the resources: requests and errors as stacked bars, latency as lines. */
function ResourceChart({ scope, measure }: { scope: ServiceScope; measure: ResMeasure }) {
  const q = useSpanAnalytics(entryQuery(scope), 'resource', measure, 5, true);
  const option = useMemo<EChartsOption | null>(() => {
    const a = q.data;
    if (!a) return null;
    const latency = measure === 'p95';
    return {
      grid: { left: 52, right: 8, top: 10, bottom: 46 },
      tooltip: { trigger: 'axis', confine: true, valueFormatter: (v) => (latency ? fmtLatency(Number(v)) : compact(Number(v))) },
      legend: { bottom: 0, left: 0, type: 'scroll', textStyle: { fontSize: 11 } },
      xAxis: { type: 'time', axisLabel: { formatter: (v: number) => hhmm(v), hideOverlap: true } },
      yAxis: { type: 'value', minInterval: latency ? undefined : 1, axisLabel: latency ? { formatter: (v: number) => fmtLatency(v) } : undefined },
      series: a.series.map((g) => ({
        name: g.group || '(none)',
        type: latency ? 'line' : 'bar',
        stack: latency ? undefined : 'r',
        showSymbol: false,
        barMaxWidth: 10,
        data: a.times.map((t, k) => [t, g.values[k] ?? 0]),
      })),
    };
  }, [q.data, measure]);
  return (
    <div className="svc-res-chart">
      <div className="svc-chart-title">
        {RES_MEASURES.find((m) => m.id === measure)!.label} <span className="muted">Top 5</span>
      </div>
      {option ? <Chart option={option} height={210} /> : <Pending q={q} />}
    </div>
  );
}

type ResSort = 'requests' | 'totalMs' | 'p95Ms' | 'errors' | 'errorRate';

export function ResourcesSection({ scope, onPick }: { scope: ServiceScope; onPick: (resource: string) => void }) {
  const q = useResources({ range: scope.range, env: scope.env, service: scope.service });
  const [search, setSearch] = useState('');
  const [sort, setSort] = useState<ResSort>('totalMs');
  const [page, setPage] = useState(0);
  const { stars: starred, toggle: toggleStar } = useStars('oc.apm.starred-resources');
  const rows = useMemo(() => {
    const f = search.trim().toLowerCase();
    return (q.data ?? []).filter((r) => !f || r.resource.toLowerCase().includes(f)).sort((a, b) => Number(starred.has(b.resource)) - Number(starred.has(a.resource)) || b[sort] - a[sort]);
  }, [q.data, search, sort, starred]);
  const max = (k: ResSort) => Math.max(...rows.map((r) => r[k]), 1e-9);
  const shown = rows.slice(page * 10, page * 10 + 10);
  const head = (k: ResSort, label: string) => (
    <th className="num sortable" onClick={() => setSort(k)}>
      {sort === k && <ArrowDown size={12} />} {label}
    </th>
  );
  const bar = (v: number, k: ResSort, tone = '') => (
    <span className="svc-cell-bar">
      {k === 'totalMs' || k === 'p95Ms' ? fmtLatency(v) : k === 'errorRate' ? fmtPct(v) : compact(v)}
      <i className={tone}>
        <b style={{ width: `${Math.min(100, (v / max(k)) * 100)}%` }} />
      </i>
    </span>
  );
  return (
    <CollapsibleSection id="resources" title="Resources" icon={Braces}>
      <div className="svc-res-charts">
        {RES_MEASURES.map((m) => (
          <ResourceChart key={m.id} scope={scope} measure={m.id} />
        ))}
      </div>
      <Pending q={q} />
      {q.data &&
        (q.data.length === 0 ? (
          <div className="hp-empty">No resources in this time range</div>
        ) : (
          <>
            <div className="svc-res-bar">
              <span>
                Showing{' '}
                <b>
                  {rows.length ? page * 10 + 1 : 0}–{page * 10 + shown.length}
                </b>{' '}
                of <b>{rows.length}</b>
              </span>
              <label className="apm-search">
                <Search size={14} />
                <input placeholder="Search Resources" value={search} onChange={(e) => (setSearch(e.target.value), setPage(0))} />
              </label>
            </div>
            <table className="apm-table svc-res-table">
              <thead>
                <tr>
                  <th className="col-star" />
                  <th>RESOURCE NAME</th>
                  {head('requests', 'REQUESTS')}
                  {head('totalMs', 'TOTAL TIME')}
                  {head('p95Ms', 'P95 LATENCY')}
                  {head('errors', 'ERRORS')}
                  {head('errorRate', 'ERROR RATE')}
                </tr>
              </thead>
              <tbody>
                {shown.map((r) => (
                  <tr key={r.resource} className={scope.resource === r.resource ? 'selected' : ''} onClick={() => onPick(r.resource)}>
                    <td className="col-star">
                      <button
                        className={`icon-plain${starred.has(r.resource) ? ' starred' : ''}`}
                        onClick={(e) => (e.stopPropagation(), toggleStar(r.resource))}
                        aria-label={starred.has(r.resource) ? 'Unstar' : 'Star'}
                      >
                        <Star size={15} />
                      </button>
                    </td>
                    <td className="mono ellipsis svc-res-name" title={r.resource}>
                      <ResourceName name={r.resource} />
                    </td>
                    <td className="num">{bar(r.requests, 'requests')}</td>
                    <td className="num">{bar(r.totalMs, 'totalMs')}</td>
                    <td className="num">{bar(r.p95Ms, 'p95Ms')}</td>
                    <td className="num">{bar(r.errors, 'errors', 'err')}</td>
                    <td className={`num ${errorTone(r.errorRate)}`}>{bar(r.errorRate, 'errorRate', 'err')}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            {rows.length > 10 && (
              <div className="svc-pager">
                <button className="btn btn-sm" disabled={page === 0} onClick={() => setPage((p) => p - 1)}>
                  Previous
                </button>
                <button className="btn btn-sm" disabled={(page + 1) * 10 >= rows.length} onClick={() => setPage((p) => p + 1)}>
                  Next
                </button>
              </div>
            )}
          </>
        ))}
    </CollapsibleSection>
  );
}

const SQL_KEYWORDS = /^(select|insert|update|delete|with|from|where|into|set|values|join|on|and|or|top|distinct|as|order|by|group|limit|returning|begin|commit)$/i;

/** SQL statements and HTTP routes get their keywords / method highlighted. */
function ResourceName({ name }: { name: string }) {
  const http = /^([A-Z]+)\s+(\/.*)$/.exec(name);
  if (http)
    return (
      <>
        <span className="svc-kw">{http[1]}</span> {http[2]}
      </>
    );
  if (!/^\s*(select|insert|update|delete|with)\b/i.test(name)) return <>{name}</>;
  return (
    <>
      {name.split(/(\s+)/).map((w, k) =>
        SQL_KEYWORDS.test(w) ? (
          <span key={k} className="svc-kw">
            {w}
          </span>
        ) : (
          w
        ),
      )}
    </>
  );
}

/* ---------- Deployments ---------- */

export function DeploymentsSection({ scope, onPick }: { scope: ServiceScope; onPick: (version: string) => void }) {
  const q = useDeployments(scope);
  return (
    <CollapsibleSection id="deployments" title="Deployments" icon={GitCommitHorizontal}>
      <Pending q={q} />
      {q.data &&
        (q.data.length === 0 ? (
          <div className="svc-empty">
            <GitCommitHorizontal size={44} />
            <h3>Set up version tagging to track deployments</h3>
            <p>
              With a version on each service (<code>DD_VERSION</code>, or <code>service.version</code> in OpenTelemetry) you can compare request volumes, error rates and latency between deployments,
              for each endpoint.
            </p>
            <Link className="btn btn-primary" to="/apm/instrument">
              Get Started <ArrowRight size={14} />
            </Link>
          </div>
        ) : (
          <table className="apm-table">
            <thead>
              <tr>
                <th>VERSION</th>
                <th>FIRST SEEN</th>
                <th>LAST SEEN</th>
                <th className="num">REQUESTS</th>
                <th className="num">ERROR RATE</th>
                <th className="num">P95 LATENCY</th>
              </tr>
            </thead>
            <tbody>
              {q.data.map((d) => (
                <tr key={d.version} className={scope.version === d.version ? 'selected' : ''} onClick={() => onPick(d.version)}>
                  <td>
                    <span className="tag-chip">{d.version}</span>
                  </td>
                  <td className="muted">{timeAgo(d.firstSeen)}</td>
                  <td className="muted">{timeAgo(d.lastSeen)}</td>
                  <td className="num">{compact(d.requests)}</td>
                  <td className={`num ${errorTone(d.errorRate)}`}>{fmtPct(d.errorRate)}</td>
                  <td className="num">{fmtLatency(d.p95Ms)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        ))}
    </CollapsibleSection>
  );
}

/* ---------- Dependencies ---------- */

function DependencyTable({ title, rows, grouped }: { title: string; rows: Dependency[]; grouped: boolean }) {
  const navigate = useNavigate();
  const groups = useMemo(() => {
    if (!grouped) return [{ key: '', rows }];
    const m = new Map<string, Dependency[]>();
    for (const r of rows) m.set(r.kind, [...(m.get(r.kind) ?? []), r]);
    return [...m].map(([key, rs]) => ({ key, rows: rs }));
  }, [rows, grouped]);
  return (
    <div className="dep-table">
      <h4>
        {title} <span className="muted">({rows.length})</span>
      </h4>
      {rows.length === 0 ? (
        <div className="hp-empty">None</div>
      ) : (
        <table className="apm-table">
          <thead>
            <tr>
              <th>{title === 'Upstream' ? 'UPSTREAM SERVICE' : 'DOWNSTREAM SERVICE'}</th>
              <th className="num">REQUESTS</th>
              <th className="num">ERROR</th>
              <th />
              <th className="num">P95 LATENCY</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {groups.map((g) => (
              <GroupRows key={g.key} label={g.key ? KIND_LABEL[g.key as Dependency['kind']] : ''} rows={g.rows} onOpen={(s) => navigate(`/apm/services/${encodeURIComponent(s)}`)} />
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

function GroupRows({ label, rows, onOpen }: { label: string; rows: Dependency[]; onOpen: (service: string) => void }) {
  return (
    <>
      {label && (
        <tr className="group-row">
          <td colSpan={6}>
            {label} <span className="muted">({rows.length})</span>
          </td>
        </tr>
      )}
      {rows.map((d) => (
        <tr key={d.service} onClick={() => onOpen(d.service)}>
          <td>
            <span className="svc-name">
              <ServiceIcon name={d.service} kind={d.kind} language={d.language} technology={d.technology} />
              {d.service}
            </span>
          </td>
          <td className="num">{fmtRate(d.reqPerSec)}</td>
          <td className={`num ${errorTone(d.errorRate)}`}>{fmtPct(d.errorRate)}</td>
          <td className="spark-cell">
            <LineSpark values={d.errorSeries} tone="error" />
          </td>
          <td className="num">{fmtLatency(d.p95Ms)}</td>
          <td className="spark-cell">
            <LineSpark values={d.p95Series} />
          </td>
        </tr>
      ))}
    </>
  );
}

export function DependenciesSection({ scope }: { scope: ServiceScope }) {
  const q = useDependencies(scope);
  const [grouped, setGrouped] = useState(false);
  return (
    <CollapsibleSection
      id="dependencies"
      title="Dependencies"
      icon={Share2}
      extra={
        <label className="svc-switch">
          <input type="checkbox" role="switch" checked={grouped} onChange={(e) => setGrouped(e.target.checked)} /> Grouped
        </label>
      }
    >
      <Pending q={q} />
      {q.data && (
        <div className="svc-deps">
          <div className="dep-tables">
            <DependencyTable title="Upstream" rows={q.data.upstream} grouped={grouped} />
            <DependencyTable title="Downstream" rows={q.data.downstream} grouped={grouped} />
          </div>
          <DependencyMap service={scope.service} deps={q.data} />
        </div>
      )}
    </CollapsibleSection>
  );
}

/* ---------- Traces ---------- */

export function TracesSection({ scope, onOpen }: { scope: ServiceScope; onOpen: (traceId: string) => void }) {
  const navigate = useNavigate();
  const [text, setText] = useState('');
  const [log, setLog] = useState(true);
  const q = entryQuery(scope, [scope.resource && `resource:"${scope.resource.replace(/"/g, '')}"`, text.trim()].filter(Boolean).join(' '));
  q.scope = 'all';
  const sample = useExploreSpans(q, 'date', 0, 500);
  const spans = sample.data?.spans ?? [];
  const option = useMemo<EChartsOption>(() => {
    const ok = spans.filter((x) => x.status !== 'error' && x.status !== 'Error');
    const err = spans.filter((x) => x.status === 'error' || x.status === 'Error');
    const point = (x: ExplorerSpan) => [x.timestamp, Math.max(x.durationMs / 1000, 1e-4), x.traceId];
    return {
      grid: { left: 56, right: 12, top: 10, bottom: 46 },
      tooltip: { trigger: 'item', formatter: (p) => `${fmtLatency(Number((p as { value: number[] }).value[1]) * 1000)}` },
      legend: { bottom: 0, left: 0, textStyle: { fontSize: 11 } },
      xAxis: { type: 'time', axisLabel: { formatter: (v: number) => hhmm(v), hideOverlap: true } },
      yAxis: {
        type: (log ? 'log' : 'value') as 'value',
        name: 'Duration (s)',
        nameLocation: 'middle',
        nameGap: 42,
        nameTextStyle: { fontSize: 11 },
        axisLabel: { formatter: (v: number) => (v < 0.01 ? v.toExponential(0) : String(v)) },
      },
      series: [
        { name: 'ok', type: 'scatter', symbolSize: 7, itemStyle: { color: statusColor('ok'), opacity: 0.55 }, data: ok.map(point) },
        { name: 'error', type: 'scatter', symbolSize: 7, itemStyle: { color: statusColor('error'), opacity: 0.7 }, data: err.map(point) },
      ],
    };
  }, [spans, log]);
  const explorer = `/apm/traces?query=${encodeURIComponent(q.query ?? '')}`;
  return (
    <CollapsibleSection id="traces" title="Traces" icon={ListTree}>
      <div className="svc-query-row">
        <label className="apm-search">
          <Search size={14} />
          <input placeholder="Enter your query" value={text} onChange={(e) => setText(e.target.value)} />
        </label>
        <button className="btn" onClick={() => navigate(explorer)}>
          <ListTree size={14} /> Trace Explorer
        </button>
      </div>
      <Pending q={sample} />
      {sample.data && (
        <>
          <div className="svc-scatter-head">
            <span>
              Showing <b>{Math.min(25, spans.length)}</b> of <b>{compact(sample.data.total)}</b> spans
            </span>
            <span className="svc-scatter-title">Span durations</span>
            <div className="apm-seg">
              <button className={log ? '' : 'active'} onClick={() => setLog(false)}>
                Linear
              </button>
              <button className={log ? 'active' : ''} onClick={() => setLog(true)}>
                Log
              </button>
            </div>
          </div>
          <Chart option={option} height={260} onClick={(p) => Array.isArray(p.value) && onOpen(String(p.value[2]))} />
          <ExplorerSpanTable spans={spans.slice(0, 25)} onOpen={onOpen} />
        </>
      )}
    </CollapsibleSection>
  );
}

function ExplorerSpanTable({ spans, onOpen }: { spans: ExplorerSpan[]; onOpen: (traceId: string) => void }) {
  if (spans.length === 0) return <div className="hp-empty">No spans match</div>;
  return (
    <table className="apm-table">
      <thead>
        <tr>
          <th className="col-status" />
          <th>DATE</th>
          <th>RESOURCE</th>
          <th>SERVICE</th>
          <th className="num">DURATION</th>
          <th>METHOD</th>
          <th>STATUS CODE</th>
        </tr>
      </thead>
      <tbody>
        {spans.map((x) => (
          <tr key={x.spanId} onClick={() => onOpen(x.traceId)}>
            <td className="col-status">
              <i className={`status-bar${x.status === 'error' || x.status === 'Error' ? ' err' : ''}`} />
            </td>
            <td className="mono nowrap">{formatLogDate(x.timestamp)}</td>
            <td className="mono ellipsis svc-res-name" title={x.resource}>
              {x.resource}
            </td>
            <td>
              <span className="svc-name">
                <ServiceIcon name={x.service} kind={x.kind === 'client' && x.dbSystem ? 'db' : 'web'} technology={x.dbSystem} size={16} />
                {x.service}
                {x.peerService && (
                  <>
                    <span className="muted">→</span>
                    <ServiceIcon name={x.peerService} kind={x.dbSystem ? 'db' : 'web'} technology={x.dbSystem} size={16} />
                    {x.peerService}
                  </>
                )}
              </span>
            </td>
            <td className="num">{fmtLatency(x.durationMs)}</td>
            <td>{x.method || ''}</td>
            <td className={x.httpStatus?.startsWith('5') ? 'tone-error' : x.httpStatus?.startsWith('4') ? 'tone-warn' : ''}>{x.httpStatus || ''}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

export function SpanTable({
  spans,
  onOpen,
  showService = true,
}: {
  spans: { traceId: string; spanId: string; timestamp: number; service: string; resource: string; durationMs: number; status: string; httpStatus: string; env: string }[];
  onOpen: (traceId: string) => void;
  showService?: boolean;
}) {
  if (spans.length === 0) return <div className="hp-empty">No traces match</div>;
  return (
    <table className="apm-table">
      <thead>
        <tr>
          <th className="col-status" />
          <th>DATE</th>
          {showService && <th>SERVICE</th>}
          <th>RESOURCE</th>
          <th className="num">DURATION</th>
          <th>HTTP</th>
          <th>ENV</th>
        </tr>
      </thead>
      <tbody>
        {spans.map((s) => (
          <tr key={s.spanId} onClick={() => onOpen(s.traceId)}>
            <td className="col-status">
              <i className={`status-bar${s.status === 'Error' ? ' err' : ''}`} />
            </td>
            <td className="mono muted">{formatLogDate(s.timestamp)}</td>
            {showService && <td>{s.service}</td>}
            <td className="mono ellipsis">{s.resource}</td>
            <td className="num">{fmtLatency(s.durationMs)}</td>
            <td className={s.httpStatus.startsWith('5') ? 'tone-error' : s.httpStatus.startsWith('4') ? 'tone-warn' : ''}>{s.httpStatus || '—'}</td>
            <td className="muted">{s.env || '—'}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/* ---------- Errors ---------- */

export function ErrorsSection({ scope, onOpen }: { scope: ServiceScope; onOpen: (traceId: string) => void }) {
  const navigate = useNavigate();
  const q = useSpanErrors({ range: scope.range, env: scope.env, service: scope.service });
  const [text, setText] = useState('');
  const [sort, setSort] = useState<'newest' | 'count'>('newest');
  const f = text.trim().toLowerCase();
  const issues = (q.data ?? [])
    .filter((e) => !f || `${e.errorType} ${e.message} ${e.resource}`.toLowerCase().includes(f))
    .sort((a, b) => (sort === 'newest' ? b.firstSeen - a.firstSeen : b.count - a.count));
  return (
    <CollapsibleSection id="errors" title="Errors" icon={Bug}>
      <div className="svc-query-row">
        <label className="apm-search">
          <Search size={14} />
          <input placeholder="Search Issues" value={text} onChange={(e) => setText(e.target.value)} />
        </label>
        <Select<'newest' | 'count'>
          label="Sort by"
          value={sort}
          onChange={setSort}
          options={[
            { value: 'newest', label: 'Newest issues' },
            { value: 'count', label: 'Most occurrences' },
          ]}
        />
        <button className="btn" onClick={() => navigate(`/errors?query=${encodeURIComponent(`service:${scope.service}`)}`)}>
          <Bug size={14} /> Error Tracking
        </button>
      </div>
      <Pending q={q} />
      {q.data && (
        <p className="svc-count">
          Showing <b>{issues.length}</b> issues to review
        </p>
      )}
      {q.data &&
        (issues.length === 0 ? (
          <div className="svc-empty">
            <Bug size={44} />
            <h3>No issues found</h3>
            <p>To view past issues, adjust the time frame.</p>
          </div>
        ) : (
          <table className="apm-table">
            <thead>
              <tr>
                <th>ERROR</th>
                <th>RESOURCE</th>
                <th className="num">COUNT</th>
                <th>FIRST SEEN</th>
                <th>LAST SEEN</th>
              </tr>
            </thead>
            <tbody>
              {issues.map((e) => (
                <tr key={`${e.resource}:${e.errorType}:${e.message}`} onClick={() => onOpen(e.sampleTraceId)} title="Open a sample trace">
                  <td>
                    <div className="mono tone-error">{e.errorType || 'Error'}</div>
                    <div className="muted ellipsis err-msg">{e.message || '(no message)'}</div>
                  </td>
                  <td className="mono">{e.resource}</td>
                  <td className="num">{compact(e.count)}</td>
                  <td className="muted">{timeAgo(e.firstSeen)}</td>
                  <td className="muted">{timeAgo(e.lastSeen)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        ))}
    </CollapsibleSection>
  );
}

/* ---------- Infrastructure ---------- */

type HostUnit = 'number' | 'percent' | 'fraction' | 'bytes' | 'bytesPerSec';

interface HostVariant {
  title: string;
  queries: (h: string) => string[];
  unit: HostUnit;
  stacked?: boolean;
}

/** Host charts in Datadog Agent metric names, with the OpenTelemetry hostmetrics equivalent tried when the Agent's are absent. */
const HOST_CHARTS: { id: string; variants: HostVariant[] }[] = [
  {
    id: 'queue',
    variants: [
      { title: 'Process Queue Length', queries: (h) => [`avg:system.proc.queue_length{${h}}`], unit: 'number' },
      { title: 'Load average (1m)', queries: (h) => [`avg:system.cpu.load_average.1m{${h}}`], unit: 'number' },
    ],
  },
  { id: 'disk-queue', variants: [{ title: 'Disk Queue Length', queries: (h) => [`avg:system.io.avg_q_sz{${h}}`], unit: 'number' }] },
  {
    id: 'cpu',
    variants: [
      { title: 'CPU usage (%)', queries: (h) => [`avg:system.cpu.user{${h}}`, `avg:system.cpu.system{${h}}`, `avg:system.cpu.iowait{${h}}`], unit: 'percent', stacked: true },
      { title: 'CPU usage (%)', queries: (h) => [`avg:system.cpu.utilization{${h},!state:idle} by {state}`], unit: 'fraction', stacked: true },
    ],
  },
  {
    id: 'memory',
    variants: [
      // The Agent reports memory in MiB.
      { title: 'Memory breakdown (MiB)', queries: (h) => [`sum:system.mem.used{${h}}`, `sum:system.mem.total{${h}}`], unit: 'number' },
      { title: 'Memory breakdown', queries: (h) => [`sum:system.memory.usage{${h}} by {state}`], unit: 'bytes', stacked: true },
    ],
  },
  {
    id: 'network',
    variants: [
      { title: 'Network traffic (bytes per sec)', queries: (h) => [`sum:system.net.bytes_rcvd{${h}}`, `sum:system.net.bytes_sent{${h}}`], unit: 'bytesPerSec' },
      { title: 'Network traffic (bytes per sec)', queries: (h) => [`sum:system.network.io{${h}} by {direction}`], unit: 'bytesPerSec' },
    ],
  },
  {
    id: 'disk',
    variants: [
      { title: 'Disk used (by device)', queries: (h) => [`max:system.disk.used{${h}} by {device}`], unit: 'bytes' },
      { title: 'Disk used (%) (by mountpoint)', queries: (h) => [`max:system.filesystem.utilization{${h}} by {mountpoint}`], unit: 'fraction' },
    ],
  },
];

function fmtUnit(v: number, unit: HostUnit): string {
  if (unit === 'percent') return `${Math.round(v)}%`;
  if (unit === 'fraction') return `${Math.round(v * 100)}%`;
  if (unit === 'bytes' || unit === 'bytesPerSec') return `${formatBytes(v)}${unit === 'bytesPerSec' ? '/s' : ''}`;
  return compact(v);
}

function HostChart({ chart, host }: { chart: (typeof HOST_CHARTS)[number]; host: string }) {
  const { range } = useTime();
  const scope = `host:${host}`;
  const q = useQuery({
    queryKey: ['apm', 'service-host', chart.id, scope, range],
    // The first naming scheme with data wins: Datadog Agent hosts, then OpenTelemetry hostmetrics.
    queryFn: async ({ signal }) => {
      for (const v of chart.variants) {
        const series = await queryTimeseries({ queries: v.queries(scope), range, points: 120 }, signal);
        if (series.some((x) => x.points.length)) return { variant: v, series };
      }
      return { variant: chart.variants[0], series: [] };
    },
  });
  const v = q.data?.variant ?? chart.variants[0];
  const option = useMemo<EChartsOption | null>(() => {
    if (!q.data?.series.length) return null;
    const { variant, series } = q.data;
    return {
      grid: { left: 64, right: 8, top: 10, bottom: 46 },
      tooltip: { trigger: 'axis', confine: true, valueFormatter: (x) => fmtUnit(Number(x), variant.unit) },
      legend: { bottom: 0, left: 0, type: 'scroll', textStyle: { fontSize: 11 } },
      xAxis: { type: 'time', axisLabel: { formatter: (t: number) => hhmm(t), hideOverlap: true } },
      yAxis: { type: 'value', axisLabel: { formatter: (x: number) => fmtUnit(x, variant.unit) } },
      series: series.map((x) => ({
        name: x.name,
        type: 'line',
        showSymbol: false,
        stack: variant.stacked ? 'total' : undefined,
        areaStyle: variant.stacked ? { opacity: 0.4 } : undefined,
        lineStyle: { width: 1.3 },
        data: x.points,
      })),
    };
  }, [q.data]);
  return (
    <div className="svc-res-chart">
      <div className="svc-chart-title">{v.title}</div>
      {q.isPending ? <Loading /> : option ? <Chart option={option} height={200} /> : <div className="hp-empty">No data</div>}
    </div>
  );
}

export function InfrastructureSection({ hosts }: { hosts: string[] }) {
  const navigate = useNavigate();
  const [picked, setPicked] = useState<string | null>(null);
  const host = picked && hosts.includes(picked) ? picked : hosts[0];
  return (
    <CollapsibleSection
      id="infrastructure"
      title="Infrastructure"
      icon={Server}
      extra={
        hosts.length > 0 && (
          <button className="btn" onClick={() => navigate(`/infrastructure?query=${encodeURIComponent(hosts.map((h) => `host:${h}`).join(' OR '))}`)}>
            <Server size={14} /> {hosts.length === 1 ? hosts[0] : `${hosts.length} hosts`}
          </button>
        )
      }
    >
      {hosts.length === 0 ? (
        <div className="hp-empty">No hosts reported (set host.name on the service's resource)</div>
      ) : (
        <>
          <div className="svc-res-bar">
            <h4 className="svc-sub-title">Host Metrics</h4>
            {hosts.length > 1 && <Select label="Host" value={host} onChange={setPicked} options={hosts.map((h) => ({ value: h, label: h }))} />}
          </div>
          <div className="svc-res-charts">
            {HOST_CHARTS.map((c) => (
              <HostChart key={`${c.id}:${host}`} chart={c} host={host} />
            ))}
          </div>
        </>
      )}
    </CollapsibleSection>
  );
}

/* ---------- Databases ---------- */

export function DatabasesSection({ scope }: { scope: ServiceScope }) {
  const q = useDatabases(scope);
  return (
    <CollapsibleSection id="databases" title="Databases" icon={Database}>
      <Pending q={q} />
      {q.data &&
        (q.data.length === 0 ? (
          <div className="svc-empty">
            <Database size={44} />
            <h3>No database calls found</h3>
            <p>Calls this service makes to databases show up here, with their query performance, once its tracer records database spans.</p>
            <Link className="btn btn-primary" to="/data/setup">
              Setup Database Monitoring
            </Link>
          </div>
        ) : (
          <table className="apm-table">
            <thead>
              <tr>
                <th>DATABASE</th>
                <th>QUERY</th>
                <th className="num">CALLS</th>
                <th className="num">P95</th>
                <th className="num">ERRORS</th>
                <th>% TIME</th>
              </tr>
            </thead>
            <tbody>
              {q.data.map((d) => (
                <tr key={`${d.dbSystem}:${d.dbName}:${d.statement}`} className="no-click">
                  <td>
                    <span className="svc-name">
                      <ServiceIcon name={d.dbSystem} kind="db" technology={d.dbSystem} />
                      {d.dbName || d.dbSystem}
                    </span>
                  </td>
                  <td className="mono ellipsis stmt" title={d.statement}>
                    {d.statement}
                  </td>
                  <td className="num">{compact(d.requests)}</td>
                  <td className="num">{fmtLatency(d.p95Ms)}</td>
                  <td className={`num ${d.errors ? 'tone-error' : ''}`}>{compact(d.errors)}</td>
                  <td>
                    <span className="share-bar">
                      <i style={{ width: `${Math.min(100, d.timeShare * 100)}%` }} />
                    </span>
                    {fmtPct(d.timeShare)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        ))}
    </CollapsibleSection>
  );
}

/* ---------- Logs ---------- */

export function LogsSection({ service }: { service: string }) {
  const navigate = useNavigate();
  const { range } = useTime();
  const q = useServiceLogs(range, service);
  const [text, setText] = useState('');
  const f = text.trim().toLowerCase();
  const logs = (q.data?.logs ?? []).filter((l) => !f || `${l.message} ${l.host} ${l.status}`.toLowerCase().includes(f));
  return (
    <CollapsibleSection id="logs" title="Logs" icon={ScrollText}>
      <div className="svc-query-row">
        <label className="apm-search">
          <Search size={14} />
          <input placeholder="Filter your logs" value={text} onChange={(e) => setText(e.target.value)} />
        </label>
        <button className="btn" onClick={() => navigate(`/logs?query=${encodeURIComponent([`service:${service}`, text.trim()].filter(Boolean).join(' '))}`)}>
          <ScrollText size={14} /> Log Explorer
        </button>
      </div>
      <Pending q={q} />
      {q.data &&
        (logs.length === 0 ? (
          <div className="svc-empty">
            <ScrollText size={44} />
            <p>No logs from this service in this timeframe. Try expanding the timeframe to view historical data.</p>
          </div>
        ) : (
          <table className="apm-table">
            <tbody>
              {logs.map((l) => (
                <tr key={l.id} className="no-click">
                  <td className="col-status">
                    <i className="status-bar" style={{ background: `var(${STATUS_COLORS[l.status as keyof typeof STATUS_COLORS] ?? STATUS_COLORS.info})` }} />
                  </td>
                  <td className="mono muted nowrap">{formatLogDate(l.timestamp)}</td>
                  <td className="mono muted">{l.host}</td>
                  <td className="mono ellipsis">{l.message}</td>
                </tr>
              ))}
            </tbody>
          </table>
        ))}
    </CollapsibleSection>
  );
}
