import { useEffect, useMemo, useState, type ReactNode } from 'react';
import { useQuery } from '@tanstack/react-query';
import type { EChartsOption } from 'echarts';
import { Check, Copy, ExternalLink, X } from 'lucide-react';
import { useNavigate } from 'react-router-dom';
import { searchSpans } from '../../api/apm/client';
import { searchLogs } from '../../api/client';
import { queryTimeseries } from '../../api/dashboards/client';
import { API_MODE } from '../../api/http';
import { getKubeResource, searchKubernetes, type KubeDetail, type KubeResource } from '../../api/kubernetes/client';
import { searchMonitors } from '../../api/monitors/client';
import { networkFlows } from '../../api/network/client';
import { Chart } from '../../components/charts/Chart';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { formatBytes } from '../../components/ui/Pager';
import { formatLongDate, timeAgo } from '../../lib/format';
import { toYaml } from '../../lib/yaml';
import { LogTable } from '../logs/LogTable';
import { COLUMNS, KIND_INFO, StatusPill, UtilCell, age, formatCores, formatCoresAxis, formatMem, logQuery, metricQueries } from './columns';

type Tab = 'yaml' | 'logs' | 'apm' | 'metrics' | 'processes' | 'network' | 'events' | 'monitors' | 'related' | 'taskdef';
const LABELS: Record<Tab, string> = {
  yaml: 'YAML',
  logs: 'Logs',
  apm: 'APM',
  metrics: 'Metrics',
  processes: 'Processes',
  network: 'Network',
  events: 'Events',
  monitors: 'Monitors',
  related: 'Related Resources',
  taskdef: 'Task Definition',
};

/** Datadog's side panel tabs, in its order: Kubernetes objects, ECS resources, containers and images. */
function tabsFor(r: KubeResource): Tab[] {
  if (r.kind === 'ecs_tasks') return ['related', 'taskdef', 'logs', 'metrics', 'apm', 'network', 'monitors'];
  if (r.kind?.startsWith('ecs_')) return ['related', 'logs', 'metrics', 'apm', 'network', 'monitors'];
  if (r.kind === 'containers') return ['logs', 'apm', 'metrics', 'processes', 'network', 'monitors'];
  if (r.kind === 'container_images') return ['related', 'logs'];
  return ['yaml', 'logs', 'apm', 'metrics', 'processes', 'network', 'events', 'monitors'];
}

const HOUR = 3600e3;

export function KubePanel({ uid, onClose, onFilter, onOpen }: { uid: string; onClose: () => void; onFilter: (key: string, value: string) => void; onOpen: (uid: string) => void }) {
  const q = useQuery({ queryKey: ['kubernetes', 'resource', uid], queryFn: () => getKubeResource(uid), refetchInterval: 60_000 });
  const [picked, setTab] = useState<Tab | null>(null);
  const tabs = q.data ? tabsFor(q.data.resource) : [];
  const tab = picked && tabs.includes(picked) ? picked : tabs[0];

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && !(e.target as HTMLElement).closest('input, textarea') && onClose();
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  return (
    <aside className="host-panel kx-panel" role="dialog" aria-label="Kubernetes resource">
      {q.isPending ? (
        <Loading label="Loading resource…" />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <div className="kx-panel-scroll">
          <header className="kx-panel-head">
            <div className="kx-panel-kind">{q.data.resource.kind ? KIND_INFO[q.data.resource.kind].singular : 'Resource'}</div>
            <div className="kx-panel-title">
              <h2>{q.data.resource.name}</h2>
              <StatusPill status={q.data.resource.status} health={q.data.resource.health} />
              <button className="icon-plain kx-panel-close" title="Close (Esc)" onClick={onClose}>
                <X size={18} />
              </button>
            </div>
            <Summary r={q.data.resource} onFilter={onFilter} />
          </header>
          <nav className="kx-tabs">
            {tabs.map((t) => (
              <button key={t} className={`kx-tab${tab === t ? ' active' : ''}`} onClick={() => setTab(t)}>
                {LABELS[t]}
                {t === 'events' && q.data.events.length > 0 && <span className="kx-tab-count">{q.data.events.length}</span>}
                {t === 'related' && q.data.related.length > 0 && <span className="kx-tab-count">{q.data.related.length}</span>}
              </button>
            ))}
          </nav>
          <div className="kx-panel-body">
            {tab === 'yaml' && <Yaml manifest={q.data.manifest} />}
            {tab === 'logs' && <Logs r={q.data.resource} />}
            {tab === 'apm' && <Apm r={q.data.resource} />}
            {tab === 'metrics' && <Metrics r={q.data.resource} />}
            {tab === 'processes' && <Processes r={q.data.resource} />}
            {tab === 'network' && <NetworkTab r={q.data.resource} />}
            {tab === 'events' && <Events d={q.data} />}
            {tab === 'monitors' && <Monitors r={q.data.resource} />}
            {tab === 'related' && <Related d={q.data} onOpen={onOpen} />}
            {tab === 'taskdef' && <TaskDefinition d={q.data} />}
          </div>
        </div>
      )}
    </aside>
  );
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="kx-field">
      <dt>{label}</dt>
      <dd>{children ?? '—'}</dd>
    </div>
  );
}

/** The panel header: the resource's table columns, owner, utilization, containers and tags. */
function Summary({ r, onFilter }: { r: KubeResource; onFilter: (key: string, value: string) => void }) {
  const cols = r.kind ? COLUMNS[r.kind].filter((c) => !['name', 'status'].includes(c.key)) : [];
  return (
    <div className="kx-summary">
      <dl className="kx-fields">
        {cols.map((c) => (
          <Field key={c.key} label={c.label}>
            {c.render(r, { filter: onFilter })}
          </Field>
        ))}
        {r.owner && <Field label="CONTROLLED BY">{`${r.owner.kind}/${r.owner.name}`}</Field>}
        {r.ip && <Field label="IP">{r.ip}</Field>}
        {r.qos && <Field label="QOS CLASS">{r.qos}</Field>}
        {r.selector && <Field label="SELECTOR">{r.selector}</Field>}
        {r.osImage && <Field label="OS IMAGE">{r.osImage}</Field>}
        {r.runtime && <Field label="CONTAINER RUNTIME">{r.runtime}</Field>}
        {r.zone && <Field label="ZONE">{r.zone}</Field>}
      </dl>

      {r.containers.length > 0 && (
        <table className="kx-mini kx-containers">
          <thead>
            <tr>
              <th>CONTAINER</th>
              <th>STATE</th>
              <th>IMAGE</th>
              <th className="num">RESTARTS</th>
              <th>CPU USAGE / REQUESTS</th>
              <th>MEMORY USAGE / REQUESTS</th>
            </tr>
          </thead>
          <tbody>
            {r.containers.map((c) => (
              <tr key={c.name + c.init}>
                <td>
                  {c.name}
                  {c.init && <span className="kx-chip">init</span>}
                </td>
                <td>
                  <span
                    className={`kx-status kx-${c.state === 'running' && c.ready ? 'ok' : c.reason && /BackOff|Err|OOM|Error/.test(c.reason) ? 'error' : c.state === 'terminated' ? 'info' : 'warning'}`}
                  >
                    {c.reason ?? c.state}
                  </span>
                </td>
                <td className="mono kx-ellipsis" title={c.image}>
                  {c.image}
                </td>
                <td className="num">{c.restarts}</td>
                <td>
                  <UtilCell u={c.cpu} kind="cpu" />
                </td>
                <td>
                  <UtilCell u={c.memory} kind="memory" />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      <div className="kx-tags">
        {r.tags.map((t) => {
          const i = t.indexOf(':');
          return (
            <button key={t} className="kx-tag" title="Filter by this tag" onClick={() => onFilter(t.slice(0, i), t.slice(i + 1))}>
              {t}
            </button>
          );
        })}
      </div>
    </div>
  );
}

function Yaml({ manifest }: { manifest: unknown }) {
  const text = useMemo(() => toYaml(manifest), [manifest]);
  const [copied, setCopied] = useState(false);
  if (manifest == null) return <Empty>The object's manifest is not available (cluster resources are computed from their nodes).</Empty>;
  return (
    <div className="kx-yaml">
      <button
        className="btn btn-sm kx-yaml-copy"
        onClick={() => {
          void navigator.clipboard.writeText(text).then(() => {
            setCopied(true);
            setTimeout(() => setCopied(false), 1500);
          });
        }}
      >
        {copied ? <Check size={13} /> : <Copy size={13} />} {copied ? 'Copied' : 'Copy'}
      </button>
      <pre className="mono">{text}</pre>
    </div>
  );
}

function Events({ d }: { d: KubeDetail }) {
  if (d.events.length === 0) return <Empty>No Kubernetes events for this resource in the last 7 days.</Empty>;
  return (
    <table className="kx-mini kx-events">
      <thead>
        <tr>
          <th>LAST SEEN</th>
          <th>TYPE</th>
          <th>REASON</th>
          <th>OBJECT</th>
          <th>MESSAGE</th>
          <th className="num">COUNT</th>
        </tr>
      </thead>
      <tbody>
        {d.events.map((e, i) => (
          <tr key={i}>
            <td title={formatLongDate(e.timestamp)}>{timeAgo(e.timestamp)}</td>
            <td>
              <span className={`kx-status kx-${e.eventType === 'Warning' ? 'warning' : 'info'}`}>{e.eventType}</span>
            </td>
            <td>{e.reason}</td>
            <td className="kx-ellipsis">{`${e.objectKind}/${e.objectName}`}</td>
            <td className="kx-event-msg">{e.message}</td>
            <td className="num">{e.count}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function Logs({ r }: { r: KubeResource }) {
  const navigate = useNavigate();
  const query = logQuery(r);
  const range = useMemo(() => {
    const to = Date.now();
    return { from: to - HOUR, to };
  }, []);
  const q = useQuery({ queryKey: ['logs', 'kubernetes', query, range], queryFn: () => searchLogs({ range, query: query! }), enabled: !!query });
  if (!query) return <Empty>Logs are not linked to this kind of resource.</Empty>;
  return (
    <div>
      <div className="kx-logs-head">
        <code>{query}</code>
        <button className="btn btn-sm" onClick={() => navigate(`/logs?query=${encodeURIComponent(query)}`)}>
          View in Log Explorer <ExternalLink size={13} />
        </button>
      </div>
      {q.isPending ? (
        <Loading label="Loading logs…" />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : q.data.total === 0 ? (
        <Empty>No logs in the past hour.</Empty>
      ) : (
        <>
          <p className="hw-showing">
            Latest <strong>{Math.min(50, q.data.logs.length)}</strong> of <strong>{q.data.total.toLocaleString()}</strong> logs in the past hour
          </p>
          <div className="hp-logs">
            <LogTable
              logs={q.data.logs.slice(0, 50)}
              selectedId={null}
              onSelect={() => navigate(`/logs?query=${encodeURIComponent(query)}`)}
              options={{ wrap: false, showHost: false, showService: true }}
            />
          </div>
        </>
      )}
    </div>
  );
}

function MetricChart({ title, query, unit }: { title: string; query: string; unit: 'cores' | 'bytes' | 'count' }) {
  const range = useMemo(() => {
    const to = Date.now();
    return { from: to - HOUR, to };
  }, []);
  const q = useQuery({ queryKey: ['kubernetes', 'metric', query, range], queryFn: () => queryTimeseries({ queries: [query], range, points: 60 }) });
  const option = useMemo<EChartsOption>(
    () => ({
      grid: { left: 52, right: 10, top: 12, bottom: 46 },
      tooltip: { trigger: 'axis', valueFormatter: (v) => (unit === 'bytes' ? formatBytes(Number(v)) : unit === 'cores' ? formatCores(Number(v)) : String(v)) },
      legend: { type: 'scroll', bottom: 0, textStyle: { fontSize: 11 } },
      xAxis: { type: 'time', splitNumber: 4, axisLabel: { hideOverlap: true } },
      yAxis: { type: 'value', axisLabel: { formatter: (v: number) => (unit === 'bytes' ? formatBytes(v) : unit === 'cores' ? formatCoresAxis(v) : String(v)) } },
      series: (q.data ?? []).map((s) => ({ name: s.name, type: 'line', showSymbol: false, lineStyle: { width: 1.4 }, data: s.points })),
    }),
    [q.data, unit],
  );
  return (
    <section className="kx-chart">
      <h4>{title}</h4>
      <code className="kx-chart-query">{query}</code>
      {q.isPending ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : q.data.length === 0 ? (
        <Empty>No data in the past hour.</Empty>
      ) : (
        <Chart option={option} height={200} />
      )}
    </section>
  );
}

function Metrics({ r }: { r: KubeResource }) {
  const charts = metricQueries(r);
  if (charts.length === 0) return <Empty>No metrics are collected for this kind of resource.</Empty>;
  return (
    <div className="kx-charts">
      <p className="muted small">Past hour, from the kubelet and cluster metrics the collectors send.</p>
      {charts.map((c) => (
        <MetricChart key={c.query} {...c} />
      ))}
    </div>
  );
}

function Apm({ r }: { r: KubeResource }) {
  const navigate = useNavigate();
  const query = logQuery(r);
  const range = useMemo(() => {
    const to = Date.now();
    return { from: to - HOUR, to };
  }, []);
  const q = useQuery({ queryKey: ['kubernetes', 'apm', query, range], queryFn: () => searchSpans({ range, query: query!, limit: 50 }), enabled: !!query && API_MODE === 'live' });
  if (!query || API_MODE !== 'live') return <Empty>No traces are linked to this kind of resource.</Empty>;
  return (
    <div>
      <div className="kx-logs-head">
        <code>{query}</code>
        <button className="btn btn-sm" onClick={() => navigate(`/apm/traces?query=${encodeURIComponent(query)}`)}>
          View in Trace Explorer <ExternalLink size={13} />
        </button>
      </div>
      {q.isPending ? (
        <Loading label="Loading traces…" />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : q.data.spans.length === 0 ? (
        <Empty>No spans from this resource in the past hour. Spans are linked when the tracer reports the pod's Kubernetes attributes (k8s.pod.name, k8s.namespace.name, …).</Empty>
      ) : (
        <table className="kx-mini">
          <thead>
            <tr>
              <th>DATE</th>
              <th>SERVICE</th>
              <th>RESOURCE</th>
              <th className="num">DURATION</th>
              <th>STATUS</th>
            </tr>
          </thead>
          <tbody>
            {q.data.spans.map((sp) => (
              <tr key={sp.spanId} className="kx-click" onClick={() => navigate(`/apm/traces?trace=${sp.traceId}`)}>
                <td>{timeAgo(sp.timestamp)}</td>
                <td>{sp.service}</td>
                <td className="kx-ellipsis">{sp.resource}</td>
                <td className="num">{sp.durationMs.toFixed(1)} ms</td>
                <td>
                  <span className={`kx-status kx-${sp.status === 'error' ? 'error' : 'ok'}`}>{sp.status}</span>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

/** Monitors whose query or tags scope them to this resource (its own tag, e.g. `kube_deployment:checkout`). */
/** Traffic from and to the resource's pods (Cloud Network Monitoring), by service. */
function NetworkTab({ r }: { r: KubeResource }) {
  const navigate = useNavigate();
  const pod = r.kind === 'containers' ? r.owner?.name : r.kind === 'pods' ? r.name : null;
  const range = useMemo(() => {
    const to = Date.now();
    return { from: to - HOUR, to };
  }, []);
  const q = useQuery({
    queryKey: ['kubernetes', 'network', pod, range],
    queryFn: () => networkFlows({ range, query: `client_pod_name:${pod}`, clientGroupBy: 'pod_name', serverGroupBy: 'service', showExternal: true, showUnresolved: true, limit: 100 }),
    enabled: !!pod,
  });
  if (!pod) return <Empty>Network traffic is shown for pods and containers.</Empty>;
  if (q.isPending) return <Loading label="Loading network traffic…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  if (q.data.flows.length === 0) return <Empty>{`No connections from pod ${pod} in the past hour. Traffic appears when the Datadog Agent's system-probe has network monitoring enabled.`}</Empty>;
  return (
    <div>
      <div className="kx-logs-head">
        <code>client_pod_name:{pod}</code>
        <button className="btn btn-sm" onClick={() => navigate(`/network?q=${encodeURIComponent(`client_pod_name:${pod}`)}`)}>
          View in Network Analytics <ExternalLink size={13} />
        </button>
      </div>
      <table className="kx-mini">
        <thead>
          <tr>
            <th>SERVER</th>
            <th className="num">VOLUME SENT</th>
            <th className="num">VOLUME RECEIVED</th>
            <th className="num">TCP RETRANSMITS</th>
          </tr>
        </thead>
        <tbody>
          {q.data.flows.map((f) => (
            <tr key={f.server}>
              <td>{f.server}</td>
              <td className="num">{formatBytes(f.bytesSent)}</td>
              <td className="num">{formatBytes(f.bytesReceived)}</td>
              <td className="num">{f.retransmits}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** Processes running in the pod or container, from the live process data. */
function Processes({ r }: { r: KubeResource }) {
  const pod = r.kind === 'containers' ? r.owner?.name : r.name;
  const scope = [`pod_name:${pod}`, r.namespace ? `kube_namespace:${r.namespace}` : '', r.kind === 'containers' ? `kube_container_name:${r.name}` : ''].filter(Boolean).join(' ');
  const q = useQuery({
    queryKey: ['kubernetes', 'processes', scope],
    queryFn: () => searchKubernetes({ kind: 'processes', query: scope, groupBy: null, sort: 'cpu', desc: true, offset: 0, limit: 200 }),
  });
  if (q.isPending) return <Loading label="Loading processes…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  if (q.data.items.length === 0) return <Empty>{`No processes reported for ${scope}. Processes come from the Datadog Agent's process collection.`}</Empty>;
  const cols = COLUMNS.processes.filter((c) => c.key !== 'pod' && c.key !== 'host');
  return (
    <table className="kx-mini">
      <thead>
        <tr>
          {cols.map((c) => (
            <th key={c.key}>{c.label}</th>
          ))}
        </tr>
      </thead>
      <tbody>
        {q.data.items.map((p) => (
          <tr key={p.uid}>
            {cols.map((c) => (
              <td key={c.key} className={c.key === 'name' ? 'kx-ellipsis' : undefined}>
                {c.render(p, { filter: () => {} })}
              </td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function Monitors({ r }: { r: KubeResource }) {
  const navigate = useNavigate();
  const own = r.tags.filter((t) => /^(pod_name|kube_deployment|kube_daemon_set|kube_stateful_set|kube_replica_set|kube_job|kube_cronjob|kube_node|kube_service):/.test(t));
  const scope = r.kind === 'namespaces' ? [`kube_namespace:${r.name}`] : r.kind === 'clusters' ? [`kube_cluster_name:${r.name}`] : own;
  const q = useQuery({
    queryKey: ['kubernetes', 'monitors', scope],
    queryFn: () => searchMonitors({ query: '', offset: 0, limit: 500, sort: null, creators: [], drafts: [], muted: [], priorities: [], recipients: [], statuses: [], teams: [], types: [] }),
    enabled: API_MODE === 'live',
  });
  if (API_MODE !== 'live') return <Empty>No monitors are scoped to this resource.</Empty>;
  if (q.isPending) return <Loading label="Loading monitors…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const related = q.data.monitors.filter((m) => scope.some((t) => m.query.includes(t) || m.tags.includes(t)));
  if (related.length === 0) return <Empty>{`No monitors are scoped to ${scope.join(' or ') || 'this resource'}.`}</Empty>;
  return (
    <table className="kx-mini">
      <thead>
        <tr>
          <th>STATUS</th>
          <th>MONITOR</th>
          <th>QUERY</th>
        </tr>
      </thead>
      <tbody>
        {related.map((m) => (
          <tr key={m.id} className="kx-click" onClick={() => navigate(`/monitors/${m.id}`)}>
            <td>
              <span className={`kx-status kx-${m.status === 'alert' ? 'error' : m.status === 'warn' ? 'warning' : m.status === 'ok' ? 'ok' : 'info'}`}>{m.status.toUpperCase()}</span>
            </td>
            <td>{m.name}</td>
            <td className="mono kx-ellipsis">{m.query}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function Related({ d, onOpen }: { d: KubeDetail; onOpen: (uid: string) => void }) {
  if (d.related.length === 0) return <Empty>No related resources.</Empty>;
  return (
    <table className="kx-mini">
      <thead>
        <tr>
          <th>TYPE</th>
          <th>NAME</th>
          <th>STATUS</th>
          <th className="num">AGE</th>
        </tr>
      </thead>
      <tbody>
        {d.related.map((x) => (
          <tr key={x.uid} className="kx-click" onClick={() => onOpen(x.uid)}>
            <td>{x.kind ? KIND_INFO[x.kind].singular : ''}</td>
            <td className="kx-ellipsis">{x.name}</td>
            <td>
              <StatusPill status={x.status} health={x.health} />
            </td>
            <td className="num">{age(x.createdAt)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/** What the task metadata reports about the task's definition and its containers. */
function TaskDefinition({ d }: { d: KubeDetail }) {
  const r = d.resource;
  const containers = d.related.filter((x) => x.kind === 'containers');
  return (
    <div>
      <dl className="kx-fields">
        <Field label="TASK DEFINITION">{r.taskDefinition}</Field>
        <Field label="LAUNCH TYPE">{r.launchType}</Field>
        <Field label="CPU RESERVATION">{r.cpuReservation == null ? '—' : `${r.cpuReservation} vCPU`}</Field>
        <Field label="MEM RESERVATION">{formatMem(r.memoryReservation)}</Field>
      </dl>
      <table className="kx-mini kx-containers">
        <thead>
          <tr>
            <th>CONTAINER</th>
            <th>IMAGE</th>
            <th>STATUS</th>
          </tr>
        </thead>
        <tbody>
          {containers.map((c) => (
            <tr key={c.uid}>
              <td>{c.name}</td>
              <td className="mono kx-ellipsis">{c.images[0] ?? '—'}</td>
              <td>
                <StatusPill status={c.status} health={c.health} />
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
