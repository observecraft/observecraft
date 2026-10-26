import { useMemo, useState, type ReactNode } from 'react';
import { useQuery } from '@tanstack/react-query';
import type { EChartsOption } from 'echarts';
import { AlertCircle, ChevronDown, ChevronRight, Clock, Info, Pin, PinOff } from 'lucide-react';
import type { TraceDetail, TraceSpan } from '../../../api/apm/client';
import { serviceLogs, traceLogs } from '../../../api/apm/client';
import { queryTimeseries } from '../../../api/dashboards/client';
import { getHostMetrics, listProcesses } from '../../../api/infrastructure/client';
import { Chart } from '../../../components/charts/Chart';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { palette } from '../../../theme/chartTheme';
import { useSpanPercentile } from '../queries';
import { fmtLatency } from '../visuals';
import { ServiceGlyph } from '../explorer/SpanBits';
import { httpMethod, isError } from './TraceView';

type Tab = 'overview' | 'errors' | 'infrastructure' | 'metrics' | 'logs' | 'processes';
const PIN_KEY = 'oc.apm.pinned-attributes';

function loadPins(): string[] {
  try {
    return JSON.parse(localStorage.getItem(PIN_KEY) ?? '[]');
  } catch {
    return [];
  }
}

/** Details of the selected span (bottom of the trace panel). */
export function SpanPanel({ span, trace }: { span: TraceSpan; trace: TraceDetail }) {
  const [tab, setTab] = useState<Tab>('overview');
  const exceptions = span.events.filter((e) => e.name === 'exception');
  const host = span.resource['host.name'];
  const pct = useSpanPercentile(span.service, span.name, span.durationUs / 1000, trace.start + span.startUs / 1000);
  const share = (span.durationUs / Math.max(trace.durationMs * 1000, 1)) * 100;
  const tabs: [Tab, string, number | null][] = [
    ['overview', 'Overview', null],
    ['errors', 'Errors', isError(span) ? Math.max(1, exceptions.length) : 0],
    ['infrastructure', 'Infrastructure', null],
    ['metrics', 'Metrics', null],
    ['logs', 'Logs', null],
    ['processes', 'Processes', null],
  ];
  // The span's time, ±30 minutes, for correlated data.
  const at = trace.start + span.startUs / 1000;
  const around = { from: Math.round(at - 30 * 60_000), to: Math.round(at + 30 * 60_000) };

  return (
    <section className="sp">
      <header className="sp-head">
        <span className="sp-svc">
          <ServiceGlyph service={span.service} kind={span.kind.toLowerCase()} dbSystem={span.attributes['db.system']} size={22} /> {span.service}
        </span>
        <i className="sp-op">{operationName(span)}</i>
        {isError(span) && <AlertCircle size={18} className="sp-err-icon" />}
        <b className="sp-res" title={span.name}>
          {resourceLabel(span)}
        </b>
        <span className="sp-right">
          {pct.data && pct.data.peers > 1 && (
            <span className="tv-pct" title={`Slower than ${pct.data.percentile}% of ${pct.data.peers.toLocaleString()} similar spans in the previous hour`}>
              p{pct.data.percentile}
            </span>
          )}
          <span className="tv-dur">
            <Clock size={15} /> {fmtLatency(span.durationUs / 1000)}
          </span>
          <span className="sp-share">
            {share.toFixed(share < 10 ? 1 : 0)}% total exec time
            <i style={{ width: `${Math.min(share, 100)}%` }} />
          </span>
        </span>
      </header>
      <nav className="sp-tabs">
        <span className="muted">Span:</span>
        {tabs.map(([id, label, count]) => (
          <button key={id} className={tab === id ? 'active' : ''} onClick={() => setTab(id)}>
            {label}
            {!!count && <span className="tv-err-count">{count}</span>}
          </button>
        ))}
      </nav>
      <div className="sp-body">
        {tab === 'overview' && <Overview span={span} />}
        {tab === 'errors' && <Errors span={span} />}
        {tab === 'infrastructure' && (host ? <Infrastructure host={host} range={around} at={at} /> : <p className="muted">This span has no host.name, so there are no host metrics to show.</p>)}
        {tab === 'metrics' && <ServiceMetrics service={span.service} range={around} at={at} />}
        {tab === 'logs' && <Logs service={span.service} traceId={trace.traceId} spanId={span.spanId} at={at} />}
        {tab === 'processes' && (host ? <Processes host={host} /> : <p className="muted">This span has no host.name.</p>)}
      </div>
    </section>
  );
}

/** "http.server.request", "db.query", … from the span kind and attributes. */
function operationName(s: TraceSpan): string {
  const kind = s.kind.toLowerCase();
  if (s.attributes['db.system']) return `${s.attributes['db.system']}.query`;
  if (s.attributes['messaging.system']) return `${s.attributes['messaging.system']}.${kind === 'producer' ? 'send' : 'process'}`;
  if (httpMethod(s)) return kind === 'client' ? 'http.client.request' : 'http.server.request';
  if (s.attributes['rpc.system']) return `${s.attributes['rpc.system']}.${kind}`;
  return `${kind}.span`;
}

function resourceLabel(s: TraceSpan): string {
  return s.attributes['db.statement'] || s.attributes['db.query.text'] || s.name;
}

function Section({ title, children, icon, defaultOpen = true, aside }: { title: string; children: ReactNode; icon?: ReactNode; defaultOpen?: boolean; aside?: ReactNode }) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <section className="sp-sec">
      <button className="sp-sec-title" onClick={() => setOpen((o) => !o)}>
        {open ? <ChevronDown size={16} /> : <ChevronRight size={16} />} {title} {icon}
        {aside && <span className="sp-sec-aside">{aside}</span>}
      </button>
      {open && <div className="sp-sec-body">{children}</div>}
    </section>
  );
}

const GROUPS: { title: string; prefixes: string[] }[] = [
  { title: 'HTTP Requests', prefixes: ['http.', 'url.', 'user_agent.', 'server.', 'client.', 'network.'] },
  { title: 'Database', prefixes: ['db.'] },
  { title: 'Messaging', prefixes: ['messaging.'] },
  { title: 'RPC', prefixes: ['rpc.'] },
  { title: 'Peer', prefixes: ['peer.'] },
];

function Overview({ span }: { span: TraceSpan }) {
  const [pins, setPins] = useState<string[]>(loadPins);
  const togglePin = (k: string) => {
    const next = pins.includes(k) ? pins.filter((p) => p !== k) : [...pins, k];
    setPins(next);
    try {
      localStorage.setItem(PIN_KEY, JSON.stringify(next));
    } catch {
      /* pins are a per-browser convenience */
    }
  };
  const attrs = span.attributes;
  const exception = span.events.find((e) => e.name === 'exception');
  const grouped = useMemo(() => {
    const used = new Set<string>();
    const out = GROUPS.map((g) => {
      const entries = Object.entries(attrs).filter(([k]) => g.prefixes.some((p) => k.startsWith(p)));
      entries.forEach(([k]) => used.add(k));
      return { title: g.title, entries };
    }).filter((g) => g.entries.length > 0);
    const rest = Object.entries(attrs).filter(([k]) => !used.has(k));
    return { out, rest };
  }, [attrs]);
  const pinned = pins.map((k) => [k, attrs[k] ?? span.resource[k]] as const).filter(([, v]) => v !== undefined);
  const table = (entries: [string, string][]) => (
    <table className="sp-attrs">
      <tbody>
        {entries
          .sort(([a], [b]) => a.localeCompare(b))
          .map(([k, v]) => (
            <tr key={k}>
              <th>{k}</th>
              <td>{v}</td>
              <td className="sp-pin">
                <button className="icon-plain" title={pins.includes(k) ? 'Unpin' : 'Pin to the top of every span'} onClick={() => togglePin(k)}>
                  {pins.includes(k) ? <PinOff size={13} /> : <Pin size={13} />}
                </button>
              </td>
            </tr>
          ))}
      </tbody>
    </table>
  );
  return (
    <>
      {isError(span) && (
        <Section title="Error Message" icon={<AlertCircle size={16} className="sp-err-icon" />}>
          <div className="sp-error">
            <AlertCircle size={16} />
            <span>{exception?.attributes['exception.message'] || span.statusMessage || 'Missing error message and stack trace'}</span>
          </div>
          {exception?.attributes['exception.type'] && <p className="muted sp-error-type">{exception.attributes['exception.type']}</p>}
        </Section>
      )}
      <Section
        title="Pinned Span Attributes"
        icon={<Pin size={14} />}
        defaultOpen={pinned.length > 0}
        aside={
          pinned.length === 0 ? (
            <span className="muted">
              No pinned tags found <Info size={13} />
            </span>
          ) : undefined
        }
      >
        {pinned.length ? table(pinned as [string, string][]) : <p className="muted">Pin an attribute with the pin icon to keep it at the top of every span.</p>}
      </Section>
      {grouped.out.map((g) => (
        <Section key={g.title} title={g.title}>
          {table(g.entries)}
        </Section>
      ))}
      {grouped.rest.length > 0 && <Section title="Span Attributes">{table(grouped.rest)}</Section>}
      <Section title="Resource Attributes" defaultOpen={false}>
        {table(Object.entries(span.resource))}
      </Section>
      <Section title="Span" defaultOpen={false}>
        {table([
          ['span_id', span.spanId],
          ['parent_id', span.parentSpanId || '—'],
          ['kind', span.kind],
          ['status', span.status],
          ['start offset', fmtLatency(span.startUs / 1000)],
        ])}
      </Section>
    </>
  );
}

function Errors({ span }: { span: TraceSpan }) {
  const exceptions = span.events.filter((e) => e.name === 'exception');
  if (!isError(span) && exceptions.length === 0) return <p className="muted">This span has no errors.</p>;
  if (exceptions.length === 0)
    return (
      <div className="sp-error">
        <AlertCircle size={16} /> {span.statusMessage || 'The span is marked as an error but recorded no exception.'}
      </div>
    );
  return (
    <>
      {exceptions.map((e, i) => (
        <div key={i} className="sp-exc">
          <b>{e.attributes['exception.type'] || 'Exception'}</b>
          <p>{e.attributes['exception.message']}</p>
          {e.attributes['exception.stacktrace'] && <pre>{e.attributes['exception.stacktrace']}</pre>}
        </div>
      ))}
    </>
  );
}

function lineChart(series: { name: string; points: [number, number][] }[], at: number, fmt?: (v: number) => string): EChartsOption {
  const colors = palette();
  return {
    grid: { left: 52, right: 10, top: 26, bottom: 22 },
    legend: { top: 0, itemWidth: 10, itemHeight: 8, textStyle: { fontSize: 11 } },
    tooltip: { trigger: 'axis', valueFormatter: fmt ? (v) => fmt(v as number) : undefined },
    xAxis: { type: 'time' },
    yAxis: { type: 'value', splitNumber: 3, axisLabel: fmt ? { formatter: (v: number) => fmt(v) } : undefined },
    series: series.map((s, i) => ({
      name: s.name,
      type: 'line',
      showSymbol: false,
      color: colors[i % colors.length],
      data: s.points,
      ...(i === 0 ? { markLine: { symbol: 'none', silent: true, lineStyle: { color: '#0b6e69', type: 'dashed' }, label: { formatter: 'span' }, data: [{ xAxis: at }] } } : {}),
    })),
  };
}

function Infrastructure({ host, range, at }: { host: string; range: { from: number; to: number }; at: number }) {
  const q = useQuery({ queryKey: ['span-host', host, range], queryFn: () => getHostMetrics(host, range), staleTime: 60_000, retry: 1 });
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  return (
    <>
      <p className="muted">
        Host <b>{host}</b>, 30 minutes either side of the span.
      </p>
      <div className="sp-charts">
        <div>
          <h4>CPU usage (%)</h4>
          <Chart
            option={lineChart(
              q.data.cpu.filter((s) => s.name !== 'idle'),
              at,
            )}
            height={150}
          />
        </div>
        <div>
          <h4>Load average</h4>
          <Chart option={lineChart(q.data.load, at)} height={150} />
        </div>
        <div>
          <h4>Memory (GiB)</h4>
          <Chart option={lineChart(q.data.memory, at)} height={150} />
        </div>
      </div>
    </>
  );
}

function ServiceMetrics({ service, range, at }: { service: string; range: { from: number; to: number }; at: number }) {
  const queries = [`sum:trace.requests{service:${service}}`, `sum:trace.errors{service:${service}}`, `avg:trace.latency.p95{service:${service}}`];
  const q = useQuery({ queryKey: ['span-metrics', service, range], queryFn: () => queryTimeseries({ queries, range, points: 60 }), staleTime: 60_000, retry: 1 });
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const find = (prefix: string) => q.data.filter((s) => s.name.includes(prefix)).map((s) => ({ name: prefix.split('.').pop()!, points: s.points as [number, number][] }));
  return (
    <div className="sp-charts">
      <div>
        <h4>Requests / s</h4>
        <Chart option={lineChart(find('trace.requests'), at)} height={150} />
      </div>
      <div>
        <h4>Errors / s</h4>
        <Chart option={lineChart(find('trace.errors'), at)} height={150} />
      </div>
      <div>
        <h4>p95 latency</h4>
        <Chart option={lineChart(find('trace.latency.p95'), at, fmtLatency)} height={150} />
      </div>
    </div>
  );
}

/** The trace's own logs (matched by trace_id); falls back to the service's logs around the span. */
function Logs({ service, traceId, spanId, at }: { service: string; traceId: string; spanId: string; at: number }) {
  const range = { from: Math.round(at - 5 * 60_000), to: Math.round(at + 5 * 60_000) };
  const linked = useQuery({ queryKey: ['trace-logs', traceId], queryFn: () => traceLogs({ from: Math.round(at - 3_600_000), to: Math.round(at + 3_600_000) }, traceId), staleTime: 60_000, retry: 1 });
  const nearby = useQuery({
    queryKey: ['span-logs', service, range],
    queryFn: () => serviceLogs(range, service),
    staleTime: 60_000,
    retry: 1,
    enabled: linked.isSuccess && linked.data.logs.length === 0,
  });
  const q = linked.isSuccess && linked.data.logs.length === 0 ? nearby : linked;
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const byTrace = q === linked;
  return (
    <>
      <p className="muted">
        {byTrace ? (
          <>
            {q.data.logs.length} log{q.data.logs.length === 1 ? '' : 's'} written in this trace; this span's are highlighted.{' '}
            <a href={`/logs?query=${encodeURIComponent(`trace_id:${traceId}`)}`}>Open in Log Explorer</a>
          </>
        ) : (
          `No logs carry this trace's id, so these are ${service}'s logs within 5 minutes of the span.`
        )}
      </p>
      {q.data.logs.length === 0 ? (
        <p className="muted">No logs.</p>
      ) : (
        <table className="sp-logs">
          <tbody>
            {q.data.logs.map((l) => (
              <tr key={l.id} className={(byTrace ? l.spanId === spanId : Math.abs(l.timestamp - at) < 1000) ? 'near' : ''}>
                <td className="mono">{new Date(l.timestamp).toLocaleTimeString(undefined, { hour12: false })}</td>
                <td>
                  <span className={`sp-log-status s-${l.status}`}>{l.status}</span>
                </td>
                <td className="sp-log-msg">{l.message}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </>
  );
}

function Processes({ host }: { host: string }) {
  const q = useQuery({ queryKey: ['span-processes', host], queryFn: () => listProcesses({ host, query: '', offset: 0, limit: 25 }), staleTime: 60_000, retry: 1 });
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  if (!q.data.enabled) return <p className="muted">Process collection isn't enabled on {host}.</p>;
  return (
    <table className="sx-table">
      <thead>
        <tr>
          <th>PID</th>
          <th>COMMAND</th>
          <th>USER</th>
          <th className="num">CPU</th>
          <th className="num">RSS</th>
        </tr>
      </thead>
      <tbody>
        {q.data.items.map((p) => (
          <tr key={p.pid}>
            <td>{p.pid}</td>
            <td className="mono">{p.command}</td>
            <td>{p.user}</td>
            <td className="num">{p.cpuPct.toFixed(1)}%</td>
            <td className="num">{(p.rssBytes / 1024 / 1024).toFixed(0)} MiB</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
