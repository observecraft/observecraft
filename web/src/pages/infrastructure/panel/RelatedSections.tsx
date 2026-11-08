// The host panel's sections drawn from other products, filtered on the host (`host:<name>`): its traces, profiles,
// network traffic, changes, security signals, and how it relates to its cloud account, cluster and services.
import { useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Cloud, ExternalLink, Flame, GitCommitHorizontal, ListTree, Network, Server, Share2, ShieldCheck, Ship, Waypoints } from 'lucide-react';
import { Link, useNavigate } from 'react-router-dom';
import { searchSpans, spanFacets } from '../../../api/apm/client';
import { searchChanges, type Change } from '../../../api/changes/client';
import type { HostDetail } from '../../../api/infrastructure/types';
import { networkFlows } from '../../../api/network/client';
import { searchProfiles } from '../../../api/profiles/client';
import { searchSignals } from '../../../api/security/client';
import { useTime } from '../../../components/shell/TimeContext';
import { Empty, ErrorState, Loading } from '../../../components/ui/AsyncState';
import { formatBytes, formatLogDate, timeAgo } from '../../../lib/format';
import { snapRange } from '../../../lib/time';
import { usePods } from '../queries';
import { HpSection } from './HpSection';

const SHOWN = 10;

/** The panel's time range, snapped so queries are reused while the range is live. */
function useHostRange() {
  const { range: live } = useTime();
  const { from, to } = snapRange(live);
  return useMemo(() => ({ from, to }), [from, to]);
}

const hostQuery = (host: HostDetail) => `host:${host.name}`;

function ViewIn({ to, label }: { to: string; label: string }) {
  const navigate = useNavigate();
  return (
    <button className="btn" onClick={() => navigate(to)}>
      {label} <ExternalLink size={13} />
    </button>
  );
}

function Showing({ count, total, noun }: { count: number; total: number; noun: string }) {
  return (
    <p className="hw-showing">
      Latest <strong>{count}</strong> of <strong>{total.toLocaleString()}</strong> {noun}
    </p>
  );
}

function ms(d: number): string {
  return d >= 1000 ? `${(d / 1000).toFixed(2)} s` : `${d.toFixed(d < 10 ? 2 : 0)} ms`;
}

export function TracesSection({ host }: { host: HostDetail }) {
  const range = useHostRange();
  const query = hostQuery(host);
  const q = useQuery({ queryKey: ['host-panel', 'spans', host.name, range], queryFn: ({ signal }) => searchSpans({ range, query, limit: SHOWN }, signal) });
  return (
    <HpSection id="traces" title="Traces" icon={ListTree} extra={<ViewIn to={`/apm/traces?query=${encodeURIComponent(query)}`} label="View in Trace Explorer" />}>
      {q.isPending ? (
        <Loading label="Loading traces…" />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : q.data.spans.length === 0 ? (
        <Empty>No spans from services on this host in the selected time range. Instrument them with a Datadog tracing library or the OpenTelemetry SDK.</Empty>
      ) : (
        <>
          <Showing count={q.data.spans.length} total={q.data.total} noun="spans" />
          <table className="hw-table">
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
              {q.data.spans.map((s) => (
                <tr key={s.spanId}>
                  <td className="mono">{formatLogDate(s.timestamp)}</td>
                  <td>{s.service}</td>
                  <td>
                    <Link to={`/apm/traces?query=${encodeURIComponent(query)}&trace=${s.traceId}&span=${s.spanId}`}>{s.resource}</Link>
                  </td>
                  <td className="num">{ms(s.durationMs)}</td>
                  <td>
                    <span className={`hw-status ${s.status === 'error' ? 'is-error' : 'is-ok'}`}>{s.status === 'error' ? 'Error' : 'OK'}</span>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}
    </HpSection>
  );
}

export function ProfilesSection({ host }: { host: HostDetail }) {
  const range = useHostRange();
  const query = hostQuery(host);
  const q = useQuery({ queryKey: ['host-panel', 'profiles', host.name, range], queryFn: () => searchProfiles({ range, query, env: '', service: '', limit: SHOWN }) });
  return (
    <HpSection id="profiles" title="Profiles" icon={Flame} extra={<ViewIn to={`/apm/profiles?q=${encodeURIComponent(query)}`} label="View in Profiles" />}>
      {q.isPending ? (
        <Loading label="Loading profiles…" />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : q.data.items.length === 0 ? (
        <Empty>No profiles from this host in the selected time range. Turn on the profiler of a Datadog tracing library (DD_PROFILING_ENABLED=true).</Empty>
      ) : (
        <table className="hw-table">
          <thead>
            <tr>
              <th>DATE</th>
              <th>SERVICE</th>
              <th>LANGUAGE</th>
              <th>VERSION</th>
              <th>DURATION</th>
            </tr>
          </thead>
          <tbody>
            {q.data.items.slice(0, SHOWN).map((p) => (
              <tr key={`${p.runtimeId}-${p.start}`}>
                <td className="mono">{formatLogDate(p.start)}</td>
                <td>
                  <Link to={`/apm/profiles?q=${encodeURIComponent(`${query} service:${p.service}`)}`}>{p.service}</Link>
                </td>
                <td>{p.language || '—'}</td>
                <td>{p.version || '—'}</td>
                <td>{Math.max(0, Math.round((p.end - p.start) / 1000))} s</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </HpSection>
  );
}

export function NetworkSection({ host }: { host: HostDetail }) {
  const range = useHostRange();
  const query = hostQuery(host);
  const q = useQuery({
    queryKey: ['host-panel', 'network', host.name, range],
    queryFn: () => networkFlows({ range, query, clientGroupBy: 'service', serverGroupBy: 'service', limit: SHOWN, showExternal: true, showUnresolved: true }),
  });
  return (
    <HpSection id="network" title="Network" icon={Network} extra={<ViewIn to={`/network?q=${encodeURIComponent(query)}`} label="View in Network Analytics" />}>
      {q.isPending ? (
        <Loading label="Loading network traffic…" />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : q.data.flows.length === 0 ? (
        <Empty>
          {q.data.enabled
            ? 'No network traffic from or to this host in the selected time range.'
            : 'No network data from this host. Turn on Cloud Network Monitoring in the Datadog Agent (Helm: datadog.networkMonitoring.enabled: true).'}
        </Empty>
      ) : (
        <>
          <Showing count={q.data.flows.length} total={q.data.total} noun="flows" />
          <table className="hw-table">
            <thead>
              <tr>
                <th>CLIENT</th>
                <th>SERVER</th>
                <th className="num">SENT</th>
                <th className="num">RECEIVED</th>
                <th className="num">RETRANSMITS</th>
                <th className="num">RTT</th>
              </tr>
            </thead>
            <tbody>
              {q.data.flows.map((f) => (
                <tr key={`${f.client}→${f.server}`}>
                  <td>{f.client}</td>
                  <td>{f.server}</td>
                  <td className="num">{formatBytes(f.bytesSent)}</td>
                  <td className="num">{formatBytes(f.bytesReceived)}</td>
                  <td className="num">{f.retransmits.toLocaleString()}</td>
                  <td className="num">{f.rttUs == null ? '—' : ms(f.rttUs / 1000)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}
    </HpSection>
  );
}

/** Services whose spans name this host (`spanFacets` on `host:<name>`). */
function useHostServices(host: HostDetail) {
  const range = useHostRange();
  return useQuery({
    queryKey: ['host-panel', 'services', host.name, range],
    queryFn: async ({ signal }) => {
      const r = await spanFacets({ range, query: hostQuery(host), filters: {}, scope: 'all' }, ['service'], signal);
      return (r.facets.service ?? []).map((v) => ({ name: v.value, count: v.count }));
    },
  });
}

/** Up to five values as `(a OR b)`, quoted where needed. */
function anyOf(values: string[]): string {
  const quoted = values.slice(0, 5).map((v) => (/[\s:()"]/.test(v) ? `"${v.replace(/"/g, '\\"')}"` : v));
  return quoted.length === 1 ? quoted[0] : `(${quoted.join(' OR ')})`;
}

export function ChangesSection({ host }: { host: HostDetail }) {
  const range = useHostRange();
  const services = useHostServices(host);
  const resources = [host.name, ...(host.kubernetes?.node && host.kubernetes.node !== host.name ? [host.kubernetes.node] : [])];
  const serviceNames = (services.data ?? []).map((s) => s.name);
  const q = useQuery({
    queryKey: ['host-panel', 'changes', host.name, resources, serviceNames, range],
    enabled: !services.isPending,
    queryFn: async () => {
      // The host's and its node's changes, and the deployments of the services running on it.
      const pages = await Promise.all([
        searchChanges({ range, query: `resource:${anyOf(resources)}`, offset: 0, limit: SHOWN }),
        ...(serviceNames.length ? [searchChanges({ range, query: `service:${anyOf(serviceNames)}`, offset: 0, limit: SHOWN })] : []),
      ]);
      const byId = new Map<string, Change>();
      pages.flatMap((p) => p.items).forEach((c) => byId.set(c.id, c));
      return [...byId.values()].sort((a, b) => b.timestamp - a.timestamp).slice(0, SHOWN);
    },
  });
  return (
    <HpSection
      id="changes"
      title="Changes"
      icon={GitCommitHorizontal}
      extra={<ViewIn to={`/infrastructure/changes?q=${encodeURIComponent(`resource:${anyOf(resources)}`)}`} label="View in Changes" />}
    >
      {q.isPending ? (
        <Loading label="Loading changes…" />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : q.data.length === 0 ? (
        <Empty>No deployments or configuration changes for this host, its node or its services in the selected time range.</Empty>
      ) : (
        <table className="hw-table">
          <thead>
            <tr>
              <th>DATE</th>
              <th>TYPE</th>
              <th>RESOURCE</th>
              <th>CHANGE</th>
            </tr>
          </thead>
          <tbody>
            {q.data.map((c) => (
              <tr key={c.id}>
                <td className="mono">{formatLogDate(c.timestamp)}</td>
                <td>{c.changeType.replace(/_/g, ' ')}</td>
                <td>
                  {c.resourceType}: {c.resource}
                </td>
                <td>{c.summary}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </HpSection>
  );
}

const SEVERITY_ORDER = ['critical', 'high', 'medium', 'low', 'info'];

export function SecuritySection({ host }: { host: HostDetail }) {
  const range = useHostRange();
  const query = hostQuery(host);
  const q = useQuery({ queryKey: ['host-panel', 'signals', host.name, range], queryFn: ({ signal }) => searchSignals({ range, query, state: '', limit: SHOWN, offset: 0, ascending: false }, signal) });
  return (
    <HpSection id="security" title="Security" icon={ShieldCheck} extra={<ViewIn to={`/security/siem/signals?q=${encodeURIComponent(query)}`} label="View in Signals" />}>
      {q.isPending ? (
        <Loading label="Loading security signals…" />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : q.data.signals.length === 0 ? (
        <Empty>No security signals about this host in the selected time range.</Empty>
      ) : (
        <>
          <Showing count={q.data.signals.length} total={q.data.total} noun="signals" />
          <table className="hw-table">
            <thead>
              <tr>
                <th>SEVERITY</th>
                <th>SIGNAL</th>
                <th>STATE</th>
                <th>LAST SEEN</th>
              </tr>
            </thead>
            <tbody>
              {[...q.data.signals]
                .sort((a, b) => SEVERITY_ORDER.indexOf(a.severity) - SEVERITY_ORDER.indexOf(b.severity) || b.lastSeen - a.lastSeen)
                .map((s) => (
                  <tr key={s.id}>
                    <td>
                      <span className={`hw-severity sev-${s.severity}`}>{s.severity}</span>
                    </td>
                    <td>
                      <Link to={`/security/siem/signals?q=${encodeURIComponent(query)}&signal=${s.id}`}>{s.title}</Link>
                    </td>
                    <td>{s.state.replace(/_/g, ' ')}</td>
                    <td>{timeAgo(s.lastSeen)}</td>
                  </tr>
                ))}
            </tbody>
          </table>
        </>
      )}
    </HpSection>
  );
}

function RelNode({ icon: Icon, kind, name, to, detail }: { icon: typeof Server; kind: string; name: string; to?: string; detail?: string }) {
  return (
    <div className="hw-rel-node">
      <Icon size={15} />
      <span className="hw-rel-kind">{kind}</span>
      {to ? <Link to={to}>{name}</Link> : <strong>{name}</strong>}
      {detail && <span className="hw-rel-detail">{detail}</span>}
    </div>
  );
}

export function RelationshipsSection({ host }: { host: HostDetail }) {
  const services = useHostServices(host);
  const pods = usePods({ host: host.name, query: '', offset: 0, limit: 500 });
  const namespaces = useMemo(() => {
    const counts = new Map<string, number>();
    (pods.data?.items ?? []).forEach((p) => counts.set(p.namespace, (counts.get(p.namespace) ?? 0) + 1));
    return [...counts.entries()].sort((a, b) => b[1] - a[1]);
  }, [pods.data]);
  const k8s = host.kubernetes;
  const cloud = host.cloud !== 'none' ? [host.cloud.toUpperCase(), host.account, host.region].filter(Boolean).join(' · ') : null;
  return (
    <HpSection id="relationships" title="Relationships" icon={Share2}>
      <div className="hw-rel">
        {cloud && <RelNode icon={Cloud} kind="Cloud" name={cloud} detail={host.hostType} />}
        <RelNode icon={Server} kind="Host" name={host.name} />
        {k8s && <RelNode icon={Ship} kind="Kubernetes" name={k8s.cluster} to="/kubernetes" detail={k8s.node ? `node ${k8s.node}${pods.data ? ` · ${pods.data.total} pods` : ''}` : undefined} />}
        {namespaces.length > 0 && (
          <div className="hw-rel-group">
            <span className="hw-rel-kind">Namespaces</span>
            {namespaces.slice(0, 12).map(([ns, n]) => (
              <span key={ns} className="hw-rel-chip">
                {ns} <em>{n}</em>
              </span>
            ))}
          </div>
        )}
        <div className="hw-rel-group">
          <span className="hw-rel-kind">
            <Waypoints size={14} /> Services
          </span>
          {services.isPending ? (
            <span className="muted-text">Loading…</span>
          ) : (services.data ?? []).length === 0 ? (
            <span className="muted-text">No services on this host send traces.</span>
          ) : (
            (services.data ?? []).slice(0, 12).map((s) => (
              <Link key={s.name} className="hw-rel-chip" to={`/apm/services/${encodeURIComponent(s.name)}`}>
                {s.name} <em>{s.count.toLocaleString()}</em>
              </Link>
            ))
          )}
        </div>
      </div>
    </HpSection>
  );
}
