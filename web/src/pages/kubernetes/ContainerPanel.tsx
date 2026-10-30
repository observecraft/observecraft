import { useEffect, useState } from 'react';
import { Link } from 'react-router-dom';
import { useQuery } from '@tanstack/react-query';
import { Copy, ExternalLink, List, Network as MapIcon, Search, X } from 'lucide-react';
import { searchKubernetes, type KubeResource } from '../../api/kubernetes/client';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { formatBytes } from '../../components/ui/Pager';
import { BarValue, ContainerIcon, age, formatMem, normalizedCpu, shortImage } from './columns';

type Tab = 'overview' | 'metrics' | 'logs' | 'network' | 'traces' | 'profiles';
const TABS: [Tab, string][] = [
  ['overview', 'Overview'],
  ['metrics', 'Resource Metrics'],
  ['logs', 'Logs'],
  ['network', 'Network'],
  ['traces', 'Traces'],
  ['profiles', 'Profiles'],
];
/** Tags shown before "+N". */
const TAGS_SHOWN = 5;

const tagValue = (r: KubeResource, key: string) => r.tags.find((t) => t.startsWith(`${key}:`))?.slice(key.length + 1);

/** The query that finds a container's processes: its runtime id, else its pod and container name. */
function processScope(r: KubeResource): string {
  const id = r.containerId ?? tagValue(r, 'container_id');
  if (id) return `container_id:${id}`;
  return [`pod_name:${r.owner?.name}`, r.namespace ? `kube_namespace:${r.namespace}` : '', `kube_container_name:${r.name}`].filter(Boolean).join(' ');
}

function Stat({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="cp-stat">
      <span>{label}</span>
      <div>{children}</div>
    </div>
  );
}

function RelatedProcesses({ r, view }: { r: KubeResource; view: 'list' | 'map' }) {
  const scope = processScope(r);
  const [filter, setFilter] = useState('');
  const q = useQuery({
    queryKey: ['kubernetes', 'processes', scope],
    queryFn: () => searchKubernetes({ kind: 'processes', query: scope, groupBy: null, sort: 'cpu', desc: true, offset: 0, limit: 200 }),
  });
  if (q.isPending) return <Loading label="Loading processes…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const procs = q.data.items.filter((p) => !filter || p.name.toLowerCase().includes(filter.toLowerCase()));
  if (view === 'map') {
    const cpu = normalizedCpu(r);
    return (
      <div className="cp-map">
        {r.owner && (
          <>
            <div className="cp-node cp-node-parent">
              <span className="cp-node-kind">{r.owner.kind}</span>
              <b>{r.owner.name}</b>
              <span className="cp-node-stats">
                <span>{cpu == null ? '—' : `${cpu.toFixed(2)}% CPU`}</span>
                <span>{formatMem(r.memory?.usage)}</span>
              </span>
            </div>
            <i className="cp-edge" />
          </>
        )}
        <div className="cp-node cp-node-self">
          <ContainerIcon r={r} />
          <div>
            <b>{r.name}</b>
            <span className="cp-node-kind">Container</span>
          </div>
        </div>
        {procs.length > 0 && <i className="cp-edge" />}
        <div className="cp-children">
          {procs.slice(0, 12).map((p) => (
            <div key={p.uid} className="cp-node cp-node-proc" title={p.name}>
              <span className="cp-proc-icon">≋</span>
              <div>
                <b>{p.name}</b>
                <span className="cp-node-kind">Process</span>
              </div>
            </div>
          ))}
          {procs.length > 12 && <span className="muted-text">+{procs.length - 12} more</span>}
        </div>
      </div>
    );
  }
  return (
    <>
      <div className="cp-related-bar">
        <span className="cp-related-type">Processes</span>
        <label className="tv-filter">
          <Search size={14} />
          <input placeholder="Filter your Processes" value={filter} onChange={(e) => setFilter(e.target.value)} />
        </label>
      </div>
      <div className="cp-related-head">
        <span>
          Showing <b>{procs.length ? `1–${Math.min(procs.length, 50)}` : 0}</b> of <b>{procs.length}</b> Processes
        </span>
        <Link to={`/process?${new URLSearchParams({ q: scope })}`}>
          View on Processes page <ExternalLink size={13} />
        </Link>
      </div>
      {procs.length === 0 ? (
        <Empty>No processes reported for this container. Processes come from the Datadog Agent's process collection.</Empty>
      ) : (
        <table className="kx-mini cp-procs">
          <thead>
            <tr>
              <th>COMMAND</th>
              <th>USERNAME</th>
              <th className="num">PID</th>
              <th className="num">CPU</th>
              <th className="num">RSS MEMORY</th>
              <th className="num">AGE</th>
            </tr>
          </thead>
          <tbody>
            {procs.slice(0, 50).map((p) => {
              const [cmd, ...args] = p.name.split(' ');
              const pct = normalizedCpu(p);
              return (
                <tr key={p.uid}>
                  <td className="cp-cmd" title={p.name}>
                    {cmd} <span className="cp-args">{args.join(' ')}</span>
                  </td>
                  <td>{p.user ?? '—'}</td>
                  <td className="num">{p.ip ?? p.uid.split(':')[2] ?? '—'}</td>
                  <td className="num">{pct == null ? '—' : <BarValue text={pct < 1 ? '< 1%' : `${Math.round(pct)}%`} share={pct / 100} />}</td>
                  <td className="num">{p.memory?.usage == null ? '—' : <BarValue text={formatMem(p.memory.usage)} share={p.memory.usage / 4 / 1024 ** 3} />}</td>
                  <td className="num">{age(p.createdAt)}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
    </>
  );
}

/** A container's side panel, as Datadog's live containers show it. The other tabs reuse the explorer's. */
export function ContainerPanel({
  r,
  onClose,
  onFilter,
  renderTab,
}: {
  r: KubeResource;
  onClose: () => void;
  onFilter: (key: string, value: string) => void;
  renderTab: (tab: 'metrics' | 'logs' | 'network' | 'apm') => React.ReactNode;
}) {
  const [tab, setTab] = useState<Tab>('overview');
  const [related, setRelated] = useState<'list' | 'map'>('list');
  const [allTags, setAllTags] = useState(false);
  useEffect(() => setTab('overview'), [r.uid]);
  const cpu = normalizedCpu(r);
  const image = shortImage(r.images[0]);
  const tags = allTags ? r.tags : r.tags.slice(0, TAGS_SHOWN);
  return (
    <div className="cp">
      <header className="cp-head">
        <ContainerIcon r={r} size={22} />
        <h2 title={r.name}>{r.name}</h2>
        <button className="icon-plain" title="Copy name" onClick={() => navigator.clipboard?.writeText(r.name)}>
          <Copy size={15} />
        </button>
        <button className="icon-plain cp-close" title="Close (Esc)" onClick={onClose}>
          <X size={18} />
        </button>
      </header>
      <div className="cp-links">
        {r.node && (
          <span>
            Host: <Link to={`/infrastructure?${new URLSearchParams({ host: r.node })}`}>{r.node}</Link>
          </span>
        )}
        {r.owner && (
          <span>
            {r.owner.kind}:{' '}
            <button className="link-btn" onClick={() => onFilter(r.owner!.kind === 'Pod' ? 'pod_name' : 'task_arn', r.owner!.kind === 'Pod' ? r.owner!.name : r.owner!.uid)}>
              {r.owner.name}
            </button>
          </span>
        )}
        <span>
          Image:{' '}
          <button className="link-btn" onClick={() => onFilter('short_image', image)}>
            {image}
          </button>
        </span>
      </div>
      <div className="cp-stats">
        <Stat label="Started">{r.createdAt ? `${age(r.createdAt)} ago` : '—'}</Stat>
        <Stat label="Status">
          <span className={`ox-up ox-up-${r.health}`}>{r.health === 'ok' ? 'UP' : r.status.toUpperCase()}</span>
        </Stat>
        <Stat label="Container Image">
          <span className="ox-image-chip">{image}</span>
        </Stat>
        <Stat label="Total CPU - Normalized">{cpu == null ? '—' : <BarValue text={cpu < 1 ? '< 1%' : `${Math.round(cpu)}%`} share={cpu / 100} />}</Stat>
        <Stat label="RSS Memory">
          {r.memory?.usage == null ? '—' : <BarValue text={formatMem(r.memory.usage)} share={r.memory.limits ? r.memory.usage / r.memory.limits : r.memory.usage / 8 / 1024 ** 3} />}
        </Stat>
        <Stat label="Net Sent (TX)">{r.netSent == null ? '—' : formatBytes(r.netSent)}</Stat>
        <Stat label="Net Received (RX)">{r.netReceived == null ? '—' : formatBytes(r.netReceived)}</Stat>
      </div>
      <nav className="kx-tabs cp-tabs">
        {TABS.map(([t, label]) => (
          <button key={t} className={`kx-tab${tab === t ? ' active' : ''}`} onClick={() => setTab(t)}>
            {label}
          </button>
        ))}
      </nav>
      <div className="cp-body">
        {tab === 'overview' && (
          <>
            <h4>ALL TAGS</h4>
            <div className="cp-tags">
              {tags.map((t) => {
                const i = t.indexOf(':');
                return (
                  <button key={t} className="kx-tag" title="Filter by this tag" onClick={() => onFilter(t.slice(0, i), t.slice(i + 1))}>
                    <span className="muted-text">{t.slice(0, i + 1)}</span>
                    {t.slice(i + 1)}
                  </button>
                );
              })}
              {r.tags.length > TAGS_SHOWN && (
                <button className="rc-more" onClick={() => setAllTags((a) => !a)}>
                  {allTags ? 'Less' : `+${r.tags.length - TAGS_SHOWN}`}
                </button>
              )}
            </div>
            <div className="cp-related-title">
              <h4>RELATED RESOURCES</h4>
              <span className="btn-group">
                <button className={`btn btn-sm${related === 'list' ? ' btn-active' : ''}`} onClick={() => setRelated('list')}>
                  <List size={14} /> List
                </button>
                <button className={`btn btn-sm${related === 'map' ? ' btn-active' : ''}`} onClick={() => setRelated('map')}>
                  <MapIcon size={14} /> Map
                </button>
              </span>
            </div>
            <RelatedProcesses r={r} view={related} />
          </>
        )}
        {tab === 'metrics' && renderTab('metrics')}
        {tab === 'logs' && renderTab('logs')}
        {tab === 'network' && renderTab('network')}
        {tab === 'traces' && renderTab('apm')}
        {tab === 'profiles' && (
          <Empty>
            Profiles of the processes in this container:{' '}
            <Link to={`/apm/profiles?${new URLSearchParams({ q: r.node ? `host:${r.node}` : `container_id:${r.containerId ?? ''}` })}`}>open them in the Profiler</Link>.
          </Empty>
        )}
      </div>
    </div>
  );
}
