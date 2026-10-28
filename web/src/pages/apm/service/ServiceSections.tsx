import { useMemo, useState } from 'react';
import type { EChartsOption } from 'echarts';
import { useNavigate } from 'react-router-dom';
import { AlertTriangle, Bug, Database, ExternalLink, GitCommitHorizontal, Hexagon, LayoutGrid, ListTree, ScrollText, Server, Share2, ShieldCheck, Target } from 'lucide-react';
import type { Dependency, ServiceScope, ServiceSummary } from '../../../api/apm/client';
import { Chart } from '../../../components/charts/Chart';
import { useTime } from '../../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { CollapsibleSection } from '../../../components/ui/CollapsibleSection';
import { compact, formatLogDate, hhmm, timeAgo } from '../../../lib/format';
import { STATUS_COLORS, statusColor, token } from '../../../theme/chartTheme';
import { useDatabases, useDependencies, useDeployments, useResources, useServiceLogs, useSpanErrors, useSpanSearch } from '../queries';
import { errorTone, fmtLatency, fmtPct, fmtRate, KIND_LABEL, LineSpark, ServiceIcon } from '../visuals';
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

export function SummarySection({ summary }: { summary: ServiceSummary }) {
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
          <div className="svc-card-value muted">None</div>
        </div>
        <div className="svc-card">
          <div className="svc-card-title">
            <Target size={14} /> Health Monitors
          </div>
          <div className="svc-card-value muted">No monitors</div>
        </div>
        <div className="svc-card">
          <div className="svc-card-title">
            <ShieldCheck size={14} /> SLOs
          </div>
          <div className="svc-card-value muted">None</div>
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

export function ResourcesSection({ scope, onPick }: { scope: ServiceScope; onPick: (resource: string) => void }) {
  const q = useResources({ range: scope.range, env: scope.env, service: scope.service });
  return (
    <CollapsibleSection id="resources" title="Resources" icon={LayoutGrid}>
      <Pending q={q} />
      {q.data &&
        (q.data.length === 0 ? (
          <div className="hp-empty">No resources in this time range</div>
        ) : (
          <table className="apm-table">
            <thead>
              <tr>
                <th>RESOURCE</th>
                <th className="num">REQUESTS</th>
                <th className="num">TOTAL</th>
                <th className="num">P50</th>
                <th className="num">P95</th>
                <th className="num">P99</th>
                <th className="num">ERROR RATE</th>
                <th>% TIME SPENT</th>
              </tr>
            </thead>
            <tbody>
              {q.data.map((r) => (
                <tr key={r.resource} className={scope.resource === r.resource ? 'selected' : ''} onClick={() => onPick(r.resource)}>
                  <td className="mono">{r.resource}</td>
                  <td className="num">{fmtRate(r.reqPerSec)}</td>
                  <td className="num">{compact(r.requests)}</td>
                  <td className="num">{fmtLatency(r.p50Ms)}</td>
                  <td className="num">{fmtLatency(r.p95Ms)}</td>
                  <td className="num">{fmtLatency(r.p99Ms)}</td>
                  <td className={`num ${errorTone(r.errorRate)}`}>{fmtPct(r.errorRate)}</td>
                  <td>
                    <span className="share-bar">
                      <i style={{ width: `${Math.min(100, r.timeShare * 100)}%` }} />
                    </span>
                    {fmtPct(r.timeShare)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        ))}
    </CollapsibleSection>
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
          <div className="hp-empty">No versions reported (set service.version on the service's resource)</div>
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
              <th>SERVICE</th>
              <th className="num">REQUESTS</th>
              <th className="num">ERRORS</th>
              <th />
              <th className="num">P95</th>
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
        <label className="toggle">
          <input type="checkbox" checked={grouped} onChange={(e) => setGrouped(e.target.checked)} /> Grouped
        </label>
      }
    >
      <Pending q={q} />
      {q.data && (
        <>
          <DependencyMap service={scope.service} deps={q.data} />
          <div className="dep-tables">
            <DependencyTable title="Downstream" rows={q.data.downstream} grouped={grouped} />
            <DependencyTable title="Upstream" rows={q.data.upstream} grouped={grouped} />
          </div>
        </>
      )}
    </CollapsibleSection>
  );
}

/* ---------- Traces ---------- */

export function TracesSection({ scope, onOpen }: { scope: ServiceScope; onOpen: (traceId: string) => void }) {
  const navigate = useNavigate();
  const [errorsOnly, setErrorsOnly] = useState(false);
  const query = [`service:${scope.service}`, scope.resource && `resource:"${scope.resource.replace(/"/g, '')}"`, scope.version && `version:${scope.version}`, errorsOnly && 'status:error']
    .filter(Boolean)
    .join(' ');
  const q = useSpanSearch({ range: scope.range, env: scope.env, query, limit: 25 });
  return (
    <CollapsibleSection
      id="traces"
      title="Traces"
      icon={ListTree}
      extra={
        <>
          <label className="toggle">
            <input type="checkbox" checked={errorsOnly} onChange={(e) => setErrorsOnly(e.target.checked)} /> Errors only
          </label>
          <button className="btn" onClick={() => navigate(`/apm/traces?query=${encodeURIComponent(query)}`)}>
            View in Trace Explorer <ExternalLink size={12} />
          </button>
        </>
      }
    >
      <Pending q={q} />
      {q.data && <SpanTable spans={q.data.spans} onOpen={onOpen} showService={false} />}
    </CollapsibleSection>
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
  return (
    <CollapsibleSection
      id="errors"
      title="Errors"
      icon={Bug}
      extra={
        <button className="btn" onClick={() => navigate('/errors')}>
          Error Tracking <ExternalLink size={12} />
        </button>
      }
    >
      <Pending q={q} />
      {q.data &&
        (q.data.length === 0 ? (
          <div className="hp-empty">No errors in this time range</div>
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
              {q.data.map((e) => (
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

export function InfrastructureSection({ hosts }: { hosts: string[] }) {
  const navigate = useNavigate();
  return (
    <CollapsibleSection id="infrastructure" title="Infrastructure" icon={Server}>
      {hosts.length === 0 ? (
        <div className="hp-empty">No hosts reported (set host.name on the service's resource)</div>
      ) : (
        <table className="apm-table">
          <thead>
            <tr>
              <th>HOST</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {hosts.map((h) => (
              <tr key={h} onClick={() => navigate('/infrastructure')}>
                <td className="mono">{h}</td>
                <td className="num">
                  <button
                    className="btn btn-sm"
                    onClick={(e) => {
                      e.stopPropagation();
                      navigate(`/logs?query=${encodeURIComponent(`host:${h}`)}`);
                    }}
                  >
                    Logs
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
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
          <div className="hp-empty">This service makes no database calls</div>
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
  return (
    <CollapsibleSection
      id="logs"
      title="Logs"
      icon={ScrollText}
      extra={
        <button className="btn" onClick={() => navigate(`/logs?query=${encodeURIComponent(`service:${service}`)}`)}>
          View in Log Explorer <ExternalLink size={12} />
        </button>
      }
    >
      <Pending q={q} />
      {q.data &&
        (q.data.logs.length === 0 ? (
          <div className="hp-empty">No logs from this service in this time range</div>
        ) : (
          <table className="apm-table">
            <tbody>
              {q.data.logs.map((l) => (
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
