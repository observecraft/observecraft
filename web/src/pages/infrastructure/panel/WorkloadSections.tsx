import { useDeferredValue, useState } from 'react';
import { Link } from 'react-router-dom';
import { Activity, ArrowUpNarrowWide, ArrowDownWideNarrow, Box, EllipsisVertical, Search, Ship } from 'lucide-react';
import type { HostDetail } from '../../../api/infrastructure/types';
import { Empty, ErrorState, Loading } from '../../../components/ui/AsyncState';
import { Select } from '../../../components/ui/Dropdown';
import { Pager, UsageBar, formatAge, formatBytes } from '../../../components/ui/Pager';
import { timeAgo } from '../../../lib/format';
import { SoftwareIcon } from '../hostIcons';
import { useContainers, usePods, useProcesses } from '../queries';
import { HpSection } from './HpSection';

const PAGE = 10;

function FilterBar({ host, value, onChange, placeholder }: { host: string; value: string; onChange: (v: string) => void; placeholder: string }) {
  return (
    <div className="hw-filter">
      <span className="hw-filter-label">Filter by</span>
      <span className="hw-filter-host mono">host:{host}</span>
      <label>
        <Search size={14} />
        <input value={value} onChange={(e) => onChange(e.target.value)} placeholder={placeholder} />
      </label>
    </div>
  );
}

function Showing({ offset, count, total, noun }: { offset: number; count: number; total: number; noun: string }) {
  return (
    <span className="hw-showing">
      Showing <strong>{total ? `${offset + 1}–${offset + count}` : '0–0'}</strong> of <strong>{total}</strong> {noun}
    </span>
  );
}

export function KubernetesSection({ host }: { host: HostDetail }) {
  const [offset, setOffset] = useState(0);
  const q = usePods({ host: host.name, query: '', offset, limit: PAGE });
  return (
    <HpSection id="kubernetes" title="Kubernetes" icon={Ship}>
      {!host.kubernetes ? (
        <Empty>This host is not part of a Kubernetes cluster.</Empty>
      ) : q.isPending ? (
        <Loading label="Loading pods…" />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <>
          <div className="hw-toolbar">
            <span className="btn-group">
              <span className="btn btn-label btn-sm">View Related</span>
              <Select value="pods" options={[{ value: 'pods', label: 'Pods' }]} onChange={() => undefined} triggerClassName="dd-trigger hw-related" />
            </span>
            <Showing offset={offset} count={q.data.items.length} total={q.data.total} noun="Pods" />
          </div>
          <table className={`hw-table${q.isPlaceholderData ? ' is-stale' : ''}`}>
            <thead>
              <tr>
                <th>
                  <ArrowUpNarrowWide size={13} /> POD
                </th>
                <th>STATUS</th>
                <th>CLUSTER</th>
                <th>NAMESPACE</th>
                <th className="num">AGE</th>
                <th className="num">READY</th>
                <th className="num">RESTARTS</th>
                <th className="num">CPU USAGE/LIMITS</th>
                <th className="num">MEM USAGE/LIMITS</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {q.data.items.map((p) => (
                <tr key={p.uid}>
                  <td className="hw-ellipsis">
                    <Link to={`/orchestration/explorer?resource=pods&panel=${encodeURIComponent(p.uid)}`}>{p.name}</Link>
                  </td>
                  <td>
                    <span className={`kx-status kx-${p.health}`}>{p.status}</span>
                  </td>
                  <td>
                    <span className="hw-link-chip">{p.cluster}</span>
                  </td>
                  <td className="hw-ellipsis">
                    <span className="hw-link-chip">{p.namespace}</span>
                  </td>
                  <td className="num">{p.ageSec == null ? '—' : formatAge(p.ageSec)}</td>
                  <td className="num">
                    <span className="ready-chip">{p.ready}</span>
                  </td>
                  <td className="num">{p.restarts}</td>
                  <td className="num">
                    {p.cpuPctOfLimit == null ? (
                      '—'
                    ) : (
                      <>
                        {p.cpuPctOfLimit}% <UsageBar pct={p.cpuPctOfLimit} />
                      </>
                    )}
                  </td>
                  <td className="num">
                    {p.memPctOfLimit == null ? (
                      '—'
                    ) : (
                      <>
                        {p.memPctOfLimit}% <UsageBar pct={p.memPctOfLimit} tone="ok" />
                      </>
                    )}
                  </td>
                  <td>
                    <EllipsisVertical size={15} className="muted" />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <Pager offset={offset} limit={PAGE} total={q.data.total} onChange={setOffset} />
        </>
      )}
    </HpSection>
  );
}

export function ContainersSection({ host }: { host: HostDetail }) {
  const [filter, setFilter] = useState('');
  const [offset, setOffset] = useState(0);
  const query = useDeferredValue(filter);
  const q = useContainers({ host: host.name, query, offset, limit: PAGE });
  const kind = host.software.includes('kubernetes') ? 'kubernetes' : 'docker';
  return (
    <HpSection id="containers" title="Containers" icon={Box}>
      {host.kubernetes && (
        <dl className="hw-cri">
          <div>
            <dt>CRI_NAME</dt>
            <dd>{host.kubernetes.criName}</dd>
          </div>
          <div>
            <dt>CRI_VERSION</dt>
            <dd>{host.kubernetes.criVersion}</dd>
          </div>
          <div>
            <dt>KUBELET_VERSION</dt>
            <dd>{host.kubernetes.kubeletVersion}</dd>
          </div>
        </dl>
      )}
      {host.software.length === 0 ? (
        <Empty>No container runtime detected on this host.</Empty>
      ) : (
        <>
          <FilterBar host={host.name} value={filter} onChange={(v) => (setFilter(v), setOffset(0))} placeholder="Filter your Containers" />
          {q.isPending ? (
            <Loading label="Loading containers…" />
          ) : q.isError ? (
            <ErrorState error={q.error} onRetry={() => q.refetch()} />
          ) : (
            <>
              <div className="hw-toolbar">
                <Showing offset={offset} count={q.data.items.length} total={q.data.total} noun="containers" />
              </div>
              <table className={`hw-table${q.isPlaceholderData ? ' is-stale' : ''}`}>
                <thead>
                  <tr>
                    <th>STATUS</th>
                    <th>CONTAINER</th>
                    <th className="num">TOTAL CPU - NORMALIZED</th>
                    <th className="num">
                      <ArrowDownWideNarrow size={13} /> RSS MEMORY
                    </th>
                    <th>POD</th>
                    <th className="num">STARTED (AGO)</th>
                  </tr>
                </thead>
                <tbody>
                  {q.data.items.map((c) => (
                    <tr key={`${c.pod}/${c.name}`}>
                      <td>
                        <span className={c.status === 'UP' ? 'up-chip' : 'down-chip'}>{c.status}</span>
                      </td>
                      <td className="hw-ellipsis" title={c.image}>
                        <span className="hw-container-name">
                          <SoftwareIcon kind={kind} size={15} /> {c.name}
                        </span>
                      </td>
                      <td className="num">
                        {c.cpuPct == null ? (
                          '—'
                        ) : (
                          <>
                            {c.cpuPct}% <UsageBar pct={c.cpuPct} />
                          </>
                        )}
                      </td>
                      <td className="num">
                        {c.rssBytes == null ? (
                          '—'
                        ) : (
                          <>
                            {formatBytes(c.rssBytes)} {c.memLimitBytes ? <UsageBar pct={(c.rssBytes / c.memLimitBytes) * 100} /> : null}
                          </>
                        )}
                      </td>
                      <td className="hw-ellipsis">{c.pod}</td>
                      <td className="num">{c.startedAt == null ? '—' : timeAgo(c.startedAt)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
              <Pager offset={offset} limit={PAGE} total={q.data.total} onChange={setOffset} />
            </>
          )}
        </>
      )}
    </HpSection>
  );
}

export function ProcessesSection({ host }: { host: HostDetail }) {
  const [filter, setFilter] = useState('');
  const [offset, setOffset] = useState(0);
  const query = useDeferredValue(filter);
  const q = useProcesses({ host: host.name, query, offset, limit: PAGE });
  return (
    <HpSection id="processes" title="Processes" icon={Activity}>
      <FilterBar host={host.name} value={filter} onChange={(v) => (setFilter(v), setOffset(0))} placeholder="Filter your Processes" />
      {q.isPending ? (
        <Loading label="Loading processes…" />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <>
          <div className="hw-toolbar">
            <Showing offset={offset} count={q.data.items.length} total={q.data.total} noun="processes" />
          </div>
          <table className="hw-table">
            <thead>
              <tr>
                <th>PROCESS</th>
                <th>PID</th>
                <th>USER</th>
                <th className="num">TOTAL CPU - NORMALIZED</th>
                <th className="num">
                  <ArrowDownWideNarrow size={13} /> RSS MEMORY
                </th>
              </tr>
            </thead>
            <tbody>
              {q.data.items.map((p) => (
                <tr key={p.pid}>
                  <td className="mono hw-ellipsis" title={p.command}>
                    {p.command}
                  </td>
                  <td className="mono">{p.pid}</td>
                  <td>{p.user}</td>
                  <td className="num">
                    {p.cpuPct}% <UsageBar pct={p.cpuPct} />
                  </td>
                  <td className="num">{formatBytes(p.rssBytes)}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {q.data.total === 0 && (
            <div className="hw-noproc">
              <Activity size={44} className="hw-noproc-icon" />
              <strong>No processes found</strong>
              <span>{q.data.enabled ? 'No processes match this filter.' : 'To correlate processes with hosts, enable the processes scraper in this host’s collector configuration.'}</span>
            </div>
          )}
          <Pager offset={offset} limit={PAGE} total={q.data.total} onChange={setOffset} />
        </>
      )}
    </HpSection>
  );
}
