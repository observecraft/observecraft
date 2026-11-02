// Kubernetes → Live Rollouts: each Deployment's latest rollout, its progress and availability.
import { useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import { ArrowDownWideNarrow, PanelLeftClose, PanelLeftOpen, Search } from 'lucide-react';
import { useQuery } from '@tanstack/react-query';
import { listRollouts, type Rollout, type RolloutStatus } from '../../api/kubernetes/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { timeAgo } from '../../lib/format';
import { applyFacets } from './autoscaling/model';
import { Facets, type FacetDef } from './Facets';
import { LiveClock, LiveProvider, OrchestrationHeader, useLive } from './OrchestrationHeader';
import '../infrastructure/infrastructure.css';
import '../settings/settings.css';
import './kubernetes.css';
import './autoscaling/autoscaling.css';

const STATUS: Record<RolloutStatus, { label: string; pill: string }> = {
  in_progress: { label: 'In progress', pill: 'as-pill-info' },
  complete: { label: 'Complete', pill: 'as-pill-on' },
  stalled: { label: 'Stalled', pill: 'as-pill-off' },
  scaled_down: { label: 'Scaled down', pill: 'as-pill-muted' },
};

const FACETS: FacetDef<Rollout>[] = [
  { key: 'status', title: 'Rollout status', value: (r) => r.status, label: (v) => STATUS[v as RolloutStatus]?.label ?? v },
  { key: 'cluster', title: 'Cluster', value: (r) => r.cluster },
  { key: 'namespace', title: 'Namespace', value: (r) => r.namespace },
];
const KEYS = Object.fromEntries(FACETS.map((f) => [f.key, f.value]));

/** `1m 35s`, `2h 4m`. */
export function duration(seconds: number): string {
  const s = Math.round(seconds);
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m ${s % 60}s`;
  return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`;
}

export function LiveRolloutsPage() {
  return (
    <LiveProvider>
      <LiveRollouts />
    </LiveProvider>
  );
}

function LiveRollouts() {
  const { interval } = useLive();
  const rollouts = useQuery({ queryKey: ['kubernetes', 'rollouts'], queryFn: listRollouts, refetchInterval: interval });
  const [query, setQuery] = useState('');
  const [selected, setSelected] = useState<Record<string, string[]>>({});
  const [controls, setControls] = useState(true);
  const all = rollouts.data ?? [];
  const q = query.trim().toLowerCase();
  const shown = useMemo(() => applyFacets(all, KEYS, selected).filter((r) => !q || [r.name, r.cluster, r.namespace, r.image ?? ''].some((v) => v.toLowerCase().includes(q))), [all, selected, q]);
  return (
    <div className="hosts-page ox-page as-page">
      <OrchestrationHeader product="kubernetes" active="Live Rollouts" right={<LiveClock updatedAt={rollouts.dataUpdatedAt} />} />
      <div className="as-body">
        <label className="as-search">
          <Search size={16} />
          <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search Live Rollouts" />
        </label>
        <div className={`as-explorer${controls ? '' : ' no-controls'}`}>
          {controls && <Facets items={all} facets={FACETS} selected={selected} onChange={setSelected} />}
          <div className="as-explorer-main">
            <div className="as-table-head as-plain">
              <button className="link-btn as-hide" onClick={() => setControls((c) => !c)}>
                {controls ? <PanelLeftClose size={15} /> : <PanelLeftOpen size={15} />} {controls ? 'Hide Controls' : 'Show Controls'}
              </button>
              <span>
                Showing <strong>{shown.length ? `1–${shown.length}` : '0–0'}</strong> of <strong>{all.length}</strong> Deployments
              </span>
            </div>
            {rollouts.isPending ? (
              <Loading label="Loading rollouts…" />
            ) : rollouts.isError ? (
              <ErrorState error={rollouts.error} onRetry={() => rollouts.refetch()} />
            ) : (
              <div className="as-table-card">
                <table className="settings-table as-table">
                  <thead>
                    <tr>
                      <th>
                        <ArrowDownWideNarrow size={14} /> Deployment
                      </th>
                      <th>Cluster</th>
                      <th>Namespace</th>
                      <th>Rollout status</th>
                      <th>Availability</th>
                      <th>Progress</th>
                      <th>Started</th>
                      <th>Duration</th>
                    </tr>
                  </thead>
                  <tbody>
                    {shown.map((r) => (
                      <tr key={r.uid}>
                        <td>
                          <div className="as-workload">
                            <Link to={`/orchestration/explorer?resource=deployments&query=${encodeURIComponent(`kube_cluster_name:${r.cluster} kube_namespace:${r.namespace} ${r.name}`)}`}>
                              {r.name}
                            </Link>
                            <span className="muted">
                              {r.revision != null && `revision ${r.revision} · `}
                              {r.image?.split('/').pop()}
                            </span>
                          </div>
                        </td>
                        <td className="as-nowrap">{r.cluster}</td>
                        <td>{r.namespace}</td>
                        <td>
                          <span className={`as-pill ${STATUS[r.status].pill}`} title={r.message ?? undefined}>
                            {STATUS[r.status].label}
                          </span>
                        </td>
                        <td>
                          <Bar value={r.available} of={r.desired} />
                        </td>
                        <td>
                          <Bar value={r.updated} of={r.desired} />
                        </td>
                        <td className="as-nowrap">{r.started ? timeAgo(r.started) : '—'}</td>
                        <td className="as-nowrap">{r.durationSeconds != null ? duration(r.durationSeconds) : '—'}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
                {shown.length === 0 && (
                  <div className="as-empty">
                    <Search size={56} strokeWidth={1.6} />
                    <h3>No Results Found</h3>
                    <p>
                      {all.length ? 'No deployment matches the filters.' : 'Deploy the Datadog Agent with the orchestrator explorer to collect your Kubernetes resources.'}{' '}
                      {!all.length && <Link to="/integrations/kubernetes?tab=install">Set up a cluster</Link>}
                    </p>
                  </div>
                )}
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

function Bar({ value, of }: { value: number; of: number }) {
  const pct = of > 0 ? Math.min(100, (value / of) * 100) : 0;
  return (
    <span className="as-bar">
      <span className="as-bar-track">
        <span style={{ width: `${pct}%` }} />
      </span>
      <span>
        {value}/{of}
      </span>
    </span>
  );
}
